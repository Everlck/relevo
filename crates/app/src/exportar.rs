//! Exportar reportes: Excel (resumen y detalle) y PDF (constancia de emisión).

use std::path::Path;

use genpdf::elements::{Break, FrameCellDecorator, Paragraph, TableLayout};
use genpdf::style::{Color, Style};
use genpdf::{Alignment, Element};
use nucleo::reportes::Reporte;
use rust_xlsxwriter::{Color as XColor, Format, FormatAlign, FormatBorder, Workbook};

use crate::tema::mmss;

/// `AAAA-MM-DD` → `DD/MM/AAAA`.
pub fn dma(f: &str) -> String {
    let p: Vec<&str> = f.split('-').collect();
    if p.len() == 3 {
        format!("{}/{}/{}", p[2], p[1], p[0])
    } else {
        f.to_string()
    }
}

pub fn periodo_texto(r: &Reporte) -> String {
    if r.desde == r.hasta {
        dma(&r.desde)
    } else {
        format!("{} al {}", dma(&r.desde), dma(&r.hasta))
    }
}

fn estado(completo: bool, emitido: f64) -> String {
    if completo {
        "Completo".into()
    } else {
        format!("Cortado a {}", mmss(emitido))
    }
}

/// Abre un archivo con el programa de Windows que le corresponde (Excel, el lector de PDF…).
pub fn abrir(ruta: &Path) {
    let r = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]).arg(ruta);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        c.spawn()
    } else {
        std::process::Command::new("xdg-open").arg(ruta).spawn()
    };
    if let Err(e) = r {
        tracing::warn!("no se pudo abrir {}: {e}", ruta.display());
    }
}

// ======================================================================
// Excel
// ======================================================================

pub fn excel(r: &Reporte, ruta: &Path) -> Result<(), String> {
    let e = |x: rust_xlsxwriter::XlsxError| x.to_string();
    let mut libro = Workbook::new();
    let titulo = Format::new().set_bold().set_font_size(14);
    let cab = Format::new()
        .set_bold()
        .set_font_color(XColor::White)
        .set_background_color(XColor::RGB(0x1F3A5F))
        .set_border(FormatBorder::Thin)
        .set_align(FormatAlign::Center);
    let pct = Format::new().set_num_format("0.0\"%\"");
    let rojo = Format::new().set_font_color(XColor::RGB(0xC0392B));

    // Resumen por cliente.
    let h = libro.add_worksheet();
    h.set_name("Resumen").map_err(e)?;
    h.write_with_format(0, 0, "Reporte de emisión", &titulo).map_err(e)?;
    h.write(1, 0, format!("Periodo: {}", periodo_texto(r))).map_err(e)?;
    let cols = [
        "Cliente",
        "Salidas",
        "Completas",
        "Cortadas",
        "Tiempo al aire",
        "Pases programados",
        "Cumplidos",
        "Sin cumplir",
        "Cumplimiento",
    ];
    for (i, c) in cols.iter().enumerate() {
        h.write_with_format(3, i as u16, *c, &cab).map_err(e)?;
    }
    for (i, c) in r.clientes.iter().enumerate() {
        let f = 4 + i as u32;
        h.write(f, 0, &c.cliente).map_err(e)?;
        h.write(f, 1, c.salidas).map_err(e)?;
        h.write(f, 2, c.completas).map_err(e)?;
        h.write(f, 3, c.cortadas).map_err(e)?;
        h.write(f, 4, mmss(c.segundos)).map_err(e)?;
        h.write(f, 5, c.programados).map_err(e)?;
        h.write(f, 6, c.cumplidos).map_err(e)?;
        if c.perdidos > 0 {
            h.write_with_format(f, 7, c.perdidos, &rojo).map_err(e)?;
        } else {
            h.write(f, 7, c.perdidos).map_err(e)?;
        }
        match c.cumplimiento() {
            Some(p) => h.write_with_format(f, 8, p, &pct).map_err(e)?,
            None => h.write(f, 8, "sin pauta").map_err(e)?,
        };
    }
    h.set_freeze_panes(4, 0).map_err(e)?;
    h.autofit();

    // Detalle de cada salida.
    let d = libro.add_worksheet();
    d.set_name("Detalle").map_err(e)?;
    let cols =
        ["Fecha", "Hora", "Cliente", "Spot", "Duración", "Al aire", "Estado", "Modo", "Quién", "Pase de la pauta"];
    for (i, c) in cols.iter().enumerate() {
        d.write_with_format(0, i as u16, *c, &cab).map_err(e)?;
    }
    for (i, s) in r.salidas.iter().enumerate() {
        let f = 1 + i as u32;
        let x = &s.salida;
        d.write(f, 0, dma(&s.fecha)).map_err(e)?;
        d.write(f, 1, &x.hora).map_err(e)?;
        d.write(f, 2, &x.cliente).map_err(e)?;
        d.write(f, 3, &x.clip).map_err(e)?;
        d.write(f, 4, mmss(x.duracion)).map_err(e)?;
        d.write(f, 5, mmss(x.emitido)).map_err(e)?;
        if x.completo {
            d.write(f, 6, estado(true, x.emitido)).map_err(e)?;
        } else {
            d.write_with_format(f, 6, estado(false, x.emitido), &rojo).map_err(e)?;
        }
        d.write(f, 7, &x.modo).map_err(e)?;
        d.write(f, 8, &x.quien).map_err(e)?;
        let pase = x.pase.as_deref().and_then(|p| p.rsplit('@').next()).unwrap_or("");
        d.write(f, 9, pase).map_err(e)?;
    }
    d.set_freeze_panes(1, 0).map_err(e)?;
    d.autofit();

    libro.save(ruta).map_err(e)
}

// ======================================================================
// PDF
// ======================================================================

/// Fuente para el PDF: Arial de Windows si está; si no, la de la app.
fn fuentes() -> Result<genpdf::fonts::FontFamily<genpdf::fonts::FontData>, String> {
    let leer = |ruta: &str| std::fs::read(ruta).ok();
    let (normal, negrita) = if cfg!(windows) {
        let win = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        (leer(&format!("{win}\\Fonts\\arial.ttf")), leer(&format!("{win}\\Fonts\\arialbd.ttf")))
    } else {
        (
            leer("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
            leer("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
        )
    };
    let normal = normal.unwrap_or_else(|| epaint_default_fonts::UBUNTU_LIGHT.to_vec());
    let negrita = negrita.unwrap_or_else(|| normal.clone());
    let f = |d: Vec<u8>| genpdf::fonts::FontData::new(d, None).map_err(|e| e.to_string());
    Ok(genpdf::fonts::FontFamily {
        regular: f(normal.clone())?,
        bold: f(negrita.clone())?,
        italic: f(normal)?,
        bold_italic: f(negrita)?,
    })
}

fn celda(t: impl Into<String>) -> impl Element {
    Paragraph::new(t.into()).padded(1)
}

fn celda_cab(t: &str) -> impl Element {
    Paragraph::new(t).styled(Style::new().bold()).padded(1)
}

/// Constancia de emisión (de un cliente o de todos).
pub fn pdf(r: &Reporte, ruta: &Path, cliente: Option<&str>, generado: &str) -> Result<(), String> {
    let e = |x: genpdf::error::Error| x.to_string();
    let mut doc = genpdf::Document::new(fuentes()?);
    doc.set_title("Constancia de emisión");
    doc.set_font_size(10);
    let mut deco = genpdf::SimplePageDecorator::new();
    deco.set_margins(14);
    doc.set_page_decorator(deco);

    let gris = Style::new().with_color(Color::Rgb(90, 90, 90));
    doc.push(Paragraph::new("Constancia de emisión").styled(Style::new().bold().with_font_size(20)));
    if let Some(c) = cliente {
        doc.push(Paragraph::new(format!("Cliente: {c}")).styled(Style::new().bold().with_font_size(13)));
    }
    doc.push(Paragraph::new(format!("Periodo: {}", periodo_texto(r))).styled(Style::new().with_font_size(11)));
    doc.push(Break::new(1));

    // Resumen.
    let mut t = TableLayout::new(vec![4, 2, 2, 2, 2, 2]);
    t.set_cell_decorator(FrameCellDecorator::new(true, true, false));
    t.row()
        .element(celda_cab("Cliente"))
        .element(celda_cab("Salidas"))
        .element(celda_cab("Completas"))
        .element(celda_cab("Al aire"))
        .element(celda_cab("Pases"))
        .element(celda_cab("Cumplido"))
        .push()
        .map_err(e)?;
    for c in &r.clientes {
        let cumpl = match c.cumplimiento() {
            Some(p) => format!("{p:.0} % ({}/{})", c.cumplidos, c.programados),
            None => "sin pauta".into(),
        };
        t.row()
            .element(celda(c.cliente.clone()))
            .element(celda(c.salidas.to_string()))
            .element(celda(c.completas.to_string()))
            .element(celda(mmss(c.segundos)))
            .element(celda(c.programados.to_string()))
            .element(celda(cumpl))
            .push()
            .map_err(e)?;
    }
    doc.push(t);
    doc.push(Break::new(1.5));

    // Detalle.
    doc.push(Paragraph::new("Detalle de salidas al aire").styled(Style::new().bold().with_font_size(12)));
    doc.push(Break::new(0.5));
    let mut t = TableLayout::new(if cliente.is_some() { vec![2, 2, 5, 2, 3] } else { vec![2, 2, 3, 4, 2, 3] });
    t.set_cell_decorator(FrameCellDecorator::new(true, true, false));
    let mut cab = t.row().element(celda_cab("Fecha")).element(celda_cab("Hora"));
    if cliente.is_none() {
        cab = cab.element(celda_cab("Cliente"));
    }
    cab.element(celda_cab("Spot")).element(celda_cab("Duración")).element(celda_cab("Estado")).push().map_err(e)?;
    for s in &r.salidas {
        let x = &s.salida;
        let mut fila = t.row().element(celda(dma(&s.fecha))).element(celda(x.hora.clone()));
        if cliente.is_none() {
            fila = fila.element(celda(x.cliente.clone()));
        }
        let spot = x.clip.rsplit(['/', '\\']).next().unwrap_or(&x.clip).to_string();
        fila.element(celda(spot))
            .element(celda(mmss(x.duracion)))
            .element(celda(estado(x.completo, x.emitido)))
            .push()
            .map_err(e)?;
    }
    if r.salidas.is_empty() {
        doc.push(t);
        doc.push(Paragraph::new("No hubo salidas al aire en este periodo.").styled(gris));
    } else {
        doc.push(t);
    }
    doc.push(Break::new(1.5));
    let mut pie =
        Paragraph::new(format!("Generado por Relevo el {generado}. Horas según el reloj de la PC de emisión."));
    pie.set_alignment(Alignment::Left);
    doc.push(pie.styled(gris.with_font_size(8)));

    doc.render_to_file(ruta).map_err(e)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use nucleo::reportes::{ResumenCliente, Salida, SalidaFecha};

    fn reporte() -> Reporte {
        Reporte {
            desde: "2026-11-01".into(),
            hasta: "2026-11-11".into(),
            salidas: vec![SalidaFecha {
                fecha: "2026-11-11".into(),
                salida: Salida {
                    hora: "10:05:00".into(),
                    clip: "CLIENTES/FERRETERIA X/SPOT 30S".into(),
                    cliente: "Ferretería X".into(),
                    pase: Some("1@2026-11-11@10:00".into()),
                    duracion: 30.0,
                    emitido: 30.0,
                    completo: true,
                    modo: "AUTO".into(),
                    quien: "PC principal".into(),
                },
            }],
            clientes: vec![ResumenCliente {
                cliente: "Ferretería X".into(),
                salidas: 1,
                completas: 1,
                segundos: 30.0,
                programados: 2,
                cumplidos: 1,
                perdidos: 1,
                ..Default::default()
            }],
            alertas: Vec::new(),
        }
    }

    #[test]
    fn exporta_excel_y_pdf() {
        let base = std::env::temp_dir().join(format!("relevo-exportar-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let r = reporte();
        excel(&r, &base.join("r.xlsx")).unwrap();
        pdf(&r, &base.join("r.pdf"), Some("Ferretería X"), "11/11/2026 12:00").unwrap();
        pdf(&r, &base.join("todos.pdf"), None, "11/11/2026 12:00").unwrap();
        let x = std::fs::read(base.join("r.xlsx")).unwrap();
        assert_eq!(&x[..2], b"PK", "un .xlsx es un zip");
        let p = std::fs::read(base.join("r.pdf")).unwrap();
        assert_eq!(&p[..4], b"%PDF");
        if std::env::var("RELEVO_GUARDAR_PDF").is_ok() {
            let _ = std::fs::copy(base.join("r.pdf"), "/tmp/constancia.pdf");
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
