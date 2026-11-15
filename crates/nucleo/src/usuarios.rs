//! Usuarios de la red: nombre, rol y contraseña (guardada con argon2, nunca en texto).
//! Se guardan en `datos/usuarios.json`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use serde::{Deserialize, Serialize};

use crate::controlador::Rol;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usuario {
    pub nombre: String,
    pub rol: Rol,
    /// Hash argon2 de la contraseña.
    hash: String,
    /// `true` mientras conserve la contraseña inicial (hay que cambiarla).
    #[serde(default)]
    pub clave_inicial: bool,
}

pub struct Usuarios {
    ruta: PathBuf,
    lista: Mutex<Vec<Usuario>>,
}

fn hash(clave: &str) -> Result<String, String> {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(clave.as_bytes(), &sal).map(|h| h.to_string()).map_err(|e| e.to_string())
}

fn coincide(clave: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(clave.as_bytes(), &h).is_ok())
}

impl Usuarios {
    /// Carga los usuarios. Si no hay ninguno, crea `admin` con contraseña `admin`
    /// (marcada para cambiarla).
    pub fn cargar(carpeta: &Path) -> Result<Usuarios, String> {
        let ruta = carpeta.join("usuarios.json");
        let lista: Vec<Usuario> = match std::fs::read_to_string(&ruta) {
            Ok(t) => serde_json::from_str(&t).map_err(|e| format!("usuarios.json dañado: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        let u = Usuarios { ruta, lista: Mutex::new(lista) };
        if u.lista.lock().expect("usuarios").is_empty() {
            let admin = Usuario { nombre: "admin".into(), rol: Rol::Admin, hash: hash("admin")?, clave_inicial: true };
            u.lista.lock().expect("usuarios").push(admin);
            u.guardar()?;
        }
        Ok(u)
    }

    fn guardar(&self) -> Result<(), String> {
        let lista = self.lista.lock().expect("usuarios").clone();
        if let Some(p) = self.ruta.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let tmp = self.ruta.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&lista).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.ruta).map_err(|e| e.to_string())
    }

    /// Devuelve el rol si el usuario y la contraseña son correctos.
    pub fn verificar(&self, nombre: &str, clave: &str) -> Option<Rol> {
        let lista = self.lista.lock().expect("usuarios");
        let u = lista.iter().find(|u| u.nombre.eq_ignore_ascii_case(nombre.trim()))?;
        coincide(clave, &u.hash).then_some(u.rol)
    }

    pub fn lista(&self) -> Vec<Usuario> {
        self.lista.lock().expect("usuarios").clone()
    }

    /// Crea un usuario o, si ya existe, cambia su rol y contraseña.
    pub fn guardar_usuario(&self, nombre: &str, clave: &str, rol: Rol) -> Result<(), String> {
        let nombre = nombre.trim();
        if nombre.is_empty() {
            return Err("Escribe un nombre de usuario.".into());
        }
        if clave.chars().count() < 4 {
            return Err("La contraseña debe tener al menos 4 caracteres.".into());
        }
        let h = hash(clave)?;
        {
            let mut lista = self.lista.lock().expect("usuarios");
            match lista.iter_mut().find(|u| u.nombre.eq_ignore_ascii_case(nombre)) {
                Some(u) => {
                    if u.rol == Rol::Admin && rol != Rol::Admin && lista_admins(&lista) == 1 {
                        return Err("Debe quedar al menos un administrador.".into());
                    }
                    let u = lista.iter_mut().find(|u| u.nombre.eq_ignore_ascii_case(nombre)).expect("existe");
                    u.rol = rol;
                    u.hash = h;
                    u.clave_inicial = false;
                }
                None => lista.push(Usuario { nombre: nombre.to_string(), rol, hash: h, clave_inicial: false }),
            }
        }
        self.guardar()
    }

    pub fn borrar(&self, nombre: &str) -> Result<(), String> {
        {
            let mut lista = self.lista.lock().expect("usuarios");
            let Some(i) = lista.iter().position(|u| u.nombre.eq_ignore_ascii_case(nombre)) else {
                return Err("No existe ese usuario.".into());
            };
            if lista[i].rol == Rol::Admin && lista_admins(&lista) == 1 {
                return Err("No se puede borrar el último administrador.".into());
            }
            lista.remove(i);
        }
        self.guardar()
    }

    /// `true` si algún usuario conserva la contraseña inicial.
    pub fn hay_clave_inicial(&self) -> bool {
        self.lista.lock().expect("usuarios").iter().any(|u| u.clave_inicial)
    }
}

fn lista_admins(lista: &[Usuario]) -> usize {
    lista.iter().filter(|u| u.rol == Rol::Admin).count()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn carpeta(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("relevo-usuarios-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn crea_admin_inicial_y_verifica() {
        let d = carpeta("a");
        let u = Usuarios::cargar(&d).unwrap();
        assert_eq!(u.verificar("admin", "admin"), Some(Rol::Admin));
        assert_eq!(u.verificar("ADMIN", "admin"), Some(Rol::Admin));
        assert_eq!(u.verificar("admin", "otra"), None);
        assert!(u.hay_clave_inicial());
        // La contraseña no se guarda en texto.
        let texto = std::fs::read_to_string(d.join("usuarios.json")).unwrap();
        assert!(!texto.contains("\"admin\",\n    \"rol\": \"Admin\",\n    \"hash\": \"admin\""));
        assert!(texto.contains("$argon2"));
    }

    #[test]
    fn crear_cambiar_y_borrar() {
        let d = carpeta("b");
        let u = Usuarios::cargar(&d).unwrap();
        u.guardar_usuario("Juan", "1234", Rol::Operador).unwrap();
        assert_eq!(u.verificar("juan", "1234"), Some(Rol::Operador));
        assert!(u.guardar_usuario("Ana", "12", Rol::Editor).is_err());
        u.guardar_usuario("admin", "nueva-clave", Rol::Admin).unwrap();
        assert!(!u.hay_clave_inicial());
        // No se puede dejar sin administradores.
        assert!(u.borrar("admin").is_err());
        assert!(u.guardar_usuario("admin", "nueva-clave", Rol::Operador).is_err());
        u.borrar("Juan").unwrap();
        // Persisten al volver a cargar.
        let u2 = Usuarios::cargar(&d).unwrap();
        assert_eq!(u2.verificar("admin", "nueva-clave"), Some(Rol::Admin));
        assert_eq!(u2.lista().len(), 1);
    }
}
