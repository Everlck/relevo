//! Ventana "Usuarios" (solo en la PC principal): quién puede conectarse desde otras PCs.

use std::sync::Arc;

use egui::{RichText, Vec2};
use nucleo::usuarios::Usuarios;
use nucleo::Rol;

use crate::tema::*;

pub struct VentanaUsuarios {
    pub abierta: bool,
    nombre: String,
    clave: String,
    rol: Rol,
    mensaje: Option<(String, bool)>,
    confirmar_borrar: Option<String>,
}

impl VentanaUsuarios {
    pub fn new() -> Self {
        VentanaUsuarios {
            abierta: false,
            nombre: String::new(),
            clave: String::new(),
            rol: Rol::Operador,
            mensaje: None,
            confirmar_borrar: None,
        }
    }

    pub fn mostrar(&mut self, ctx: &egui::Context, usuarios: &Arc<Usuarios>, puerto: Option<u16>) {
        if !self.abierta {
            return;
        }
        let mut abierta = true;
        egui::Window::new("Usuarios y modo remoto")
            .open(&mut abierta)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                match puerto {
                    Some(p) => ui.label(
                        RichText::new(format!(
                            "Las otras PCs se conectan con Relevo en modo \"remoto\", a la IP de esta PC y el puerto {p}."
                        ))
                        .color(TENUE),
                    ),
                    None => ui.label(
                        RichText::new("El modo remoto está apagado (servir = false en config.toml).").color(AMBAR),
                    ),
                };
                ui.add_space(6.0);

                egui::Grid::new("lista_usuarios").num_columns(3).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                    ui.label(RichText::new("Usuario").strong().family(crate::tema::negrita()));
                    ui.label(RichText::new("Rol").strong().family(crate::tema::negrita()));
                    ui.label("");
                    ui.end_row();
                    for u in usuarios.lista() {
                        ui.horizontal(|ui| {
                            ui.label(&u.nombre);
                            if u.clave_inicial {
                                ui.label(RichText::new("⚠ contraseña inicial").size(11.0).color(AMBAR));
                            }
                        });
                        ui.label(u.rol.nombre());
                        ui.horizontal(|ui| {
                            if ui.small_button("Editar").clicked() {
                                self.nombre = u.nombre.clone();
                                self.rol = u.rol;
                                self.clave.clear();
                            }
                            let confirmando = self.confirmar_borrar.as_deref() == Some(u.nombre.as_str());
                            let t = if confirmando { "¿Seguro?" } else { "Borrar" };
                            if ui.small_button(RichText::new(t).color(AIRE)).clicked() {
                                if confirmando {
                                    self.mensaje = Some(match usuarios.borrar(&u.nombre) {
                                        Ok(()) => (format!("Se borró {}", u.nombre), false),
                                        Err(e) => (e, true),
                                    });
                                    self.confirmar_borrar = None;
                                } else {
                                    self.confirmar_borrar = Some(u.nombre.clone());
                                }
                            }
                        });
                        ui.end_row();
                    }
                });

                ui.separator();
                ui.label(RichText::new("Crear usuario o cambiar contraseña").strong().family(crate::tema::negrita()));
                egui::Grid::new("form_usuario").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Usuario");
                    ui.add(egui::TextEdit::singleline(&mut self.nombre).desired_width(200.0));
                    ui.end_row();
                    ui.label("Contraseña");
                    ui.add(egui::TextEdit::singleline(&mut self.clave).password(true).desired_width(200.0));
                    ui.end_row();
                    ui.label("Rol");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.rol, Rol::Operador, "Operador")
                            .on_hover_text("Emite con el mando y ve todo");
                        ui.radio_value(&mut self.rol, Rol::Editor, "Editor")
                            .on_hover_text("Además programa la pauta y sube videos");
                        ui.radio_value(&mut self.rol, Rol::Admin, "Administrador")
                            .on_hover_text("Además puede quitar el mando a otro");
                    });
                    ui.end_row();
                });
                ui.add_space(6.0);
                if ui
                    .add(egui::Button::new("Guardar usuario").fill(ACENTO_OSCURO).min_size(Vec2::new(140.0, 30.0)))
                    .clicked()
                {
                    self.mensaje = Some(match usuarios.guardar_usuario(&self.nombre, &self.clave, self.rol) {
                        Ok(()) => {
                            let m = format!("Guardado: {} ({})", self.nombre.trim(), self.rol.nombre());
                            self.nombre.clear();
                            self.clave.clear();
                            (m, false)
                        }
                        Err(e) => (e, true),
                    });
                }
                if let Some((m, error)) = &self.mensaje {
                    ui.label(RichText::new(m).color(if *error { AIRE } else { TV }));
                }
            });
        self.abierta = abierta;
    }
}
