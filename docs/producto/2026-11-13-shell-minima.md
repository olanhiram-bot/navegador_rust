# Shell mínima (fase 5, en curso)

Fecha: 2026-11-13

## Qué hay

`crates/shell` es una ventana egui con:

- **pestañas**: una `Session<Chrome>` por pestaña, con su URL, generación y log;
- **la página visible**, embebida desde Chromium por `Page.startScreencast`;
- **barra de dirección**: abre una URL o busca en DuckDuckGo si escribes
  palabras; atrás/adelante/recargar contra el historial de CDP;
- **scroll** con la rueda, mapeado a coordenadas CSS de la página;
- **enlaces y campos**: un enlace navega a su `href`; un `textbox`,
  `searchbox`, `combobox` o `select` recibe un click real de foco y lo tecleado
  se reenvía (texto compuesto + teclas especiales);
- **acciones sobre botones**: click → hit-test → ref `@eN` → `Session::act` →
  tarjeta modal → aprobar/denegar → recibo;
- **panel del agente**: generación, refs vivas, recibos de la pestaña, ruta del
  log, handoff y cesión/recuperación del control.

```bash
cargo run -p shell -- https://ejemplo.com --log /tmp/recibos.jsonl

# Para que Google/Cloudflare no detecten el modo headless:
# Arch: sudo pacman -S xorg-server-xvfb
# Con Xvfb en PATH, Chromium corre headful en un display virtual.
# NAVEGADOR_HEADLESS=1 fuerza headless (CI).
```

## Arquitectura

```
hilo egui (eframe)              hilo motor (tokio)
  ShellApp  ── comandos ──▶     por pestaña: Session<Chrome>
            ◀── eventos ───     Frame | Snapshot | Act | Receipts | Handoff
```

- El motor es el único dueño de los navegadores. La UI nunca toca CDP.
- Si `Xvfb` está instalado, Chromium corre headful en un display X virtual y se
  sigue viendo por screencast. Sin Xvfb cae a `--headless=new`; ahí los sitios
  con anti-bot pueden rechazarlo.
- Los frames llegan en JPEG, se decodifican a RGBA en el hilo del motor y la UI
  los sube a una textura por pestaña. Cada frame se confirma con
  `screencastFrameAck`, sin lo cual Chromium deja de enviar.
- Solo la pestaña activa emite screencast: al cambiar se detiene el anterior y
  se arranca el nuevo. Los snapshots mantienen la generación, así que la shell
  habla el mismo idioma que MCP (`@eN`, generación, recibos).
- Cada pestaña escribe su propio log (`recibos-tab0.jsonl`, …) para no romper la
  cadena con dos escritores; MCP sigue usando su log independiente.
- `EngineHandle::shutdown` pide el cierre y espera a que el hilo del motor
  dropee las sesiones, que matan sus grupos de procesos.
- El motor despierta a la UI con `request_repaint` cuando emite un evento y la
  UI descarta los frames viejos: solo aplica el último de cada lote.
- El screencast va a JPEG 80 con tope 1600×1000; el `deviceScaleFactor` se
  limita a 1.75 y a un presupuesto de 4.5 Mpx.
- La decodificación JPEG corre en `spawn_blocking`, y `image`, `zune-jpeg` y
  `base64` se compilan con `opt-level = 3` también en `dev` (sin eso, un frame
  tardaba ~150 ms y la navegación se sentía rota).

## Actuar sin reenviar el click

El click no se manda a la página. Se hace `elementFromPoint` en el mundo
aislado, se devuelve el selector CSS del elemento accionable y se cruza con el
snapshot vivo para obtener su ref. Si el selector no está en el snapshot, no se
actúa: una ref solo vale para la generación que la produjo.

Solo los elementos con rol `button` pasan a `Session::act`. Los enlaces se
resuelven a su `href` absoluto y navegan; los campos de texto y `select`
reciben un click real para enfocar, sin ejecutar ninguna acción. Checkboxes,
radios y elementos `clickable` se anuncian y no se tocan. La acción sigue las
dos fases de siempre: sin aprobación, `act` devuelve el hash y la tarjeta; al
aprobar, se llama con ese hash exacto, se observa la petición y solo entonces se
firma el recibo. La tarjeta es una ventana egui con las filas reales, avisos y
el hash corto; «Denegar» está siempre disponible.

## Panel del agente

El panel está conectado a la misma sesión que la página: muestra la generación,
las refs que el agente puede citar, el número y último hash de recibo, la ruta
del log y el informe de handoff. «Ceder control» bloquea los clicks de la shell
mientras el agente conduce y genera ese informe.

**Límite declarado:** el agente no se conecta por un socket a la ventana; sigue
conduciendo por `navegador-mcp`. El panel comparte el estado de la sesión local,
no un canal MCP en proceso.

## Verificación

- `cargo test -p shell`: cuatro tests contra Chromium real y dos unitarios.
  - frame RGBA + tamaño CSS y snapshot con elementos;
  - click sobre «Pagar $24.90» → `ApprovalRequired` con la tarjeta mostrando
    `24.90` → aprobar → `Executed` con recibo 0 verificado en disco;
  - abrir `settings.html` en una segunda pestaña, volver a la primera y cerrar
    la segunda;
  - navegar `shop → settings`, volver atrás y adelante con el historial.
- El primer test del puente comprueba el apagado ordenado del motor; tras la
  suite no quedan procesos de Chromium.

## Límites

- Checkboxes, radios y elementos `clickable` no se accionan: no hay tarjeta
  para ellos.
- Sin Xvfb, algunos sitios (Google, Cloudflare) bloquean Chromium headless como
  tráfico automatizado; no es un fallo de la shell.
- Matar la shell con SIGTERM deja Chromium vivo: el cierre ordenado solo corre
  al salir normalmente.
- El panel del agente no es un canal MCP en proceso.
- El demo de 60 segundos del gate de la fase 5 no está grabado.

## Siguiente

1. Grabar el demo de 60 s de punta a punta.
2. Checkboxes, radios y controles de formulario con su política.
3. Conectar de verdad al agente (socket o MCP embebido) y reflejar sus actos en
   el panel.
