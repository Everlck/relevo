/// Carpetas que informa CasparCG con `INFO PATHS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rutas {
    /// Carpeta desde donde se inició CasparCG (puede faltar).
    pub inicial: Option<String>,
    /// Carpeta de medios, tal como la escribe CasparCG (a veces relativa).
    pub medios: String,
}

impl Rutas {
    pub fn desde_xml(xml: &str) -> Option<Rutas> {
        let medios = etiqueta(xml, "media-path")?;
        let inicial = etiqueta(xml, "initial-path");
        Some(Rutas { inicial, medios })
    }

    /// Ruta absoluta de la carpeta de medios, sin barra final.
    pub fn carpeta_medios(&self) -> String {
        let medios = self.medios.trim();
        let combinada = match &self.inicial {
            Some(ini) if !es_absoluta(medios) => {
                let sep = if ini.contains('\\') { '\\' } else { '/' };
                let ini = ini.trim_end_matches(['/', '\\']);
                let rel = medios.trim_start_matches("./").trim_start_matches(".\\");
                format!("{ini}{sep}{rel}")
            }
            _ => medios.to_string(),
        };
        let mut r = combinada.trim_end_matches(['/', '\\']).to_string();
        if r.ends_with(':') {
            r.push('\\');
        }
        r
    }
}

fn es_absoluta(ruta: &str) -> bool {
    let b = ruta.as_bytes();
    ruta.starts_with('/') || ruta.starts_with("\\\\") || (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic())
}

fn etiqueta(xml: &str, nombre: &str) -> Option<String> {
    let abre = format!("<{nombre}>");
    let cierra = format!("</{nombre}>");
    let ini = xml.find(&abre)? + abre.len();
    let fin = xml[ini..].find(&cierra)? + ini;
    let valor = xml[ini..fin].trim();
    (!valor.is_empty()).then(|| valor.to_string())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<paths>
   <media-path>media/</media-path>
   <log-path>log/</log-path>
   <initial-path>C:\CasparCG Server\</initial-path>
</paths>"#;

    #[test]
    fn relativa_a_la_carpeta_inicial() {
        let r = Rutas::desde_xml(XML).unwrap();
        assert_eq!(r.carpeta_medios(), r"C:\CasparCG Server\media");
    }

    #[test]
    fn absoluta_windows() {
        let r =
            Rutas::desde_xml("<paths><media-path>D:\\Spots\\</media-path><initial-path>C:\\x\\</initial-path></paths>")
                .unwrap();
        assert_eq!(r.carpeta_medios(), r"D:\Spots");
    }

    #[test]
    fn absoluta_linux_y_sin_inicial() {
        let r = Rutas::desde_xml("<paths><media-path>/srv/media/</media-path></paths>").unwrap();
        assert_eq!(r.carpeta_medios(), "/srv/media");
    }

    #[test]
    fn sin_media_path() {
        assert!(Rutas::desde_xml("<paths></paths>").is_none());
    }
}
