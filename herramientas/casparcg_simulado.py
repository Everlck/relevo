"""CasparCG simulado para probar la app sin tener CasparCG instalado.

Uso:
    python herramientas/casparcg_simulado.py [--medios CARPETA] [--puerto 5250] [--osc 6250]

- Con --medios, lista los videos reales de esa carpeta (como la carpeta "media"
  de CasparCG) e informa esa ruta en INFO PATHS. Sin --medios usa una lista fija.
- Responde a VERSION, INFO PATHS, CLS, THUMBNAIL RETRIEVE, PLAY, LOADBG, PAUSE,
  RESUME, STOP, CLEAR, CALL (LOOP y SEEK) y MIXER, y muestra en pantalla cada comando.
- Envía por OSC el tiempo del clip al aire (como CasparCG 2.3), y encadena los
  clips precargados con AUTO.
"""
import argparse
import base64
import os
import socket
import socketserver
import struct
import threading
import time
import zlib

EXT_VIDEO = {".mp4", ".mov", ".m4v", ".mxf", ".avi", ".mkv", ".mpg", ".mpeg", ".ts", ".wmv", ".webm"}
FIJOS = [
    ("CLIENTES/FERRETERIA X/SPOT 30S", 30.0),
    ("CLIENTES/FARMACIA SALUD/SPOT 20S", 20.0),
    ("CLIENTES/RESTAURANTE EL SOL", 15.0),
    ("PROMOS/VOLVEMOS EN BREVE", 5.0),
    ("IDS/ID CANAL", 10.0),
]
NTSC = 30000 / 1001


def duracion_mp4(ruta):
    """Lee moov/mvhd de un MP4/MOV. Devuelve segundos o None."""
    try:
        with open(ruta, "rb") as f:
            datos = f.read()
    except OSError:
        return None

    def caja(ini, fin, tipo):
        pos = ini
        while pos + 8 <= fin:
            tam, t = struct.unpack(">I4s", datos[pos:pos + 8])
            cab = 8
            if tam == 1:
                tam = struct.unpack(">Q", datos[pos + 8:pos + 16])[0]
                cab = 16
            elif tam == 0:
                tam = fin - pos
            if tam < cab:
                return None
            if t == tipo:
                return pos + cab, pos + tam
            pos += tam
        return None

    moov = caja(0, len(datos), b"moov")
    if not moov:
        return None
    mvhd = caja(moov[0], moov[1], b"mvhd")
    if not mvhd:
        return None
    p = mvhd[0]
    if datos[p] == 1:
        escala, dur = struct.unpack(">IQ", datos[p + 20:p + 32])
    else:
        escala, dur = struct.unpack(">II", datos[p + 12:p + 20])
    return dur / escala if escala else None


def png(ancho, alto, semilla):
    """PNG de color (degradado) para las miniaturas."""
    h = zlib.crc32(semilla.encode()) & 0xFFFFFF
    r0, g0, b0 = (h >> 16) & 0xFF, (h >> 8) & 0xFF, h & 0xFF
    filas = b""
    for y in range(alto):
        fila = bytearray([0])
        for x in range(ancho):
            k = 0.35 + 0.65 * (x / ancho)
            banda = 1.0 if (alto * 0.62 < y < alto * 0.80) else 0.0
            r = int(min(255, r0 * k * 0.8 + 40 * banda))
            g = int(min(255, g0 * k * 0.8 + 40 * banda))
            b = int(min(255, b0 * k * 0.8 + 40 * banda))
            fila += bytes((r, g, b))
        filas += bytes(fila)

    def trozo(tipo, cont):
        return struct.pack(">I", len(cont)) + tipo + cont + struct.pack(">I", zlib.crc32(tipo + cont) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + trozo(b"IHDR", struct.pack(">IIBBBBB", ancho, alto, 8, 2, 0, 0, 0))
            + trozo(b"IDAT", zlib.compress(filas, 9)) + trozo(b"IEND", b""))


def _relleno(b):
    b += b"\x00"
    return b + b"\x00" * ((4 - len(b) % 4) % 4)


def osc_mensaje(direccion, *floats):
    return _relleno(direccion.encode()) + _relleno(("," + "f" * len(floats)).encode()) + b"".join(struct.pack(">f", v) for v in floats)


def osc_enteros(direccion, *enteros):
    return _relleno(direccion.encode()) + _relleno(("," + "i" * len(enteros)).encode()) + b"".join(struct.pack(">i", v) for v in enteros)


FPS_NUM, FPS_DEN = 30000, 1001


def transicion_de(cmd):
    """Segundos de la transición de un PLAY/LOADBG (MIX 15 → 15 cuadros)."""
    p = cmd.upper().split()
    for i, w in enumerate(p):
        if w in ("MIX", "WIPE", "PUSH", "SLIDE") and i + 1 < len(p) and p[i + 1].isdigit():
            return int(p[i + 1]) * FPS_DEN / FPS_NUM
    return 0.0


def osc_texto(direccion, texto):
    return _relleno(direccion.encode()) + _relleno(b",s") + _relleno(texto.encode())


class Estado:
    def __init__(self, medios, osc_puerto):
        self.medios = medios
        self.osc_puerto = osc_puerto
        self.lock = threading.Lock()
        self.capas = {}  # (canal, capa) -> dict(fg, total, inicio, pausado_en, bg, bg_total, bucle)
        self.udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

    def lista(self):
        if not self.medios:
            return list(FIJOS)
        res = []
        for raiz, _, archivos in os.walk(self.medios):
            for a in archivos:
                base, ext = os.path.splitext(a)
                if ext.lower() not in EXT_VIDEO:
                    continue
                rel = os.path.relpath(os.path.join(raiz, base), self.medios).replace("\\", "/")
                dur = duracion_mp4(os.path.join(raiz, a)) or 10.0
                res.append((rel.upper(), dur))
        return sorted(res)

    def buscar(self, nombre):
        # Rutas completas (como acepta CasparCG 2.3 para archivos fuera de "media").
        if os.path.isabs(nombre) or (len(nombre) > 2 and nombre[1] == ":"):
            if os.path.isfile(nombre):
                return duracion_mp4(nombre) or 10.0
            return None
        nombre = nombre.upper()
        for n, d in self.lista():
            if n == nombre:
                return d
        return None

    def ruta(self, nombre):
        """Ruta del archivo como la informa CasparCG 2.3 (foreground/file/path)."""
        if os.path.isabs(nombre):
            return nombre
        if self.medios:
            for raiz, _, archivos in os.walk(self.medios):
                for a in archivos:
                    base, ext = os.path.splitext(a)
                    rel = os.path.relpath(os.path.join(raiz, base), self.medios).replace("\\", "/")
                    if rel.upper() == nombre.upper():
                        return os.path.join(raiz, a)
        return "media/" + nombre.lower() + ".mp4"

    def bucle_osc(self):
        # Como CasparCG: el estado de cada cuadro (25 por segundo), la ruta antes que el tiempo.
        while True:
            time.sleep(0.04)
            if not self.osc_puerto:
                continue
            ahora = time.time()
            with self.lock:
                # Cuadros por segundo y niveles de audio de la salida (como CasparCG 2.3+).
                suena = any(c.get("fg") and not str(c.get("fg")).startswith("#") and c.get("pausado_en") is None
                            for c in self.capas.values())
                import math, random
                for canal in sorted({k[0] for k in self.capas} or {1}):
                    self.udp.sendto(osc_enteros(f"/channel/{canal}/framerate", FPS_NUM, FPS_DEN), ("127.0.0.1", self.osc_puerto))
                    if suena:
                        base = 0.35 + 0.25 * math.sin(ahora * 3.1)
                        l = max(0.0, min(1.0, base + random.uniform(-0.12, 0.12)))
                        r = max(0.0, min(1.0, base * 0.9 + random.uniform(-0.12, 0.12)))
                    else:
                        l = r = 0.0
                    self.udp.sendto(osc_enteros(f"/channel/{canal}/mixer/audio/volume", int(l * 2147483647), int(r * 2147483647)), ("127.0.0.1", self.osc_puerto))
                for (canal, capa), c in self.capas.items():
                    if not c.get("fg"):
                        continue
                    t = (c["pausado_en"] if c.get("pausado_en") is not None else ahora) - c["inicio"]
                    if t < 0:
                        continue
                    if c["bucle"] and c["total"]:
                        t %= c["total"]
                    # Con transición, el precargado entra unos cuadros antes del final.
                    if t >= c["total"] - c.get("bg_trans", 0.0) and not c["bucle"] and c.get("bg"):
                        if c.get("bg"):
                            # AUTO: entra el precargado.
                            c["inicio"] += c["total"] - c.get("bg_trans", 0.0)
                            c["fg"], c["total"], c["bg"] = c["bg"], c["bg_total"], None
                            print(f"  [AUTO] {canal}-{capa}: {c['fg']}" + (f" (transición {c.get('bg_trans', 0):.2f} s)" if c.get("bg_trans") else ""))
                            if c["fg"].startswith("#"):
                                c["fg"] = None
                            continue
                    if t >= c["total"] and not c["bucle"]:
                        t = c["total"]
                    base = f"/channel/{canal}/stage/layer/{capa}/foreground/"
                    if "://" in str(c["fg"]):
                        # Canal de red: durante el corte simulado no llega nada.
                        if IPTV_CORTE and IPTV_CORTE[0] <= ahora - INICIO < IPTV_CORTE[1]:
                            continue
                        self.udp.sendto(osc_texto(base + "producer", "ffmpeg"), ("127.0.0.1", self.osc_puerto))
                        self.udp.sendto(osc_mensaje(base + "file/time", t, 0.0), ("127.0.0.1", self.osc_puerto))
                        continue
                    if not SIN_RUTA:
                        self.udp.sendto(osc_texto(base + "file/path", self.ruta(c["fg"])), ("127.0.0.1", self.osc_puerto))
                    # El CasparCG real informa el tiempo con unos cuadros de atraso.
                    msg = osc_mensaje(base + "file/time", max(0.0, t - ATRASO_OSC), c["total"])
                    self.udp.sendto(msg, ("127.0.0.1", self.osc_puerto))


ESTADO = None
RETARDO = 0.3
SIN_RUTA = False
ATRASO_OSC = 0.0
EXTRA = 0.0
CLS_FALLA = 0.0
IPTV_CORTE = None
INICIO = time.time()


def cuadro_seek(cmd):
    """El número tras SEEK (0 si no hay)."""
    p = cmd.upper().split()
    if "SEEK" in p:
        i = p.index("SEEK")
        if i + 1 < len(p) and p[i + 1].isdigit():
            return int(p[i + 1])
    return 0


def capa_de(cmd):
    try:
        cc = cmd.split()[1].split("-")
        return int(cc[0]), int(cc[1])
    except (IndexError, ValueError):
        return 1, 10


def clip_de(cmd):
    if '"' in cmd:
        return cmd.split('"')[1]
    partes = cmd.split()
    return partes[2] if len(partes) > 2 else ""


class Manejador(socketserver.StreamRequestHandler):
    def handle(self):
        print(f"cliente conectado: {self.client_address}")
        for linea in self.rfile:
            cmd = linea.decode("utf-8", "replace").strip()
            if not cmd:
                continue
            palabra = cmd.split()[0].upper()
            if palabra not in ("VERSION", "THUMBNAIL"):
                print(f"  <- {cmd}")
            self.wfile.write(self.responder(cmd, palabra).encode())
        print(f"cliente desconectado: {self.client_address}")

    def responder(self, cmd, palabra):
        e = ESTADO
        if palabra == "VERSION":
            return "201 VERSION OK\r\n2.3.3 SIMULADO\r\n"
        if cmd.upper() == "INFO PATHS":
            media = (os.path.abspath(e.medios) + os.sep) if e.medios else "media/"
            xml = ['<?xml version="1.0" encoding="utf-8"?>', "<paths>", f"   <media-path>{media}</media-path>",
                   "   <log-path>log/</log-path>", f"   <initial-path>{os.getcwd()}{os.sep}</initial-path>", "</paths>"]
            return "201 INFO OK\r\n" + "\r\n".join(xml) + "\r\n"
        if palabra == "CLS":
            # Como CasparCG 2.3 mientras su media scanner arranca.
            if time.time() < INICIO + CLS_FALLA:
                return "501 CLS FAILED\r\n"
            filas = []
            for n, d in e.lista():
                cuadros = int(round(d * NTSC))
                filas.append(f'"{n}" MOVIE {cuadros * 10000} 20261111120000 {cuadros} 1001/30000')
            return "200 CLS OK\r\n" + "".join(f + "\r\n" for f in filas) + "\r\n"
        if palabra == "THUMBNAIL":
            nombre = clip_de(cmd)
            if e.buscar(nombre) is None:
                return "404 THUMBNAIL RETRIEVE ERROR\r\n"
            return "201 THUMBNAIL RETRIEVE OK\r\n" + base64.b64encode(png(256, 144, nombre.upper())).decode() + "\r\n"
        if palabra == "PLAY" and len(cmd.split()) == 2:
            # PLAY sin clip: pasa a primer plano lo precargado.
            canal, capa = capa_de(cmd)
            with e.lock:
                c = e.capas.get((canal, capa))
                if c and c.get("bg"):
                    fg = None if c["bg"].startswith("#") else c["bg"]
                    c.update(fg=fg, total=c["bg_total"], inicio=time.time(), pausado_en=None, bg=None)
            return "202 PLAY OK\r\n"
        if palabra in ("PLAY", "LOADBG"):
            canal, capa = capa_de(cmd)
            nombre = clip_de(cmd)
            if nombre.startswith("#") or "://" in nombre:
                # Color o canal de red (IPTV): no termina nunca.
                total = 1e9
            else:
                total = e.buscar(nombre)
                if total is None:
                    return f"404 {palabra} FAILED\r\n"
                # El CasparCG real puede reproducir el archivo un poco más largo de lo que
                # dice su cabecera (por ejemplo, si el audio dura más que el video).
                total += EXTRA
            with e.lock:
                c = e.capas.setdefault((canal, capa), {"fg": None, "total": 0, "inicio": 0, "bg": None, "bucle": False})
                if palabra == "PLAY":
                    # CasparCG tarda un poco en abrir el archivo antes de que empiece a verse.
                    # SEEK n: empieza en el cuadro n (cuadros del canal).
                    desde = cuadro_seek(cmd) / NTSC
                    c.update(fg=nombre if (os.path.isabs(nombre) or "://" in nombre) else nombre.upper(), total=total, inicio=time.time() + RETARDO - desde, pausado_en=None, bg=None,
                             bucle=" LOOP" in cmd.upper())
                else:
                    c.update(bg=nombre.upper(), bg_total=total, bg_trans=transicion_de(cmd))
            return f"202 {palabra} OK\r\n"
        if palabra in ("PAUSE", "RESUME", "STOP", "CLEAR", "CALL", "MIXER"):
            canal, capa = capa_de(cmd)
            with e.lock:
                c = e.capas.get((canal, capa))
                if c:
                    if palabra == "PAUSE" and c.get("pausado_en") is None:
                        c["pausado_en"] = time.time()
                    elif palabra == "RESUME" and c.get("pausado_en") is not None:
                        c["inicio"] += time.time() - c["pausado_en"]
                        c["pausado_en"] = None
                    elif palabra in ("STOP", "CLEAR"):
                        c.update(fg=None, bg=None)
                    elif palabra == "CALL" and " SEEK " in cmd.upper():
                        # Salta a ese cuadro: el clip sigue desde ahí.
                        ref = c["pausado_en"] if c.get("pausado_en") is not None else time.time()
                        c["inicio"] = ref - cuadro_seek(cmd) / NTSC
                    elif palabra == "CALL" and "LOOP" in cmd.upper():
                        c["bucle"] = cmd.strip().endswith("1")
            return f"202 {palabra} OK\r\n"
        return "400 ERROR\r\n"


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description="CasparCG simulado")
    ap.add_argument("--medios", help="carpeta con videos que hará de carpeta 'media'")
    ap.add_argument("--puerto", type=int, default=5250)
    ap.add_argument("--osc", type=int, default=6250, help="puerto OSC de la app (0 = no enviar)")
    ap.add_argument("--retardo", type=float, default=0.3, help="segundos que tarda un PLAY en empezar")
    ap.add_argument("--sin-ruta", action="store_true", help="no informar la ruta por OSC (como CasparCG 2.0)")
    ap.add_argument("--atraso-osc", type=float, default=0.0, help="segundos de atraso del tiempo informado por OSC")
    ap.add_argument("--cls-falla", type=float, default=0.0, help="segundos iniciales en que CLS responde 501 (scanner arrancando)")
    ap.add_argument("--extra", type=float, default=0.0, help="segundos que el clip dura de más respecto a su archivo")
    ap.add_argument("--iptv-corte", help="A:B = el canal de red deja de llegar entre los segundos A y B")
    a = ap.parse_args()
    RETARDO = a.retardo
    SIN_RUTA = a.sin_ruta
    ATRASO_OSC = a.atraso_osc
    EXTRA = a.extra
    CLS_FALLA = a.cls_falla
    if a.iptv_corte:
        x, y = a.iptv_corte.split(":")
        IPTV_CORTE = (float(x), float(y))
    ESTADO = Estado(a.medios, a.osc)
    threading.Thread(target=ESTADO.bucle_osc, daemon=True).start()
    socketserver.ThreadingTCPServer.allow_reuse_address = True
    with socketserver.ThreadingTCPServer(("127.0.0.1", a.puerto), Manejador) as servidor:
        print(f"CasparCG simulado en 127.0.0.1:{a.puerto} · medios: {a.medios or '(lista fija)'} · OSC → {a.osc}")
        servidor.serve_forever()
