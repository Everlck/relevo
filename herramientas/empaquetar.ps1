# Arma el instalador de Relevo con ffmpeg incluido.
#
# Uso (en PowerShell, desde la carpeta Relevo):
#     powershell -ExecutionPolicy Bypass -File herramientas\empaquetar.ps1
#
# Hace esto:
#   1. Compila Relevo en modo final (cargo build --release).
#   2. Descarga ffmpeg para Windows (solo la primera vez; queda en instalador\ffmpeg).
#   3. Junta todo en instalador\salida\Relevo (programa, config.toml, ffmpeg).
#   4. Si tienes Inno Setup, crea instalador\salida\Relevo-<versión>-Instalador.exe.
#      Si no, crea instalador\salida\Relevo.zip (se descomprime y se usa igual).

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
Set-Location $raiz
$inst = Join-Path $raiz "instalador"
$salida = Join-Path $inst "salida"
$carpeta = Join-Path $salida "Relevo"
$ffdir = Join-Path $inst "ffmpeg"
# La versión sale de Cargo.toml (un solo lugar para el programa y el instalador).
$version = (Select-String -Path (Join-Path $raiz "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Relevo $version" -ForegroundColor Cyan

Write-Host "1/4  Compilando Relevo (puede tardar unos minutos)..." -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "La compilación falló." }

Write-Host "2/4  ffmpeg..." -ForegroundColor Cyan
$ffexe = Join-Path $ffdir "ffmpeg.exe"
if (-not (Test-Path $ffexe)) {
    New-Item -ItemType Directory -Force $ffdir | Out-Null
    $zip = Join-Path $env:TEMP "ffmpeg-relevo.zip"
    $fuentes = @(
        "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip",
        "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
    )
    $ok = $false
    foreach ($url in $fuentes) {
        try {
            Write-Host "     Descargando $url"
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
            $ok = $true
            break
        } catch {
            Write-Host "     No se pudo: $($_.Exception.Message)" -ForegroundColor Yellow
        }
    }
    if (-not $ok) { throw "No se pudo descargar ffmpeg. Descárgalo a mano y deja ffmpeg.exe en $ffdir" }
    $tmp = Join-Path $env:TEMP "ffmpeg-relevo"
    if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
    Expand-Archive -Path $zip -DestinationPath $tmp
    $encontrado = Get-ChildItem -Path $tmp -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
    if (-not $encontrado) { throw "El zip descargado no trae ffmpeg.exe" }
    Copy-Item $encontrado.FullName $ffexe
    $licencia = Get-ChildItem -Path $tmp -Recurse -Include "LICENSE*", "COPYING*" | Select-Object -First 1
    if ($licencia) { Copy-Item $licencia.FullName (Join-Path $ffdir "LICENCIA-ffmpeg.txt") }
    Remove-Item -Recurse -Force $tmp
    Remove-Item -Force $zip
} else {
    Write-Host "     Ya estaba descargado: $ffexe"
}

Write-Host "3/4  Juntando archivos en $carpeta" -ForegroundColor Cyan
if (Test-Path $carpeta) { Remove-Item -Recurse -Force $carpeta }
New-Item -ItemType Directory -Force $carpeta | Out-Null
Copy-Item "target\release\relevo.exe" $carpeta
# La configuración de ejemplo (limpia): tu config.toml no va en el instalador.
Copy-Item "config.ejemplo.toml" (Join-Path $carpeta "config.toml")
Copy-Item "README.md" $carpeta
Copy-Item $ffexe $carpeta
if (Test-Path (Join-Path $ffdir "LICENCIA-ffmpeg.txt")) { Copy-Item (Join-Path $ffdir "LICENCIA-ffmpeg.txt") $carpeta }

Write-Host "4/4  Instalador..." -ForegroundColor Cyan
$iscc = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($iscc) {
    & $iscc "/DVersion=$version" (Join-Path $inst "relevo.iss")
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup falló." }
    Write-Host ""
    Write-Host "Listo: instalador\salida\Relevo-$version-Instalador.exe" -ForegroundColor Green
} else {
    $zipFinal = Join-Path $salida "Relevo.zip"
    if (Test-Path $zipFinal) { Remove-Item -Force $zipFinal }
    Compress-Archive -Path $carpeta -DestinationPath $zipFinal
    Write-Host ""
    Write-Host "Listo: instalador\salida\Relevo.zip (con ffmpeg incluido)" -ForegroundColor Green
    Write-Host "Para tener un instalador .exe, instala Inno Setup y vuelve a ejecutar este script:" -ForegroundColor Yellow
    Write-Host "    winget install JRSoftware.InnoSetup" -ForegroundColor Yellow
}
