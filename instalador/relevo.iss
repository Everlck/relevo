; Instalador de Relevo (Inno Setup 6). No se compila a mano: lo usa herramientas\empaquetar.ps1,
; que antes deja todo en instalador\salida\Relevo (programa, config.toml y ffmpeg).

#define Nombre "Relevo"
; La versión la pasa empaquetar.ps1 (la toma de Cargo.toml); esta es por si se compila a mano.
#ifndef Version
  #define Version "1.0.0"
#endif

[Setup]
; Si Relevo está abierto, se cierra antes de reemplazarlo (al actualizar desde Relevo).
CloseApplications=force
AppId={{6C1C6E57-3B0A-4E0F-9C55-1E6B2D7A2F31}
AppName={#Nombre} · Control de publicidad
AppVersion={#Version}
AppPublisher=Relevo
; En la carpeta del usuario: no pide permisos de administrador y la app puede
; guardar ahí sus datos (pautas, registro, tandas).
DefaultDirName={localappdata}\Programs\{#Nombre}
DefaultGroupName={#Nombre}
PrivilegesRequired=lowest
OutputDir=salida
OutputBaseFilename=Relevo-{#Version}-Instalador
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\relevo.exe
; Ícono del instalador (el mismo que lleva relevo.exe por dentro).
SetupIconFile=..\crates\app\recursos\relevo.ico

[Languages]
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "escritorio"; Description: "Crear un acceso directo en el Escritorio"
Name: "cortafuegos"; Description: "Permitir el modo remoto (abrir el puerto 7800 en el Firewall de Windows; pedirá permiso)"; Flags: unchecked

[Files]
Source: "salida\Relevo\relevo.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "salida\Relevo\ffmpeg.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "salida\Relevo\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "salida\Relevo\LICENCIA-ffmpeg.txt"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
; La configuración no se pisa al actualizar.
Source: "salida\Relevo\config.toml"; DestDir: "{app}"; Flags: onlyifdoesntexist uninsneveruninstall

[Icons]
Name: "{group}\{#Nombre}"; Filename: "{app}\relevo.exe"; WorkingDir: "{app}"; IconFilename: "{app}\relevo.exe"
Name: "{group}\Desinstalar {#Nombre}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\{#Nombre}"; Filename: "{app}\relevo.exe"; WorkingDir: "{app}"; IconFilename: "{app}\relevo.exe"; Tasks: escritorio

[Run]
Filename: "netsh"; Parameters: "advfirewall firewall add rule name=""Relevo remoto"" dir=in action=allow protocol=TCP localport=7800 profile=private"; Verb: runas; Flags: shellexec runhidden waituntilterminated; Tasks: cortafuegos; StatusMsg: "Abriendo el puerto 7800..."; Check: not WizardSilent
Filename: "{app}\relevo.exe"; Description: "Abrir Relevo"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent
; Actualización desde Relevo (instalación silenciosa): se vuelve a abrir solo.
Filename: "{app}\relevo.exe"; WorkingDir: "{app}"; Flags: nowait; Check: WizardSilent
