//! Archivos de la PC: qué es un video, cómo se llama dentro de CasparCG,
//! duración de MP4/MOV/MKV sin herramientas externas y copia a la carpeta de medios.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Extensiones que se tratan como video.
pub const EXTENSIONES_VIDEO: &[&str] =
    &["mp4", "mov", "m4v", "mxf", "avi", "mkv", "mpg", "mpeg", "ts", "m2ts", "wmv", "webm", "flv", "dv"];

pub fn es_video(ruta: &Path) -> bool {
    ruta.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONES_VIDEO.iter().any(|v| v.eq_ignore_ascii_case(e)))
}

/// Subcarpeta de medios donde se copian los videos que estaban fuera.
pub const CARPETA_IMPORTADOS: &str = "IMPORTADOS";

/// Si `archivo` está dentro de `carpeta_medios`, devuelve su nombre de clip para
/// CasparCG: ruta relativa con `/`, sin extensión. Compara sin distinguir
/// mayúsculas, como Windows.
pub fn clip_de(archivo: &Path, carpeta_medios: &Path) -> Option<String> {
    let a = normalizar(archivo);
    let m = normalizar(carpeta_medios);
    let ca: Vec<String> = componentes(&a);
    let cm: Vec<String> = componentes(&m);
    if ca.len() <= cm.len() {
        return None;
    }
    if !ca.iter().zip(&cm).all(|(x, y)| x.eq_ignore_ascii_case(y)) {
        return None;
    }
    let mut partes: Vec<String> = ca[cm.len()..].to_vec();
    let ultimo = partes.pop()?;
    let sin_ext = match ultimo.rfind('.') {
        Some(i) if i > 0 => ultimo[..i].to_string(),
        _ => ultimo,
    };
    partes.push(sin_ext);
    Some(partes.join("/"))
}

fn normalizar(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn componentes(p: &Path) -> Vec<String> {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            Component::Prefix(pre) => Some(
                pre.as_os_str().to_string_lossy().trim_start_matches(r"\\?\").trim_start_matches(r"UNC\").to_string(),
            ),
            _ => None,
        })
        .collect()
}

/// Videos de la carpeta de medios leídos del disco, con los mismos nombres que les da
/// CasparCG (ruta relativa con `/`, sin extensión, en mayúsculas). Se usa cuando el
/// media scanner de CasparCG todavía no responde a CLS.
pub fn listar_medios_disco(carpeta: &Path) -> Vec<amcp::Medio> {
    let mut v = Vec::new();
    let mut pendientes = vec![(carpeta.to_path_buf(), 0usize)];
    while let Some((dir, nivel)) = pendientes.pop() {
        let Ok(entradas) = std::fs::read_dir(&dir) else { continue };
        for e in entradas.flatten() {
            let ruta = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if nivel < 8 {
                    pendientes.push((ruta, nivel + 1));
                }
                continue;
            }
            if !es_video(&ruta) {
                continue;
            }
            let Some(rel) = ruta.strip_prefix(carpeta).ok() else { continue };
            let mut nombre = rel.to_string_lossy().replace('\\', "/");
            if let Some(i) = nombre.rfind('.') {
                if !nombre[i..].contains('/') {
                    nombre.truncate(i);
                }
            }
            let modificado = meta
                .modified()
                .ok()
                .map(time::OffsetDateTime::from)
                .and_then(|f| {
                    f.format(time::macros::format_description!("[year][month][day][hour][minute][second]")).ok()
                })
                .unwrap_or_default();
            let duracion = duracion_mp4(&ruta).map(|d| d.as_secs_f64()).unwrap_or(0.0);
            v.push(amcp::Medio {
                nombre: nombre.to_uppercase(),
                tipo: amcp::TipoMedio::Movie,
                tamano_bytes: meta.len(),
                modificado,
                cuadros: 0,
                fps: 0.0,
                duracion_seg: duracion,
            });
        }
    }
    v
}

/// Duración de un video leyendo su cabecera: MP4/MOV/M4V (`moov/mvhd`) y MKV/WebM
/// (`Segment/Info/Duration`). `None` en otros formatos (ahí la da ffmpeg o CasparCG).
pub fn duracion_video(ruta: &Path) -> Option<Duration> {
    let ext = ruta.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "mkv" | "webm" | "mka" => duracion_mkv(ruta),
        _ => duracion_mp4(ruta),
    }
}

/// Duración de un MKV/WebM (Matroska): `Segment → Info → Duration × TimestampScale`.
pub fn duracion_mkv(ruta: &Path) -> Option<Duration> {
    let mut f = File::open(ruta).ok()?;
    let largo = f.metadata().ok()?.len();
    let (id, tam, ini) = leer_elemento(&mut f, 0)?;
    if id != 0x1A45_DFA3 {
        return None; // no es EBML
    }
    let (id, tam_seg, ini_seg) = leer_elemento(&mut f, ini + tam?)?;
    if id != 0x1853_8067 {
        return None; // no es un Segment
    }
    let fin_seg = tam_seg.map(|t| (ini_seg + t).min(largo)).unwrap_or(largo);
    let mut pos = ini_seg;
    // La Info suele estar al principio; se recorren pocos elementos.
    for _ in 0..256 {
        if pos >= fin_seg {
            break;
        }
        let (id, tam, ini) = leer_elemento(&mut f, pos)?;
        match id {
            0x1549_A966 => return leer_info_mkv(&mut f, ini, ini + tam?),
            0x1F43_B675 => return None, // ya empiezan los cuadros (Cluster) sin Info
            _ => pos = ini + tam?,
        }
    }
    None
}

fn leer_info_mkv(f: &mut File, desde: u64, hasta: u64) -> Option<Duration> {
    let mut escala: u64 = 1_000_000; // nanosegundos por unidad (por defecto, 1 ms)
    let mut duracion: Option<f64> = None;
    let mut pos = desde;
    while pos < hasta {
        let (id, tam, ini) = leer_elemento(f, pos)?;
        let tam = tam?;
        if tam <= 8 {
            let mut b = vec![0u8; tam as usize];
            f.seek(SeekFrom::Start(ini)).ok()?;
            f.read_exact(&mut b).ok()?;
            match id {
                0x2A_D7B1 => escala = b.iter().fold(0u64, |a, x| (a << 8) | *x as u64),
                0x4489 if tam == 4 => duracion = Some(f32::from_be_bytes(b[..4].try_into().ok()?) as f64),
                0x4489 if tam == 8 => duracion = Some(f64::from_be_bytes(b[..8].try_into().ok()?)),
                _ => {}
            }
        }
        pos = ini + tam;
    }
    let seg = duracion? * escala as f64 / 1e9;
    (seg.is_finite() && seg > 0.0).then(|| Duration::from_secs_f64(seg))
}

/// Un elemento EBML en `pos`: su id, su tamaño (`None` = desconocido) y dónde empiezan
/// sus datos.
fn leer_elemento(f: &mut File, pos: u64) -> Option<(u64, Option<u64>, u64)> {
    f.seek(SeekFrom::Start(pos)).ok()?;
    let (id, n_id) = leer_vint(f, true)?;
    let (tam, n_tam) = leer_vint(f, false)?;
    let todos_unos = (1u64 << (7 * n_tam)) - 1;
    let tam = (tam != todos_unos).then_some(tam);
    Some((id, tam, pos + n_id as u64 + n_tam as u64))
}

/// Número de largo variable de EBML. Los ids se guardan con su marca; los tamaños, sin ella.
fn leer_vint(f: &mut File, con_marca: bool) -> Option<(u64, u32)> {
    let mut b = [0u8; 1];
    f.read_exact(&mut b).ok()?;
    if b[0] == 0 {
        return None;
    }
    let n = b[0].leading_zeros() + 1;
    if n > 8 {
        return None;
    }
    let mut v = if con_marca { b[0] as u64 } else { (b[0] as u64) & (0xFFu64 >> n) };
    for _ in 1..n {
        f.read_exact(&mut b).ok()?;
        v = (v << 8) | b[0] as u64;
    }
    Some((v, n))
}

/// Duración de un MP4/MOV/M4V leyendo su cabecera (`moov/mvhd`). `None` en otros formatos.
pub fn duracion_mp4(ruta: &Path) -> Option<Duration> {
    let mut f = File::open(ruta).ok()?;
    let largo = f.metadata().ok()?.len();
    let (ini, fin) = buscar_caja(&mut f, 0, largo, b"moov")?;
    let (ini_mvhd, _) = buscar_caja(&mut f, ini, fin, b"mvhd")?;
    f.seek(SeekFrom::Start(ini_mvhd)).ok()?;
    let mut version = [0u8; 4];
    f.read_exact(&mut version).ok()?;
    let (escala, dur) = if version[0] == 1 {
        let mut b = [0u8; 28];
        f.read_exact(&mut b).ok()?;
        (u32::from_be_bytes(b[16..20].try_into().ok()?), u64::from_be_bytes(b[20..28].try_into().ok()?))
    } else {
        let mut b = [0u8; 16];
        f.read_exact(&mut b).ok()?;
        (u32::from_be_bytes(b[8..12].try_into().ok()?), u32::from_be_bytes(b[12..16].try_into().ok()?) as u64)
    };
    if escala == 0 || dur == 0 || dur == u32::MAX as u64 || dur == u64::MAX {
        return None;
    }
    Some(Duration::from_secs_f64(dur as f64 / escala as f64))
}

/// Busca una caja ISO-BMFF dentro de [desde, hasta). Devuelve el inicio y fin de su contenido.
fn buscar_caja(f: &mut File, desde: u64, hasta: u64, tipo: &[u8; 4]) -> Option<(u64, u64)> {
    let mut pos = desde;
    while pos + 8 <= hasta {
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut cab = [0u8; 8];
        f.read_exact(&mut cab).ok()?;
        let mut tam = u32::from_be_bytes(cab[0..4].try_into().ok()?) as u64;
        let mut cabecera = 8;
        if tam == 1 {
            let mut g = [0u8; 8];
            f.read_exact(&mut g).ok()?;
            tam = u64::from_be_bytes(g);
            cabecera = 16;
        } else if tam == 0 {
            tam = hasta - pos;
        }
        if tam < cabecera {
            return None;
        }
        if &cab[4..8] == tipo {
            return Some((pos + cabecera, (pos + tam).min(hasta)));
        }
        pos += tam;
    }
    None
}

/// Destino para copiar `origen` dentro de la carpeta de medios. Si ya existe un
/// archivo con el mismo nombre y tamaño, se reutiliza (no hace falta copiar).
pub fn destino_importacion(origen: &Path, carpeta_medios: &Path) -> io::Result<(PathBuf, bool)> {
    let tam = std::fs::metadata(origen)?.len();
    let carpeta = carpeta_medios.join(CARPETA_IMPORTADOS);
    let nombre = origen.file_name().ok_or_else(|| io::Error::other("ruta sin nombre de archivo"))?;
    let base = Path::new(nombre).file_stem().unwrap_or(nombre).to_string_lossy().into_owned();
    let ext = Path::new(nombre).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1.. {
        let candidato = if n == 1 { carpeta.join(nombre) } else { carpeta.join(format!("{base} ({n}){ext}")) };
        match std::fs::metadata(&candidato) {
            Ok(m) if m.len() == tam => return Ok((candidato, true)),
            Ok(_) => continue,
            Err(_) => return Ok((candidato, false)),
        }
    }
    unreachable!()
}

/// Copia por bloques avisando el avance (0.0 a 1.0). Escribe primero a un archivo
/// temporal y lo renombra al final, para que CasparCG nunca vea un video a medias.
pub fn copiar_con_progreso(origen: &Path, destino: &Path, mut avance: impl FnMut(f32)) -> io::Result<()> {
    if let Some(p) = destino.parent() {
        std::fs::create_dir_all(p)?;
    }
    let total = std::fs::metadata(origen)?.len().max(1);
    let temporal = destino.with_extension("copiando");
    let resultado = (|| {
        let mut r = File::open(origen)?;
        let mut w = File::create(&temporal)?;
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        let mut hecho = 0u64;
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n])?;
            hecho += n as u64;
            avance(hecho as f32 / total as f32);
        }
        w.sync_all()?;
        Ok(())
    })();
    match resultado {
        Ok(()) => std::fs::rename(&temporal, destino),
        Err(e) => {
            let _ = std::fs::remove_file(&temporal);
            Err(e)
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn duracion_de_un_mkv() {
        // EBML mínimo: cabecera, Segment de tamaño desconocido, SeekHead vacío e Info
        // con TimestampScale = 1 ms y Duration = 12 345 (float de 8 bytes) → 12,345 s.
        let mut b: Vec<u8> = vec![0x1A, 0x45, 0xDF, 0xA3, 0x80];
        b.extend([0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        b.extend([0x11, 0x4D, 0x9B, 0x74, 0x80]);
        let mut info = vec![0x2A, 0xD7, 0xB1, 0x83, 0x0F, 0x42, 0x40];
        info.extend([0x44, 0x89, 0x88]);
        info.extend(12345.0f64.to_be_bytes());
        b.extend([0x15, 0x49, 0xA9, 0x66, 0x80 | info.len() as u8]);
        b.extend(info);
        b.extend([0x1F, 0x43, 0xB6, 0x75, 0x80]);
        let ruta = std::env::temp_dir().join(format!("relevo-prueba-{}.mkv", std::process::id()));
        std::fs::write(&ruta, &b).unwrap();
        let d = duracion_video(&ruta).unwrap();
        assert!((d.as_secs_f64() - 12.345).abs() < 1e-6, "{d:?}");
        // Un MKV roto no da duración (y no se cuelga).
        std::fs::write(&ruta, [0x1A, 0x45, 0xDF, 0xA3, 0x80, 0x00]).unwrap();
        assert!(duracion_video(&ruta).is_none());
        let _ = std::fs::remove_file(&ruta);
    }

    fn tmp(nombre: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("relevo-pruebas-{}-{nombre}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reconoce_videos() {
        assert!(es_video(Path::new("a/spot.MP4")));
        assert!(es_video(Path::new("spot.mov")));
        assert!(!es_video(Path::new("notas.txt")));
        assert!(!es_video(Path::new("sin_extension")));
    }

    #[test]
    fn nombre_de_clip() {
        let m = Path::new("/caspar/media");
        assert_eq!(
            clip_de(Path::new("/caspar/media/CLIENTES/Ferreteria X/spot 30s.mp4"), m).unwrap(),
            "CLIENTES/Ferreteria X/spot 30s"
        );
        assert_eq!(clip_de(Path::new("/caspar/MEDIA/id.mov"), m).unwrap(), "id");
        assert!(clip_de(Path::new("/otra/carpeta/spot.mp4"), m).is_none());
        assert!(clip_de(Path::new("/caspar/media"), m).is_none());
    }

    /// MP4 mínimo: ftyp + moov/mvhd (versión 0) con escala 1000 y duración 30 000.
    fn mp4_de_prueba(ruta: &Path, version1: bool) {
        let mut mvhd = Vec::new();
        if version1 {
            mvhd.extend([1, 0, 0, 0]);
            mvhd.extend([0u8; 16]);
            mvhd.extend(1000u32.to_be_bytes());
            mvhd.extend(30_000u64.to_be_bytes());
        } else {
            mvhd.extend([0, 0, 0, 0]);
            mvhd.extend([0u8; 8]);
            mvhd.extend(1000u32.to_be_bytes());
            mvhd.extend(30_000u32.to_be_bytes());
        }
        mvhd.extend([0u8; 80]);
        let caja = |tipo: &[u8; 4], cont: &[u8]| {
            let mut v = ((cont.len() + 8) as u32).to_be_bytes().to_vec();
            v.extend(tipo);
            v.extend(cont);
            v
        };
        let mut f = caja(b"ftyp", b"isom\0\0\0\0isom");
        f.extend(caja(b"free", &[0u8; 10]));
        f.extend(caja(b"moov", &caja(b"mvhd", &mvhd)));
        std::fs::write(ruta, f).unwrap();
    }

    #[test]
    fn duracion_de_mp4() {
        let d = tmp("mp4");
        let a = d.join("a.mp4");
        mp4_de_prueba(&a, false);
        assert_eq!(duracion_mp4(&a), Some(Duration::from_secs(30)));
        let b = d.join("b.mov");
        mp4_de_prueba(&b, true);
        assert_eq!(duracion_mp4(&b), Some(Duration::from_secs(30)));
        let c = d.join("c.mp4");
        std::fs::write(&c, b"esto no es un mp4").unwrap();
        assert_eq!(duracion_mp4(&c), None);
    }

    #[test]
    fn importar_copia_y_reutiliza() {
        let d = tmp("imp");
        let origen = d.join("fuera").join("spot.mp4");
        std::fs::create_dir_all(origen.parent().unwrap()).unwrap();
        std::fs::write(&origen, vec![7u8; 10_000_000]).unwrap();
        let medios = d.join("media");

        let (dest, existe) = destino_importacion(&origen, &medios).unwrap();
        assert!(!existe);
        assert_eq!(dest, medios.join("IMPORTADOS").join("spot.mp4"));
        let mut ultimo = 0.0;
        copiar_con_progreso(&origen, &dest, |p| ultimo = p).unwrap();
        assert_eq!(ultimo, 1.0);
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), 10_000_000);
        assert!(!dest.with_extension("copiando").exists());

        // Mismo archivo otra vez: se reutiliza.
        assert_eq!(destino_importacion(&origen, &medios).unwrap(), (dest.clone(), true));
        // Otro archivo con el mismo nombre: se numera.
        std::fs::write(&origen, b"distinto").unwrap();
        let (dest2, existe2) = destino_importacion(&origen, &medios).unwrap();
        assert!(!existe2);
        assert_eq!(dest2.file_name().unwrap(), "spot (2).mp4");
    }

    #[test]
    fn lista_los_videos_del_disco_como_casparcg() {
        let d = std::env::temp_dir().join(format!("relevo-disco-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Promos")).unwrap();
        std::fs::write(d.join("Promos").join("Volvemos.mp4"), b"x").unwrap();
        std::fs::write(d.join("spot.v2.mov"), b"x").unwrap();
        std::fs::write(d.join("notas.txt"), b"x").unwrap();
        let mut v: Vec<String> = listar_medios_disco(&d).into_iter().map(|m| m.nombre).collect();
        v.sort();
        assert_eq!(v, vec!["PROMOS/VOLVEMOS".to_string(), "SPOT.V2".to_string()]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
