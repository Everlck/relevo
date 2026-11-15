//! Respaldo automático: una vez al día se guarda en un .zip todo lo que no se puede
//! volver a armar (pautas, usuarios, registro de emisión, revisiones de la biblioteca,
//! configuración…). No incluye videos.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

/// Lo que no se respalda: subidas a medias, temporales, videos de la papelera y el log.
fn incluir(rel: &str) -> bool {
    let r = rel.replace('\\', "/");
    !(r.starts_with("subidas/")
        || r.starts_with("temporal/")
        || (r.starts_with("papelera/") && r != "papelera/papelera.json")
        || r.ends_with(".tmp")
        || r.ends_with(".log"))
}

fn archivos(carpeta: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let mut pendientes = vec![carpeta.to_path_buf()];
    while let Some(d) = pendientes.pop() {
        let Ok(l) = std::fs::read_dir(&d) else { continue };
        for e in l.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => pendientes.push(p),
                Ok(t) if t.is_file() => v.push(p),
                _ => {}
            }
        }
    }
    v.sort();
    v
}

/// Crea `destino/nombre` (.zip) con la carpeta de datos y los archivos `extra`
/// (por ejemplo `config.toml`). Devuelve la ruta del respaldo.
pub fn respaldar(carpeta_datos: &Path, extra: &[PathBuf], destino: &Path, nombre: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(destino).map_err(|e| format!("no se pudo crear {}: {e}", destino.display()))?;
    let final_ = destino.join(nombre);
    let tmp = destino.join(format!("{nombre}.tmp"));
    let f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(f);
    let op = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut buf = Vec::new();
    let mut agregar = |zip: &mut zip::ZipWriter<std::fs::File>, archivo: &Path, nombre: &str| -> Result<(), String> {
        buf.clear();
        let Ok(mut f) = std::fs::File::open(archivo) else { return Ok(()) };
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        zip.start_file(nombre, op).map_err(|e| e.to_string())?;
        zip.write_all(&buf).map_err(|e| e.to_string())
    };
    for a in archivos(carpeta_datos) {
        let rel = a.strip_prefix(carpeta_datos).unwrap_or(&a).to_string_lossy().replace('\\', "/");
        if incluir(&rel) {
            agregar(&mut zip, &a, &format!("datos/{rel}"))?;
        }
    }
    for a in extra {
        if let Some(n) = a.file_name() {
            agregar(&mut zip, a, &n.to_string_lossy())?;
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &final_).map_err(|e| e.to_string())?;
    Ok(final_)
}

/// Respaldos que hay en la carpeta, del más nuevo al más viejo.
pub fn lista(destino: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(destino)
        .map(|l| {
            l.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    n.starts_with("respaldo-") && n.ends_with(".zip")
                })
                .collect()
        })
        .unwrap_or_default();
    // El nombre lleva la fecha: ordenar por nombre es ordenar por fecha.
    v.sort();
    v.reverse();
    v
}

/// Borra los respaldos más viejos, dejando `guardar`.
pub fn podar(destino: &Path, guardar: usize) -> usize {
    let v = lista(destino);
    let mut n = 0;
    for p in v.iter().skip(guardar.max(1)) {
        if std::fs::remove_file(p).is_ok() {
            n += 1;
        }
    }
    n
}

/// ¿Ya hay un respaldo de este día (`AAAA-MM-DD`)?
pub fn hay_de(destino: &Path, dia: &str) -> bool {
    lista(destino)
        .iter()
        .any(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&format!("respaldo-{dia}"))))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn respalda_lo_importante_y_poda() {
        let base = std::env::temp_dir().join(format!("relevo-respaldo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let datos = base.join("datos");
        std::fs::create_dir_all(datos.join("registro")).unwrap();
        std::fs::create_dir_all(datos.join("subidas")).unwrap();
        std::fs::create_dir_all(datos.join("papelera")).unwrap();
        std::fs::write(datos.join("pautas.json"), "{}").unwrap();
        std::fs::write(datos.join("registro").join("2026-11-11.jsonl"), "x").unwrap();
        std::fs::write(datos.join("subidas").join("a.parte"), "no").unwrap();
        std::fs::write(datos.join("papelera").join("00001 spot.mp4"), "no").unwrap();
        std::fs::write(datos.join("papelera").join("papelera.json"), "[]").unwrap();
        std::fs::write(base.join("config.toml"), "[casparcg]").unwrap();

        let dest = base.join("respaldos");
        let r = respaldar(&datos, &[base.join("config.toml")], &dest, "respaldo-2026-11-11.zip").unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&r).unwrap()).unwrap();
        let mut nombres: Vec<String> =
            (0..z.len()).map(|i| z.by_index(i).unwrap().name().unwrap().to_string()).collect();
        nombres.sort();
        assert_eq!(
            nombres,
            ["config.toml", "datos/papelera/papelera.json", "datos/pautas.json", "datos/registro/2026-11-11.jsonl"]
        );
        assert!(hay_de(&dest, "2026-11-11"));
        assert!(!hay_de(&dest, "2026-11-12"));

        for d in ["2026-11-08", "2026-11-09", "2026-11-10"] {
            respaldar(&datos, &[], &dest, &format!("respaldo-{d}.zip")).unwrap();
        }
        assert_eq!(podar(&dest, 2), 2);
        let quedan: Vec<String> =
            lista(&dest).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(quedan, ["respaldo-2026-11-11.zip", "respaldo-2026-11-10.zip"]);
        let _ = std::fs::remove_dir_all(&base);
    }
}
