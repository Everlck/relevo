//! Atajos de teclado configurables ("Space", "Ctrl+Right", "F5"…).

use eframe::egui::{self, Key, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Atajo {
    pub mods: Modifiers,
    pub tecla: Key,
}

/// Lee "Ctrl+Shift+F5". Vacío o desconocido = sin atajo.
pub fn leer(texto: &str) -> Option<Atajo> {
    let mut mods = Modifiers::NONE;
    let mut tecla = None;
    for parte in texto.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match parte.to_lowercase().as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "shift" | "mayus" | "mayús" => mods.shift = true,
            "alt" => mods.alt = true,
            "espacio" => tecla = Some(Key::Space),
            "supr" => tecla = Some(Key::Delete),
            _ => tecla = Key::from_name(parte),
        }
    }
    // egui trata Ctrl como "command" en Windows y Linux.
    mods.command = mods.ctrl;
    tecla.map(|tecla| Atajo { mods, tecla })
}

pub fn escribir(mods: Modifiers, tecla: Key) -> String {
    let mut s = String::new();
    if mods.ctrl || mods.command {
        s.push_str("Ctrl+");
    }
    if mods.shift {
        s.push_str("Shift+");
    }
    if mods.alt {
        s.push_str("Alt+");
    }
    s.push_str(tecla.name());
    s
}

/// Cómo se le muestra al operador.
pub fn bonito(texto: &str) -> String {
    if texto.trim().is_empty() {
        return "—".into();
    }
    texto
        .split('+')
        .map(|p| match p.trim() {
            "Space" => "Espacio",
            "Delete" => "Supr",
            "Enter" => "Enter",
            "Escape" => "Esc",
            "ArrowRight" | "Right" => "→",
            "ArrowLeft" | "Left" => "←",
            "ArrowUp" | "Up" => "↑",
            "ArrowDown" | "Down" => "↓",
            "Backspace" => "Retroceso",
            "Shift" => "Mayús",
            otro => otro,
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// ¿Se pulsó este atajo? Lo "consume" para que no active además un botón con foco.
pub fn pulsado(i: &mut egui::InputState, texto: &str) -> bool {
    match leer(texto) {
        Some(a) => i.consume_key(a.mods, a.tecla),
        None => false,
    }
}

/// Para la ventana de Ajustes: la primera tecla pulsada en este cuadro (con sus
/// modificadores), si hay.
pub fn capturar(i: &egui::InputState) -> Option<(Modifiers, Key)> {
    i.events.iter().find_map(|e| match e {
        egui::Event::Key { key, pressed: true, modifiers, repeat: false, .. } => Some((*modifiers, *key)),
        _ => None,
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn leer_y_escribir() {
        let a = leer("Ctrl+Shift+F5").unwrap();
        assert!(a.mods.ctrl && a.mods.shift && !a.mods.alt);
        assert_eq!(a.tecla, Key::F5);
        assert_eq!(escribir(a.mods, a.tecla), "Ctrl+Shift+F5");
        assert_eq!(leer("Space").unwrap().tecla, Key::Space);
        assert_eq!(leer("espacio").unwrap().tecla, Key::Space);
        assert!(leer("").is_none());
        assert!(leer("Ctrl+").is_none());
        assert_eq!(bonito("Ctrl+ArrowRight"), "Ctrl + →");
        // Lo que se escribe se vuelve a leer igual.
        for k in [Key::Space, Key::Delete, Key::ArrowRight, Key::F11, Key::A, Key::Num1] {
            assert_eq!(leer(&escribir(Modifiers::NONE, k)).unwrap().tecla, k);
        }
    }
}
