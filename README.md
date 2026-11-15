# Relevo · App de control de publicidad

**Versión 1.0.0** · Windows · para CasparCG Server 2.3 o posterior

Programa de escritorio para Windows que controla **CasparCG Server** y pone al aire la **publicidad local** sobre la señal de un canal: arma las tandas, las programa por horario (pauta), detecta los cortes del canal y deja constancia de cada salida.

Hecho en Rust, con interfaz **egui**.

## Qué hace

**Se pone en marcha sola**
- Busca la carpeta de CasparCG (C:\, D:\, Archivos de programa, Escritorio, Descargas…).
- Si CasparCG o su scanner no están abiertos, **los abre**.
- Activa el **tiempo exacto (OSC)** en `casparcg.config` (guarda antes una copia `casparcg.config.antes-de-relevo`; si ya había una de antes, `antes-de-p-pv`, esa se respeta como la original). Si CasparCG ya estaba abierto, aparece un botón para reiniciarlo cuando no haya nada al aire.

**Pauta y modo AUTO**
- Pestaña **Publicidades** (F5; también el botón *🕓 Publicidades* de la Pauta de hoy o *Programar este spot* en el clic derecho de un video):
  - **Izquierda:** todas las publicidades **agrupadas por cliente**, con buscador y filtros (*Todas, Hoy, Activas, Pausadas, Terminadas*). Cada una muestra su estado, días y horas; al pasar el mouse, ⏸ / ⏵ la pausa o la activa sin abrirla.
  - **En el medio, la ficha:** *Spot* (cliente —con los que ya existen a mano—, nombre, video de CasparCG con su miniatura y duración), *Vigencia* (desde / hasta, con atajos: 1 semana, 15 días, 1 mes, 3 meses, fin de mes), *Días* (todos, algunos, intercalado o fechas sueltas), *Horarios* (fichas de hora; **pegar la lista entera del cliente** en 24 h o con AM/PM —«6:00, 8:30, 10:45 ... 8:00PM, 10 pm»—, **plantillas de horario** con nombre para reusar en otras publicidades, **copiar las horas de otra publicidad**, «Repetir cada 2 horas desde 08:00 hasta 20:00»; y **qué tan exacta es la hora**: minutos *antes* y *después* por separado, con atajos Exacta / Normal / Suele atrasarse / Flexible) y *Opciones* (prioridad, activa, notas).
  - **Guardar / Descartar** abajo (Ctrl+S); avisa si hay cambios sin guardar antes de abrir otra. *Duplicar* crea otra igual; *Borrar* pide confirmar.
  - **Derecha, cómo queda:** sus pases de hoy (salió, ahora, pendiente, perdido, programado) y los **próximos 7 días** como líneas de 0 a 24 h con un punto por pase, con el total de pases y el tiempo al aire.
  - El **cliente** se usa en los reportes (si está vacío, el nombre).
- **Pauta de hoy** (derecha): cada pase con su hora y estado: **AHORA** (en ventana), **✔** emitido, **PENDIENTE** (no salió en su horario o se cortó: se recupera solo), **PERDIDO** (ya no se recupera; con **+** se añade a la tanda) o programado. Al lado, **×3**: cuántas veces salió hoy ese video (con relleno y repeticiones).
- Modo **AUTO**: la tanda se arma sola. El operador solo pulsa **Play (Espacio)** cuando empieza el corte. Orden:
  1. Los **prioritarios** en su ventana (★★ antes que ★).
  2. Si la tanda nueva ya toca (llegó su hora o faltan menos de 5 min): sus spots y **después lo pendiente** de tandas anteriores. Si todavía no toca, primero lo pendiente.
  3. Un mismo spot no se repite en la tanda.
- **Si detienes una tanda a mitad** (por ejemplo, en el spot 6 de 10): el 6 se cortó, así que **no cuenta** y vuelve a empezar; siguen el 7, 8, 9 y 10. Si mientras tanto ya toca otra tanda, va según el orden de arriba.
- **Recuperar lo que no salió** (⚙ Ajustes › Pauta, por defecto 3 h): un pase que no salió en su horario (el canal no siempre respeta las horas) o que se cortó queda **PENDIENTE** y entra solo en el próximo corte, hasta 3 h después de su horario. Con 0, no se recupera.
- Un pase cuenta como emitido cuando sale **completo** (se guarda en `datos/emitidos/AAAA-MM-DD.json`) y no se repite.
- La pauta se guarda en `datos/pautas.json`.

**Archivos (izquierda): como el Explorador de Windows**
- Arriba, a todo el ancho: la ruta (⬅ ➡ ⬆, migas de pan, ✏, 📌), el buscador y el orden. Debajo, el panel de carpetas y la lista.
- **Cómo se ven los videos** (botones arriba a la derecha): **⊞ Miniaturas** o **☰ Lista** (una línea por video, sin miniaturas: caben muchos más). Se recuerda.
- **Clic** en un video solo lo marca (Enter o doble clic lo añaden). **Modo elegir:** se activa con Ctrl+clic, Mayús+clic, la **casilla** que aparece a la izquierda al pasar por ahí (arrastrar por las casillas elige varios) o un **marco** arrastrando desde un espacio vacío. Ctrl+A elige todos; Esc o × deja de elegir. Al cambiar de carpeta no queda nada elegido.
- El panel de carpetas solo despliega una carpeta cuando la abres con ▸ (no se abre solo hasta la carpeta actual).
- **Arrastrar a la tanda:** si arrastras uno de los elegidos, van **todos**, en su orden, donde los sueltes.
- **Pie de Archivos:** cuántas carpetas y videos hay y cuánto duran; con videos elegidos, **+ Añadir a la tanda** (Enter), **⏭ a continuación** y × para dejar de elegir. El clic derecho sobre varios elegidos también los añade.
- Panel de carpetas (se oculta con **☰**):
  - **★ Acceso rápido:** CasparCG, Escritorio, Descargas, Documentos, Imágenes, Música, Videos y las carpetas que anclas.
  - **💻 Este equipo:** las unidades (C:, D:, E:…) en árbol; cada carpeta se despliega con ▸.
- **Anclar una carpeta:** botón **📌** junto a la ruta, o clic derecho sobre una carpeta → *Anclar a acceso rápido*. Clic derecho sobre una anclada: *Quitar*, *Subir*, *Bajar*. Se guardan en `datos\accesos.json` de cada PC.
- Atrás, subir y una ruta que se puede escribir a mano (Enter para ir).
- **Buscador** en la carpeta actual y sus subcarpetas.
- Cada video muestra su miniatura, duración, tamaño y una etiqueta: **CasparCG** (ya está en su carpeta), **sin copiar** (se reproduce desde donde está), **se copiará** o, en una PC remota, **se subirá**.
- **Doble clic** en un video: se añade al final de la tanda. **Clic derecho:** *Añadir*, *Poner como siguiente*, *Al aire ahora*, *Vista previa*, *Programar este spot* (abre Publicidades con ese video).
- **Vista previa dentro de la app** (clic derecho → *Vista previa*): una ventanita que se puede mover y ensanchar, con play/pausa, barra para moverse, **sonido** y volumen. No sale al aire. Necesita **ffmpeg**.
  - **⚙ Salida de sonido:** elige por dónde suena (por ejemplo, los audífonos). Usa una salida que **no** vaya a la mezcladora.
  - Por seguridad, en la PC principal la vista previa **se calla sola mientras hay publicidad al aire** (`silenciar_al_aire`).

**Tanda (centro)**
- Añadir **arrastrando el video desde la columna Archivos** (o desde la lista de CasparCG) y soltándolo **donde quieras en la tanda**: una línea azul marca dónde va a caer. También con doble clic, Enter, clic derecho (*Añadir*, *Poner como siguiente*, *Al aire ahora*) o **arrastrando videos desde el Explorador de Windows**.
- **Cambiar el orden arrastrando** un spot hacia arriba o abajo (la lista se desplaza sola si lo llevas al borde).
- Si el video está fuera de la carpeta de CasparCG, **se usa directamente desde su carpeta** (CasparCG 2.3 acepta rutas completas; la app lo comprueba al conectar). Si CasparCG no lo acepta, o se elige `externos = "copiar"`, se **copia** a `media\IMPORTADOS` (con barra de progreso) y se reutiliza si ya estaba.
- Ojo: si se borra o mueve el video original, ese spot deja de salir. Para videos que se usan siempre conviene la carpeta de CasparCG.
- **Arriba:** MANUAL / AUTO, **⊞ Miniaturas | ☰ Lista** (cómo se ven los spots; se recuerda) y **📁 Tandas** (guardar la tanda, guardar solo los elegidos, abrir una guardada o borrarla).
- **Elegir spots:**
  - **Clic** en un spot solo lo marca (abajo aparece *⏵ Desde aquí*). **Ctrl+clic** suma o quita uno y activa el modo elegir; **Mayús+clic** elige desde el último hasta ese.
  - **Con el mouse:** arrastra sobre los **números** de la izquierda, o desde un espacio vacío de la lista: un marco elige todos los que toca (con Ctrl, se suman a los que ya estaban).
  - **Ctrl+A** (con el puntero sobre la tanda) elige todos; **Esc** o clic en un espacio vacío deja de elegir.
- **Abajo (pie de la tanda):** sin nada elegido, cuántos spots, el total y, al aire, cuánto queda; además *Vaciar*. **Con spots elegidos, ahí mismo aparecen sus acciones:** ⏵ al aire ahora (si es uno), ⏭ a continuación, ⏶ subir, ⏷ bajar, **…** (al principio, al final, duplicar, guardar como tanda, elegir todos), ✖ quitar y × dejar de elegir.
- Arrastrando uno de los elegidos se mueven todos juntos. Al pasar el mouse por un spot aparece su ✖ para quitarlo.
- **Clic derecho** sobre un spot (o sobre los elegidos): poner al aire ahora, a continuación, subir, bajar, al principio, al final, duplicar, guardar como tanda, vista previa, **mostrar en Archivos**, elegir todos y quitar.
- Atajos: Supr quita lo elegido, Ctrl+↑/↓ lo mueve, Ctrl+D lo duplica.
- Las copias hechas con *Duplicar* no cumplen ningún pase de la pauta.
- **Guardar y abrir tandas** con nombre (se guardan en la carpeta `tandas`, junto a `config.toml`).

**Tamaño de las secciones**
- Entre Al aire, Archivos, Tanda y Pauta (y dentro de Archivos, entre las carpetas y la lista) hay separaciones con tres puntitos: **pon el mouse encima y arrastra** para cambiar el tamaño. **Doble clic** en una separación la vuelve a su tamaño normal.
- Los tamaños se guardan por PC en `datos\diseno.json`.

**Al aire (arriba)**
- **Monitor de programa** a la izquierda: la imagen del spot que está al aire, en el mismo segundo que CasparCG (sin sonido). Con la tanda detenida muestra *Señal de TV*. En una PC remota se ve la miniatura del spot.
- **Vúmetros** (L/R) junto al monitor: el audio real de la salida de CasparCG (picos en dBFS, verde / ámbar desde −18 / rojo desde −6, con marca del pico). Llegan por OSC; solo en la PC principal.
- Estado (**AL AIRE** / **EN PAUSA** / **SEÑAL DE TV**), *Spot 2 de 4 · tanda −0:27*, la **transición** elegida y la **cuenta regresiva** grande (destella los últimos 5 s del último spot).
- Nombre del spot, tiempo transcurrido / total, barra del spot y, debajo, **la tanda entera por tramos** (uno por spot).
- **SIGUE**: miniatura, nombre y duración del próximo (o *Vuelve la señal de TV*).
- Anterior, play/pausa, siguiente, detener, **bucle** (no / toda la tanda / solo el spot), **aleatorio** y volumen.
- El botón **VOLVER A TV** (y la tecla Esc) están **ocultos** por ahora; se activan en ⚙ Ajustes → Interfaz.
- Encadena los spots sin negros y, al terminar el último, vuelve la señal de TV sola.

**Transiciones** (botón ✨ en *Al aire*): **corte, fundido, barrido, empuje o deslizar**, de 0,25 a 1,5 s (o lo que se ponga en Ajustes). Se usan al pasar de un spot a otro, al entrar desde la señal de TV y al volver a ella (fundido a TV). Con fundido, CasparCG empieza el spot siguiente mientras termina el anterior, y Relevo lo tiene en cuenta. Por defecto: fundido de 0,5 s (`[transiciones]` en config.toml); la elegida en pantalla queda guardada en `datos\transicion.json`.
- **Tiempo exacto** leyendo el OSC de CasparCG (si no está configurado, lo calcula con la duración).

**Derecha:** **alertas** (contratos por vencer, videos que faltan), pauta del día y registro de lo que va pasando.

### Atajos de teclado

| Tecla | Acción |
|---|---|
| Espacio | Play / pausa (empieza por el spot seleccionado de la tanda) |
| Esc | Volver a TV (solo si el botón está activado en ⚙ Ajustes → Interfaz) |
| Ctrl + → / Ctrl + ← | Spot siguiente / anterior |
| Ctrl + Retroceso | Detener la tanda |
| Ctrl + ↑ / Ctrl + ↓ | Subir / bajar el spot seleccionado |
| F1 / F2 / F3 / F4 / F5 | Emisión / Biblioteca / Reportes / CasparCG / Publicidades |
| F11 | Vista compacta |
| Enter | Añadir a la tanda el video seleccionado en Archivos |
| Supr | Quitar de la tanda el spot seleccionado |

## ⚙ Ajustes

El botón **⚙ Ajustes** (arriba a la derecha) cambia la configuración sin abrir `config.toml`: CasparCG (IP, puertos, canal, capa, carpetas), **salida de video**, archivos y ffmpeg, pauta y corte, vista previa, Biblioteca, red y API, respaldos e interfaz (animaciones). Se guarda en `config.toml` sin perder sus comentarios. Lo que necesita reiniciar Relevo lo dice, con un botón **Reiniciar Relevo ahora** (no se puede mientras hay publicidad al aire).

### Salida de video de CasparCG (segunda pantalla)

Por defecto (`[casparcg] pantalla = "auto"`), al abrir Relevo:
- Si la PC tiene **dos o más monitores**, Relevo deja configurada la ventana de video de CasparCG (*screen consumer* del canal) **a pantalla completa en el segundo monitor**: `device` = ese monitor, `windowed=false`, `borderless=true`, `always-on-top=true`. Así ocupa el monitor exacto, sin cambiar su resolución.
- Con un solo monitor no toca nada.
- También se puede elegir un monitor concreto, una ventana normal o «no tocar» en **⚙ Ajustes → Salida de video**, donde se ven los monitores dibujados y cuál usa CasparCG.
- Antes del primer cambio se guarda la copia `casparcg.config.antes-de-relevo`. Si el canal no tenía salida de pantalla, se agrega. Lo demás del archivo no se toca.
- CasparCG lee su configuración al arrancar: si ya estaba abierto, abajo aparece **⟳ Reiniciar CasparCG para aplicar los cambios** (solo en señal de TV).

### Al cerrar Relevo

- Si hay publicidad al aire, se detiene y vuelve la señal de TV (`[casparcg] detener_al_cerrar`).
- **CasparCG y su scanner se cierran también, si los abrió Relevo** (`[casparcg] cerrar_al_salir = "si"`). Si ya estaban abiertos antes de Relevo, no se tocan. Con `"preguntar"` sale un aviso: *Cerrar todo* / *Dejar CasparCG abierto* / *Cancelar*; con `"no"` CasparCG sigue abierto. Se cambia en ⚙ Ajustes › CasparCG.
- Cuando Relevo se reinicia solo (para aplicar ajustes) o se actualiza, CasparCG **no** se cierra: el Relevo nuevo se conecta y lo sigue considerando suyo.

## Tamaño de la interfaz

La interfaz está pensada para una ventana de unos 1480 × 860. En pantallas más chicas (1366 × 768) o con la **escala de Windows al 125 %** (1080 se vuelve 1536 × 864), Relevo **achica todo lo justo para que quepa** (hasta un 70 %) y se abre maximizado. En ⚙ Ajustes › Interfaz › *Tamaño de la interfaz* se puede fijar a mano (70 % a 130 %).

## Aspecto y rendimiento

- **Letra Inter** (normal y seminegrita) incluida dentro del programa, así se ve igual en todas las PCs (licencia libre OFL, en `crates/app/recursos`). Los números de tiempo siguen en letra de ancho fijo.
- Cabecera con la marca, pestañas de texto (la activa subrayada), fecha y hora; barras de desplazamiento finas; ícono propio en la ventana y la barra de tareas.
- **La ventana solo se redibuja cuando hace falta:** con todo quieto, una vez por segundo (el reloj); al aire, unas 20 veces por segundo. El núcleo avisa a la pantalla solo cuando algo cambió de verdad y, aunque CasparCG mande el tiempo 60 veces por segundo, publica como mucho 20.
- Lo que antes se consultaba al disco en cada cuadro (dónde está un clip, si existe una carpeta anclada) ahora se recuerda unos segundos.
- `cargo build --release` compila con optimización completa (LTO, un solo bloque de código, sin símbolos). Con `cargo run` a secas, las bibliotecas también van optimizadas para que la ventana se mueva fluida.

Todos los atajos **se cambian** en ⚙ Ajustes → Atajos de teclado (botón *Cambiar* y se pulsa la tecla, sola o con Ctrl, Mayús o Alt). Quedan en `[atajos]` de config.toml.

## Avisos visuales (sin sonido)

La PC principal va conectada a la mezcladora, así que Relevo **no emite ningún sonido de aviso**. Los avisos son un marco que destella alrededor de toda la ventana y un cartel arriba (no mueven nada de sitio):
- **Vuelve la señal de TV en N** (ámbar): los últimos segundos de la tanda. Por defecto 10 s; se cambia o se apaga en ⚙ Ajustes → Interfaz (`aviso_fin_seg`).
- **Sin conexión con CasparCG** (rojo): si se pierde la conexión con publicidad al aire.
- **Corte detectado** (azul): cuando la detección ve que el canal fue a comerciales y la tanda no salió sola.

## Notificaciones

Los avisos salen **abajo a la derecha**, pequeños, y se van solos: 2,5 s los normales y 5 s los errores (con el mouse encima no se van). Los avisos importantes de la emisión (corte detectado, vuelve la señal de TV, sin conexión…) siguen siendo el marco y el cartel de arriba, sin sonido.

## Detección del corte del canal

Relevo puede mirar la señal del canal y avisar (o lanzar la tanda) cuando el canal va a comerciales. Vigila **el logo del canal**: mientras hay programa está en su esquina; en los comerciales desaparece.

Todo se hace en su propia ventana: **✂** (barra de abajo) → **Configurar…**. A la izquierda está la señal en grande y el gráfico; a la derecha, los pasos.

1. **Señal del canal** — de dónde viene:
   - **Capturadora:** se elige de la lista (Relevo busca las que hay; ⟳ vuelve a buscar).
   - **Página web:** se pega la dirección de la página con la señal en vivo y se pulsa *Conectar*. Relevo busca el video dentro de la página (y dentro de sus marcos). También vale la dirección directa del video (m3u8, rtmp…). Si `yt-dlp` está instalado en la PC, lo usa para los sitios conocidos (YouTube, Facebook…).
   - **IPTV:** mira el mismo canal que reproduce CasparCG por IPTV (ver «Señal de TV por IPTV»).
   - **Pantalla:** se elige el **monitor** (o todo el escritorio). Sirve cuando la página no deja sacar el video: se abre a pantalla completa en ese monitor y se deja ahí.
   - **Archivo:** un video grabado del canal, para probar.
2. **Logo del canal** — sobre la imagen, **se arrastra con el mouse** un recuadro alrededor del logo. Con el programa al aire (no en comerciales) se pulsa **Aprender el logo ahora** (8 segundos). Al lado queda la imagen de cómo lo ve Relevo. Si se mueve la zona hay que aprenderlo otra vez.
3. **Al detectar el corte** — **solo avisar** (cartel azul; se inicia con el atajo de play) o **iniciar la tanda sola**; segundos sin logo para dar el corte por empezado (2 s) y espera mínima entre cortes (3 min).

El **gráfico** muestra el parecido con el logo de los últimos 90 segundos: verde con programa, rojo en el corte. La línea blanca es el mínimo; se sube o baja con el mouse, o con **Ajustar solo** (la pone en medio cuando ya se vio programa y comerciales).

Abajo está el interruptor **Detección ENCENDIDA / APAGADA**. Con la ventana abierta la detección funciona aunque esté apagada, **en modo prueba**: se ve todo y se anota en «Últimos avisos», pero no avisa ni inicia la tanda. Si se cambia de señal estando encendida, se apaga sola (para no dar un corte falso) y hay que encenderla otra vez. Los cambios se guardan solos en `config.toml`.

En la barra de abajo, **✂** muestra el estado (*Programa*, *CORTE*, *sin señal*…) y con un clic se enciende o apaga. Solo funciona en la PC principal y necesita ffmpeg. El logo aprendido se guarda en `datos\logo_corte.json`.

Límites: si el canal quita el logo dentro del programa (o lo deja en los comerciales), se equivoca; conviene usarlo un tiempo en «solo avisar» antes de pasarlo a «iniciar».

## Modos: Publicidad y Transmisión

Arriba, junto a la transición, se elige el modo (no se puede cambiar con publicidad al aire):

- **🎞 Publicidad** (el de siempre): CasparCG solo pone la publicidad; el canal va por la mezcladora. La detección de corte puede avisar (o iniciar la tanda).
- **📺 Transmisión** (como OBS): CasparCG muestra el canal por IPTV y la publicidad entra encima. Se pasa a la publicidad a mano (Play) o sola, con la detección de corte. Solo se puede elegir si está permitido en **⚙ Ajustes → Señal por IPTV**.

### Monitores

**Modo Publicidad: un solo monitor** a la izquierda. Con la señal de TV muestra el canal (**📺 CANAL**); con publicidad, el spot (**⏺ AL AIRE**). Con **⚙** en el monitor se elige qué señal del canal se ve.

**Modo Transmisión: dos monitores**, como un mezclador de video: **PROGRAMA** a la izquierda, los tiempos y botones en medio, y **PREVIEW** a la derecha. Ninguno suena.

| Al aire | PROGRAMA | PREVIEW |
|---|---|---|
| Publicidad | el spot al aire | **el canal** (la señal del canal) |
| Señal de TV | **el canal** (por IPTV) | **el spot que sale al dar Play** (el primero, el seleccionado o donde se detuvo) |

La **señal del canal** se elige con **⚙**: capturadora, página web o video, canal IPTV, pantalla (un monitor) o archivo; o «Lo mismo que la detección de corte». En modo Transmisión, si no se elige ninguna, se usa el canal IPTV.

**Tamaño:** la separación de debajo se arrastra para agrandar o achicar la sección entera: el video, los tiempos y los botones crecen o se achican juntos (doble clic: tamaño normal). Si en el ancho no caben más grandes, los botones se quedan en el tamaño que cabe.

Si varios miran la misma señal (monitores y detección), se abre **un solo ffmpeg** para todos (una capturadora no se puede abrir dos veces a la vez). La detección es independiente: puede mirar otra señal.

### Línea de tiempo (ir a otro punto)

Debajo del nombre del spot al aire:

- **Línea del spot**: el avance, marcas cada segundo (más largas cada 5) y, en azul al final, la **zona donde entra el siguiente spot** con la transición. Al pasar el mouse dice en qué segundo caerías y cuánto faltaría. **Clic o arrastrar**: ir a ese punto (también en pausa: se ve el cuadro de ahí).
- **Línea de la tanda**: un tramo por spot (con su número), el actual con su avance y, si se detuvo a mitad, el de «quedó aquí» en ámbar. **Clic en un tramo**: ese spot desde ese punto.
- **−5 s / +5 s** y **⇥ 3 s** (ir a los últimos 3 segundos), para revisar cómo pasa de un spot al siguiente o si un video tiene un error.

Solo con la tanda al aire o en pausa (con la señal de TV no hace nada) y con el mando. Usa `CALL … SEEK` y `PLAY … SEEK` de CasparCG; el paso al siguiente spot sigue siendo automático.

### Detener a mitad de la tanda

**⏹ con publicidad al aire** vuelve a la señal de TV, pero Relevo recuerda dónde quedó: por ejemplo, en el spot 4 de 7.

- En la tanda, ese spot se marca **QUEDÓ AQUÍ** (ámbar) y el que viene, **SIGUE**. Arriba se ve «Detenida en el spot 4 de 7 · faltan m:ss» y el PREVIEW muestra ese spot.
- **⏵ Play vuelve a empezar ese spot** desde el principio y sigue con el 5, 6 y 7 (no repite el bloque).
- Con la tanda detenida, **⏮ ⏭** cambian desde qué spot seguirá.
- **⏹ otra vez** (o dos veces seguidas) **reinicia la tanda**: el próximo Play empieza desde el primer spot. ⏹ también quita la selección de la tanda, para que Play no empiece desde un spot seleccionado antes.
- Si la tanda termina sola, o se vacía o se abre otra, no queda nada pendiente.

## Señal de TV por IPTV (modo Transmisión)

En modo Transmisión, CasparCG reproduce el canal desde la red (IPTV). Se configura en **⚙ Ajustes → Señal por IPTV**, y se aplica al guardar, sin reiniciar.

- **Dirección del canal:** la que funciona en VLC (`http://…/canal.m3u8`, `udp://@239.x.x.x:1234`, `rtp://…`). También se puede cargar una **lista `.m3u`** (dirección o archivo) y elegir el canal por su nombre.
- **Capa:** el canal suena en una capa **debajo** de la publicidad (por defecto 10; la publicidad va en la 20). Al entrar la tanda, la publicidad lo tapa y **el sonido del canal se baja a cero** con el mismo fundido de la transición; al terminar, vuelve solo.
- **Vigilancia:** Relevo mira por OSC que el canal siga llegando. Si pasan 6 s sin señal, lo vuelve a pedir (y después espera 10, 20, 40, 60 s entre intentos) y muestra un **aviso rojo** «SE CORTÓ LA SEÑAL IPTV» (sin sonido). En la barra de abajo, **📺 IPTV** muestra el estado (*en vivo*, *conectando*, *CORTADA*); con un clic: *Reconectar ahora* o *Configurar…*.
- **Retraso:** el canal por IPTV llega con retraso (unos 20 s con HLS). Es parejo para todo; para la detección de corte conviene la fuente **📺 IPTV**, que mira la misma dirección y por eso va al mismo tiempo.
- **Cuando vuelve el logo** con publicidad al aire, la detección puede **avisar** (cartel verde «VOLVIÓ EL PROGRAMA DEL CANAL») o, en modo Transmisión, **volver al canal** cortando la tanda (en la ventana de detección, «Cuando vuelve el logo»).
- Si CasparCG se reinicia, Relevo vuelve a pedir el canal solo. Sin OSC, el estado queda en «enviado (sin OSC)»: no se puede comprobar si llega.

Antes de usarlo al aire conviene dejarlo varias horas en prueba con el canal real.

## CasparCG (pestaña CasparCG, F4)

Solo en la PC donde está CasparCG.

- **Estado** de CasparCG Server y del media scanner: abierto, cerrado o **cuántas copias** hay abiertas (cada copia de CasparCG abre su propia salida de video; si hay más de una, se avisa y con **⟳ Reiniciar** queda una sola). Se revisa cada 3 s mientras la pestaña está a la vista.
- **Abrir / Reiniciar / Cerrar.** Reiniciar y Cerrar cortan la salida, así que solo se pueden en señal de TV y piden confirmación. Si CasparCG ya responde por AMCP, no se ofrece abrir otra copia.
- **Consolas dentro de Relevo**: la de CasparCG (su registro más reciente, de la carpeta `<log-path>` de `casparcg.config`) y la del scanner. Colores por nivel, búsqueda, filtro de nivel, «Ocultar consultas de Relevo» (VERSION, INFO, CLS… que Relevo pregunta cada poco) y «Seguir al final».
- **Consolas ocultas** (⚙ Ajustes › CasparCG › Consolas, o `consolas = "ocultas"`): Relevo abre CasparCG y el scanner **sin sus ventanas negras**; lo que escriben va a `datos/consolas/` y se ve en esta pestaña. CasparCG recibe una entrada que nunca se cierra, así que no gasta procesador esperando teclas y sigue funcionando aunque Relevo se cierre. Vale desde la próxima vez que se abran.

**Detener al cerrar Relevo** (⚙ Ajustes › CasparCG, activado): si cierras Relevo con una publicidad al aire, se detiene y vuelve la señal de TV. Desmárcalo si quieres que CasparCG siga reproduciendo aunque Relevo esté cerrado.

**Salida siempre visible** (⚙ Ajustes › CasparCG, activado): si la ventana de video de CasparCG se minimiza —por ejemplo con **Windows + D** («Mostrar escritorio»)—, CasparCG la deja en negro aunque siga sonando. Relevo la vuelve a mostrar sola en menos de un segundo, sin quitarle el foco a lo que estés usando (las consolas no se tocan). En la pestaña CasparCG también está el botón **🖵 Mostrar salida**.

Al arrancar, Relevo primero mira si CasparCG y el scanner ya están abiertos y solo abre lo que falta (antes, con nombres de scanner largos como `casparcg-scanner-v1.4.0-win32-x64.exe`, no lo veía y lo abría otra vez). Tampoco añade una segunda salida de pantalla si `casparcg.config` ya tiene `<screen />`.

## Explorador de archivos

- **⬅ ➡ ⬆**: atrás, adelante y subir una carpeta. Al subir o volver, la carpeta de la que vienes queda marcada en la lista.
- **Botones de atrás y adelante del mouse** (los laterales), con el puntero sobre Archivos. En la lista de CasparCG, atrás sube una carpeta.
- **Teclado**: Alt+← / Alt+→ (atrás / adelante), Alt+↑ o Retroceso (subir), Ctrl+L (escribir la ruta).
- **Ruta en migas de pan**: clic en una carpeta de la ruta para ir ahí; si no cabe, las primeras quedan en «…». Con ✏ o doble clic en la ruta se escribe a mano (Enter: ir; Esc: cancelar).
- **Orden**: por nombre, más recientes, más largos o más pesados (las carpetas siempre primero). Se recuerda.
- Arriba de la lista, cuántas carpetas y videos hay y cuánto duran en total. Esc en el buscador lo borra.
- Al abrir Relevo vuelve a la carpeta donde se quedó la última vez (`datos/ultima_carpeta.json`). La primera vez abre la carpeta de medios de CasparCG.

## Miniaturas y vista previa

- **Miniaturas reales:** Relevo saca con ffmpeg un cuadro de cada video (de cualquier carpeta, no solo los de CasparCG) y lo muestra en la lista de archivos, la tanda, «Sigue» y la Biblioteca. Se hacen en segundo plano y se guardan en `datos\miniaturas`, así que solo tardan la primera vez. Si no hay ffmpeg se usan las de CasparCG y, si tampoco, un color.
- **Vista previa:** clic derecho en un video (de Archivos, la tanda o la lista de CasparCG) → *Vista previa*. Se abre en una ventanita dentro de la app que se puede mover y ensanchar.
- El bloque «Al aire» mide siempre lo mismo: al dar Play, Archivos, Tanda y Pauta no se mueven.

## Ventanas y animaciones

- Las ventanas emergentes (guardar tanda, confirmaciones, Ajustes) oscurecen lo de atrás, tienen un ícono según lo que piden (rojo si borra algo), se cierran con ✖, Esc o clic fuera, y aparecen con una animación corta.
- Los avisos entran por la derecha, muestran cuánto les queda y no se van mientras el mouse está encima.
- Transiciones: fundido al cambiar de pantalla o a la vista compacta; el borde de «Al aire» cambia de color suavemente y respira mientras hay publicidad al aire; un spot que llega a la tanda se resalta un momento.
- Se apagan en **⚙ Ajustes → Interfaz** (`[interfaz] animaciones = false`), para PCs lentas.

## Duración del corte y relleno (modo AUTO)

En la tanda, con **AUTO**, se elige la **duración del corte** (sin tope, 0:30 … 5:00, y ajuste fino de 5 en 5 s):
- Si los pases en ventana no entran, van **primero los prioritarios**; los que no entran quedan para el **siguiente corte** (se avisa cuáles).
- **Rellenar:** el tiempo que sobra se completa **primero con spots de otros clientes** (los de pautas activas hoy, el que menos salió hoy primero, cada uno una vez por tanda; se quita en ⚙ Ajustes › Pauta) y después con los clips de las carpetas de relleno (por defecto `IDS` y `PROMOS`), el más largo que entre y sin repetir uno seguido. El relleno no cuenta como pase del cliente.
- **Repetir:** si la tanda (con el relleno) termina y el corte sigue, vuelve a empezar hasta que vuelvas a TV. Las repeticiones no cuentan como pases ni se recuperan si se cortan.
- La elección se recuerda (`datos\corte.json`).

## Vista compacta

Botón **⛶ Compacta** (arriba a la derecha): solo la imagen al aire, la cuenta regresiva grande y cuatro botones grandes (**Iniciar tanda / Pausa**, **Siguiente**, **AUTO/MANUAL**, **VOLVER A TV**) más la tanda. Pensada para operar desde otra sala o con pantalla táctil. **⛶ Vista completa** vuelve.

## Respaldo automático

Una vez al día (al abrir Relevo y luego cada hora se comprueba) se guarda `respaldos\respaldo-AAAA-MM-DD.zip` con pautas, usuarios, registro de emisión, revisiones de la biblioteca y `config.toml` (los videos no). Se guardan los últimos 30.
- En la barra de abajo: **💾 Respaldo: dd/mm**. Clic: *Respaldar ahora* o *Abrir la carpeta de respaldos*.
- Conviene que la carpeta esté en **otro disco o una USB**: `[respaldo] carpeta = 'D:\Respaldos Relevo'`.
- **Restaurar:** cierra Relevo y descomprime el respaldo en la carpeta de Relevo (reemplaza `datos` y `config.toml`).

## API para botoneras (Stream Deck…)

Con `[api] activa = true` y una `clave` en `config.toml`, la PC principal acepta órdenes por la red en el puerto 7801:

| Dirección | Qué hace |
|---|---|
| `/api/iniciar-corte` | Pone AUTO y lanza la tanda del corte |
| `/api/play` · `/api/pausa` · `/api/play-pausa` | Reproducir / pausar |
| `/api/siguiente` · `/api/anterior` | Cambiar de spot |
| `/api/volver-a-tv` | Volver a TV (siempre) |
| `/api/auto` · `/api/manual` | Cambiar de modo |
| `/api/estado` | Qué está al aire, cuánto falta, qué sigue (JSON) |

Cada una lleva la clave: `http://192.168.1.50:7801/api/volver-a-tv?clave=TUCLAVE` (o la cabecera `X-Clave`). Abriendo `http://IP:7801/api` se ve la lista.

**Stream Deck:** acción *Sistema → Sitio web*, pega la dirección y marca *GET request in background* (así no abre el navegador). Un botón por acción.

Abre el puerto 7801 en el firewall si la botonera está en otra PC (el instalador solo abre el 7800).

## Biblioteca (pestaña 🎬 Biblioteca)

Todos los videos de CasparCG en una tabla, revisados uno por uno con ffmpeg: **formato, cuadros/s, audio y volumen (LUFS)**. Cada spot queda **✔ Listo** o con lo que hay que corregir (*muy fuerte*, *muy bajo*, *sin audio*, *otra resolución*, *otros cuadros/s*). El número rojo en la pestaña son los spots con problemas graves.

- Filtros: *Todos*, *Para revisar*, *En pauta*, *Sin usar*, y buscador.
- Al elegir un spot: miniatura grande (clic = vista previa con sonido), sus datos, en qué pautas está y las acciones:
  - **🔊 Nivelar volumen:** lo lleva al volumen del canal (-23 LUFS por defecto). La imagen no se toca (es rápido).
  - **🎞 Convertir al formato del canal:** resolución y cuadros/s del canal, H.264 + AAC, y volumen nivelado.
  - **Reemplazar con otro video…** (o arrastrar el nuevo video a la ventana): el spot **conserva su nombre, sus pautas y su lugar en la tanda**.
  - **A la papelera.** Avisa si alguna pauta lo usa.
- Los trabajos se hacen de a uno, en segundo plano y con prioridad baja. **Si el spot está en la tanda, el archivo se cambia cuando salga de ella** (nunca mientras se emite).
- **Papelera:** al nivelar, convertir, reemplazar o borrar, la versión anterior va a la papelera (`datos\papelera`) y se puede **restaurar**. Se borra sola a los 30 días.
- Arriba se ve el **espacio libre** del disco de los videos; avisa si queda poco (menos de 5 GB).
- Desde una PC remota se puede hacer todo esto con un usuario **Editor** o **Admin** (el reemplazo sube el video a la principal).

## Reportes (pestaña 📊 Reportes)

Cada spot que sale al aire queda anotado en `datos\registro\AAAA-MM-DD.jsonl`: hora, cliente, spot, cuánto estuvo al aire (**completo** o **cortado** y en qué segundo), modo AUTO/MANUAL, quién tenía el mando y qué pase de la pauta cumplió.

- **🔮 Simular un día:** qué saldría en cada corte de hoy, mañana o cualquier fecha, con la pauta, la duración del corte y el relleno actuales; qué pases no entrarían en su ventana y qué pautas no tienen video.
- Periodos: *Hoy, Ayer, Esta semana, Este mes, Mes pasado* o fechas a mano (día/mes/año).
- **Salidas al aire:** la lista completa, lo más reciente arriba.
- **Por cliente:** salidas, tiempo al aire, cortadas y **cumplimiento de la pauta** (pases que salieron completos de los que tocaban). Clic en un cliente para ver solo lo suyo.
- **⬇ Excel:** hoja *Resumen* y hoja *Detalle*. **⬇ PDF:** *Constancia de emisión* del cliente elegido (o de todos), lista para enviar.
- **Alertas:** contratos (pautas con fecha de fin) que vencen en 7 días o vencieron hace poco, y pautas cuyo video no está en CasparCG. También aparecen arriba de la *Pauta de hoy*.
- El cliente de cada salida es el nombre de su pauta; si salió a mano y no tiene pauta, el nombre de su carpeta (`CLIENTES\FERRETERIA X\spot.mp4` → *FERRETERIA X*).
- Desde una PC remota también se ven y exportan (el reporte se arma en la principal).

## Modo remoto (otras PC de la red)

La PC principal (la que tiene CasparCG) abre un servidor en el **puerto 7800**. Desde otra PC se ejecuta la misma app con `modo = "remoto"` y se ve y maneja todo igual: tanda, transporte, pauta, explorador de CasparCG y miniaturas.

- **Usuarios y roles** (botón **👥 Usuarios** en la PC principal):
  - *Operador*: maneja la tanda y el aire.
  - *Editor*: además cambia la pauta.
  - *Admin*: además gestiona usuarios y puede quitar el mando.
- Usuario inicial **admin / admin**. **Cámbialo** (el botón muestra ⚠ mientras siga así).
- **Mando:** solo una PC opera a la vez. El primero que toca algo lo toma; los demás ven *Bloqueado* y pueden **pedirlo**. Un Admin puede **quitarlo**. **VOLVER A TV (Esc) funciona siempre, desde cualquier PC.**
- **Subir videos:** en la PC remota se arrastran o se añaden videos propios; se suben a `media\IMPORTADOS` de la PC principal y entran a la tanda. **Si se corta la red, la subida continúa desde donde quedó** al reconectar (lo recibido se guarda 2 días en `datos\subidas`).
- Si se corta la red, la remota se reconecta sola. La principal sigue emitiendo sin ella.

Configuración en la PC remota (`config.toml`):

```toml
[remoto]
modo = "remoto"
servidor = "192.168.1.50"   # IP de la PC principal
usuario = "admin"
```

En la PC principal hay que **permitir el puerto 7800 en el Firewall de Windows** (la primera vez Windows lo pregunta; marca *Redes privadas*), o en PowerShell como administrador:

```powershell
New-NetFirewallRule -DisplayName "Relevo remoto" -Direction Inbound -Protocol TCP -LocalPort 7800 -Action Allow -Profile Private
```

## Instalador (con ffmpeg incluido)

En la carpeta Relevo, en PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File herramientas\empaquetar.ps1
```

Compila Relevo, descarga ffmpeg (solo la primera vez) y deja en `instalador\salida`:
- **`Relevo-<versión>-Instalador.exe`** (por ejemplo `Relevo-1.0.0-Instalador.exe`) si tienes Inno Setup (`winget install JRSoftware.InnoSetup`), o
- **`Relevo.zip`** si no lo tienes (se descomprime y se usa igual).

El instalador no pide permisos de administrador: instala en `%LOCALAPPDATA%\Programs\Relevo`, crea los accesos directos (menú Inicio y Escritorio, con el ícono de Relevo, que va dentro de `relevo.exe`) y, si marcas la opción, abre el puerto 7800 del firewall para el modo remoto. Al actualizar **no pisa tu `config.toml` ni tus datos**. Con el instalador ya no hace falta instalar ffmpeg aparte.

## Actualizaciones

Relevo revisa en GitHub (`Everlck/relevo`, Releases) al abrirse y cada 6 horas si hay una versión nueva. Si la hay, la barra de abajo dice **⬆ Relevo X.Y.Z disponible** y aparece un aviso.

- Con un clic se ven las novedades y el botón **Actualizar ahora**: baja el instalador, cierra Relevo, lo instala en silencio y **lo vuelve a abrir solo**.
- **Nunca con publicidad al aire:** si se pide durante una tanda, se instala cuando termina.
- Se conservan `config.toml`, las pautas, las tandas y el registro.
- Nunca se actualiza sin que alguien lo pida. Se apaga con `[actualizacion] activa = false`.
- Si el repositorio fuera privado, cada PC necesita una clave de GitHub de solo lectura en `[actualizacion] token`.

## Publicar una versión nueva

1. Sube el número en `Cargo.toml` (`version = "1.0.1"`) y haz commit y push.
2. Crea el tag de esa versión y súbelo. En GitHub Desktop: *History* → clic derecho en el commit → **Create Tag…** → `v1.0.1` → *Push origin*. O en github.com: *Releases → Draft a new release → Choose a tag → v1.0.1 → Create new tag* (y cierra ese borrador sin publicarlo).
3. GitHub Actions (*Publicar versión*) corre las pruebas en Windows, arma `Relevo-1.0.1-Instalador.exe` con ffmpeg y crea la Release con el instalador. Tarda unos 15 a 25 minutos; se sigue en la pestaña **Actions**.
4. Las PCs con Relevo lo ven solas.

El tag tiene que coincidir con `Cargo.toml` (`v1.0.1` ↔ `1.0.1`); si no, Actions se detiene y lo dice. Desde **Actions → Publicar versión → Run workflow** se puede armar el instalador sin publicar nada (queda para descargar en esa ejecución).

## Requisitos (PC principal, Windows)

1. **Rust:** <https://rustup.rs>.
2. **CasparCG Server 2.3 LTS:** <https://github.com/CasparCG/server/releases>, descomprimido por ejemplo en `C:\CasparCG Server`. La app lo encuentra y lo abre sola junto con **scanner.exe** (el que da la lista de videos, duraciones y miniaturas).
3. **ffmpeg** (para la vista previa, el monitor y la Biblioteca): **no hace falta instalarlo**.
   - Viene con el instalador.
   - Si Relevo no lo encuentra (por ejemplo, compilando el proyecto), **lo descarga solo** la primera vez (~35 MB comprimido en `.7z`, versión oficial *essentials* de gyan.dev bajada de su copia en GitHub, que es mucho más rápida; si GitHub no responde, de gyan.dev). Mientras baja, la barra de abajo muestra los MB, la velocidad y cuánto falta. Queda en `%LOCALAPPDATA%\Relevo\ffmpeg` y lo usan todas las copias de Relevo de la PC.
   - **Actualización:** una vez al día revisa si hay una versión nueva. En la barra de abajo, **🎞 ffmpeg** muestra la versión; si hay una nueva dice *hay versión X* y se actualiza con un clic (*Actualizar*). También *Buscar actualización*.
   - Lo busca en: el descargado por Relevo, junto al programa, `instalador\ffmpeg` del proyecto, winget (aunque la terminal no se haya reiniciado), `C:\ffmpeg\bin`, Chocolatey, Scoop y el PATH. O se indica en `[archivos] ffmpeg`.

## Cómo ejecutarla

```powershell
# En la carpeta Relevo (la app abre CasparCG y su scanner si hace falta):
cargo run --release
```

El programa queda en `target\release\relevo.exe`; se puede copiar junto con `config.toml` (y `ffmpeg.exe`) a otra carpeta y abrirlo con doble clic. La versión final no abre la ventana negra de consola: lo que antes salía ahí queda en `datos\relevo.log`.

## Configuración (`config.toml`)

Cada PC tiene su propio `config.toml` (con sus direcciones y claves), que **no se sube a GitHub**. En el repositorio va `config.ejemplo.toml`: si Relevo no encuentra `config.toml`, lo crea a partir del ejemplo la primera vez que se abre. Casi todo se cambia desde **⚙ Ajustes** sin tocar el archivo.

| Opción | Para qué sirve | Por defecto |
|---|---|---|
| `host` | IP de la PC donde corre CasparCG | `127.0.0.1` |
| `puerto` | Puerto AMCP de CasparCG | `5250` |
| `canal` / `capa_publicidad` | Dónde sale la publicidad | `1` / `20` |
| `carpeta_medios` | Carpeta `media` de CasparCG. Vacía = se la pregunta a CasparCG | vacía |
| `osc_puerto` | Puerto donde llega el tiempo exacto (OSC). `0` = no usar | `6250` |
| `carpeta_programa` | Carpeta de `casparcg.exe`. Vacía = buscarla sola | vacía |
| `iniciar_automaticamente` | Abrir CasparCG y scanner, y configurar el OSC al iniciar | `true` |
| `[pauta] orden` | Spots normales en AUTO: `"hora"` (el que vence antes) o `"cercania"` (el más cercano a la hora actual) | `"hora"` |
| `[pauta] tolerancia_por_defecto` | Margen (antes y después) de una publicidad nueva, en minutos; luego se cambia en cada una | `15` |
| `[interfaz] hora_12` | Mostrar las horas como `8:30 pm` en vez de `20:30` | `false` |
| `[pauta] duracion_corte` | Duración del corte en AUTO, `"m:ss"` (`"0:00"` = sin tope) | `"0:00"` |
| `[pauta] relleno` | Carpetas (o clips) de CasparCG para rellenar el corte | `["IDS", "PROMOS"]` |
| `[respaldo] carpeta` / `guardar` | Dónde van los respaldos (vacía = `respaldos`) y cuántos se guardan | vacía / `30` |
| `[api] activa` / `puerto` / `clave` | API para botoneras | `false` / `7801` / vacía |
| `[archivos] externos` | Videos fuera de CasparCG: `"directo"` (usarlos desde su carpeta) o `"copiar"` (copiarlos a IMPORTADOS) | `"directo"` |
| `[archivos] ffmpeg` | Ruta de `ffmpeg.exe`. Vacía = buscarlo (y si no hay, descargarlo) | vacía |
| `[archivos] actualizar_ffmpeg` | Revisar una vez al día si hay versión nueva de ffmpeg | `true` |
| `[vista_previa] sonido` / `salida_audio` | Con sonido al abrir, y por qué salida (vacía = la de Windows; también se elige con ⚙) | `true` / vacía |
| `[vista_previa] silenciar_al_aire` | En la principal, callar la vista previa mientras hay publicidad al aire | `true` |
| `[biblioteca] resolucion` / `fps` | Formato del canal | `"1920x1080"` / `29.97` |
| `[biblioteca] lufs` / `tolerancia` | Volumen objetivo y diferencia aceptada (dB) | `-23.0` / `2.0` |
| `[remoto] modo` | `"principal"` (la PC con CasparCG) o `"remoto"` | `"principal"` |
| `[remoto] servir` / `puerto` | Aceptar PC remotas y en qué puerto | `true` / `7800` |
| `[remoto] servidor` / `usuario` | En una remota: IP de la principal y usuario recordado | vacío |

### Tiempo exacto (OSC)

La app lo configura sola en `casparcg.config`. Si prefieres hacerlo a mano (o CasparCG está en otra PC), agrega dentro de `<configuration>`:

```xml
<osc>
  <predefined-clients>
    <predefined-client>
      <address>127.0.0.1</address>
      <port>6250</port>
    </predefined-client>
  </predefined-clients>
</osc>
```

En la barra inferior de la app aparece **⏱ Tiempo exacto (OSC)** en verde mientras sale un spot.

**Lista de videos al abrir:** CasparCG pide la lista de videos (CLS) a su *media scanner* (`scanner.exe`), que tarda unos segundos en arrancar; mientras tanto responde `501 CLS FAILED`. Relevo reintenta solo cada pocos segundos y, mientras, lee los videos directamente de la carpeta de medios. Solo avisa si el scanner sigue sin responder después de ~30 s (entonces revisa que el scanner esté abierto). Relevo abre el scanner solo al iniciar: en CasparCG 2.3 es `scanner.exe` y desde 2.4 viene como `casparcg-scanner-vX.Y.Z-win32-x64.exe`; Relevo reconoce los dos.

Con OSC, CasparCG informa además **qué archivo** está al aire. Así el paso de un spot al siguiente lo decide CasparCG y no el reloj de Relevo: el que sigue se prepara recién cuando CasparCG ya entró al nuevo, aunque los spots duren lo mismo o el mismo spot vaya dos veces seguidas. Sin OSC (barra en *Tiempo estimado*), Relevo usa su reloj y deja un margen de un poco más de un segundo antes de preparar el siguiente, para no pisar al que CasparCG tiene en espera. Por eso conviene tener el OSC activo (reinicia CasparCG una vez si la barra dice *Tiempo estimado*).

> La copia automática de videos funciona cuando CasparCG corre en **la misma PC** que la app (o si su carpeta `media` está compartida en la red y se indica en `carpeta_medios`).

## Probar sin CasparCG

```powershell
python herramientas/casparcg_simulado.py --medios "C:\ruta\a\una\carpeta\con\videos"
# en otra consola:
cargo run
```

El simulador hace de CasparCG con los videos de esa carpeta: responde a los comandos, genera miniaturas, envía por OSC la ruta y el tiempo del clip (25 veces por segundo, como CasparCG 2.3), encadena los precargados con AUTO y muestra cada comando que recibe.

Para probar situaciones del CasparCG real: `--retardo 0.6` (segundos que tarda un PLAY en empezar), `--atraso-osc 0.15` (el tiempo llega con atraso), `--extra 0.3` (el clip dura más que lo que dice su archivo) y `--sin-ruta` (sin la ruta por OSC, como CasparCG 2.0) y `--cls-falla 8` (CLS responde 501 los primeros segundos, como cuando el media scanner aún está arrancando).

## Pruebas

```powershell
cargo test
```

177 pruebas: protocolo AMCP, lógica de la tanda (incluido detener a mitad y retomar ese spot, y el paso de un spot al siguiente: lo decide CasparCG cuando informa qué archivo está al aire; si no, el siguiente se precarga con margen para no pisar al que espera), OSC (ruta y tiempo), copia de archivos, duración de MP4/MOV, explorador, tandas guardadas, **arrastrar y soltar** (dónde cae un video, mover un spot, anchos de las columnas), **pauta** (ventanas, prioridad, orden, días, emitidos; incluido el ejemplo de 10:15/10:30) **arranque** (buscar CasparCG, añadir el OSC sin romper `casparcg.config`, monitores y salida a pantalla completa en el segundo monitor) **ajustes** (guardar en `config.toml` sin perder comentarios) **modo remoto** (clave incorrecta, mando, roles, subida de un video de 3 MB, subida cortada que continúa, reemplazo que espera a que el spot salga de la tanda) **biblioteca** (lectura de ffmpeg, problemas frente al canal, papelera, y nivelar un video real con ffmpeg) **duración del corte y relleno**, **simulador del día**, **respaldo**, **API** (clave, acciones, iniciar corte), **ffmpeg** (versiones, dónde buscarlo, descarga con dirección de respaldo y reemplazo, sacar ffmpeg de un `.7z`) y **reportes** (cliente de cada salida, cumplimiento con pases cortados, alertas de contratos, periodos, exportar a Excel y PDF, reporte pedido desde una PC remota) y **detección de corte** (parecido del logo, encontrar el video dentro de una página, captura de un monitor) e **IPTV** (lista `.m3u`, vigilancia y reintentos del canal, señal de vida por OSC) y **señales** (fuentes del preview, un solo lector compartido, reducción a grises para la detección) y **actualizaciones** (leer la Release de GitHub, sin instalador o borrador no cuenta) y **CasparCG** (copias abiertas con nombres largos, salida sin consola, niveles de su registro, ocultar las consultas de Relevo, leer solo lo nuevo) y **explorador** (orden, migas de pan, atrás/adelante/subir) y **saltos dentro del spot** (adelantar, retroceder, en pausa, otro spot desde un punto, sin confundirse con el tiempo viejo de CasparCG) y **pauta** (horas con AM/PM y listas pegadas, margen antes/después distinto, plantillas de horario, pendientes, tanda nueva antes o después de lo pendiente, tanda detenida, relleno con otros clientes).

## Estructura

```
crates/amcp/      cliente de CasparCG (protocolo AMCP, reconexión)
crates/nucleo/    tanda, controlador, OSC, archivos e importación
crates/nucleo/src/pauta.rs  motor de pauta (pases, ventanas, tanda sugerida)
crates/nucleo/src/remoto.rs servidor y cliente del modo remoto (WebSocket, puerto 7800)
crates/nucleo/src/usuarios.rs usuarios y roles (claves con argon2, en datos/usuarios.json)
crates/nucleo/src/biblioteca.rs revisión con ffmpeg, nivelar, convertir, papelera
crates/nucleo/src/reportes.rs   registro de salidas, reportes por cliente, alertas
crates/nucleo/src/respaldo.rs   respaldo diario en .zip
crates/nucleo/src/api.rs        API HTTP para botoneras
crates/app/       programa de escritorio (egui): ventana, explorador, navegación, monitor, editor de pauta,
                  arranque, vista previa, biblioteca, reportes (Excel/PDF), usuarios, ffmpeg (descarga y
                  actualización), diseno (tamaño de las secciones y arrastrar y soltar),
                  ajustes, dialogo (ventanas emergentes y animaciones)
herramientas/     CasparCG simulado para pruebas y empaquetar.ps1 (arma el instalador)
instalador/       script de Inno Setup
.github/workflows publicar.yml: arma el instalador y publica la Release al subir un tag
```

## Limitaciones de esta versión

- La duración en el explorador se lee directamente de MP4/MOV/M4V y **MKV/WebM**; para otros formatos (MXF, AVI…) aparece cuando el video ya está en CasparCG (o la da ffmpeg al añadirlo).
- El monitor de programa reproduce el mismo archivo al mismo tiempo que CasparCG: no es la señal de salida de CasparCG (no muestra transiciones ni la capa de TV).
- El cumplimiento de la pauta se calcula con las pautas actuales, desde el primer día con registro. Si cambias las horas de una pauta, los días pasados se cuentan con las horas nuevas.
- Si en AUTO aparece un pase nuevo en ventana, la tanda se vuelve a armar (se pierden los cambios a mano hechos a esa tanda).
- La detección automática del corte (SCTE-35 del RX8200 o las cortinas del canal) queda como opción futura.
- El uso directo de videos (sin copiar) se probó con el simulador; en tu CasparCG real, mira el registro al conectar: debe decir *CasparCG reproduce videos desde cualquier carpeta*.
- Desde una PC remota, la vista previa solo funciona con videos de esa PC (los de CasparCG están en la principal).
- La revisión de la Biblioteca necesita ffmpeg en la PC principal. La primera vez revisa todos los videos (un spot de 30 s tarda menos de un segundo).
- El volumen objetivo (-23 LUFS) es la norma europea; si el canal usa otra, cámbiala en `[biblioteca] lufs`.
- La conexión remota no va cifrada: úsala solo dentro de la red del canal.
