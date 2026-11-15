//! En Windows, mete el ícono de Relevo dentro de relevo.exe (accesos directos,
//! barra de tareas, Explorador) y sus datos (nombre, versión).

fn main() {
    println!("cargo:rerun-if-changed=recursos/relevo.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("recursos/relevo.ico");
    // Datos completos del programa (los antivirus desconfían menos de un .exe que dice
    // qué es y quién lo hace).
    res.set("ProductName", "Relevo");
    res.set("FileDescription", "Relevo · Control de publicidad para CasparCG");
    res.set("CompanyName", "Relevo");
    res.set("LegalCopyright", "© Everlck · Licencia MIT");
    res.set("OriginalFilename", "relevo.exe");
    res.set("InternalName", "relevo");
    res.set("Comments", "https://github.com/Everlck/relevo");
    // Manifiesto: se ejecuta como el usuario (sin pedir permisos de administrador) y
    // declara las versiones de Windows en las que funciona.
    res.set_manifest(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Relevo" version="1.0.0.0"/>
  <description>Relevo - Control de publicidad para CasparCG</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
    </application>
  </compatibility>
</assembly>
"#,
    );
    if let Err(e) = res.compile() {
        // Sin el ícono el programa funciona igual: solo se avisa.
        println!("cargo:warning=no se pudo poner el ícono en relevo.exe: {e}");
    }
}
