# Shell mínima (fase 5, en curso)

Fecha: 2026-11-13

## Qué hay

`crates/shell` es una ventana egui con:

- **pestañas**: una `Session<Chrome>` por pestaña, con su URL, generación y log;
- **la página visible**, embebida desde Chromium por `Page.startScreencast`;
- **barra de dirección**: escribe una URL, Enter o «Ir»;
- **scroll** con la rueda, mapeado a coordenadas CSS de la página;
- **acciones sobre botones**: click → hit-test → ref `@eN` → `Session::act` →
  tarjeta modal → aprobar/denegar → recibo;
- **panel del agente**: generación, refs vivas, recibos de la pestaña, ruta del
  log, handoff y cesión/recuperación del control.

```bash
cargo run -p shell -- https://ejemplo.com --log /tmp/recibos.jsonl
```

## Arquitectura

```
hilo egui (eframe)              hilo motor (tokio)
  ShellApp  ── comandos ──▶     por pestaña: Session<Chrome>
            ◀── eventos ───     Frame | Snapshot | Act | Receipts | Handoff
```

- El motor es el único dueño de los navegadores. La UI nunca toca CDP.
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

## Actuar sin reenviar el click

El click no se manda a la página. Se hace `elementFromPoint` en el mundo
aislado, se devuelve el selector CSS del elemento accionable y se cruza con el
snapshot vivo para obtener su ref. Si el selector no está en el snapshot, no se
actúa: una ref solo vale para la generación que la produjo.

Solo los elementos con rol `button` pasan a `Session::act`. Enlaces y campos de
texto se anuncian en la barra de estado y no ejecutan nada. La acción sigue las
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

- `cargo test -p shell`: tres tests contra Chromium real.
  - frame RGBA + tamaño CSS y snapshot con elementos;
  - click sobre «Pagar $24.90» → `ApprovalRequired` con la tarjeta mostrando
    `24.90` → aprobar → `Executed` con recibo 0 verificado en disco;
  - abrir `settings.html` en una segunda pestaña, volver a la primera y cerrar
    la segunda.
- El primer test del puente comprueba el apagado ordenado del motor; tras la
  suite no quedan procesos de Chromium.

## Límites

- Sin teclado hacia la página: no se escriben campos de texto.
- Los clicks en enlaces todavía no navegan; se usa la barra de dirección.
- El panel del agente no es un canal MCP en proceso.
- El demo de 60 segundos del gate de la fase 5 no está grabado.

## Siguiente

1. Grabar el demo de 60 s de punta a punta.
2. Teclado hacia campos y navegación por enlaces.
3. Conectar de verdad al agente (socket o MCP embebido) y reflejar sus actos en
   el panel.
