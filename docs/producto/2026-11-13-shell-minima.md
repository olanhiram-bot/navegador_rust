# Shell mínima (fase 5, en curso)

Fecha: 2026-11-13

## Qué hay

`crates/shell` es una ventana egui con:

- **la página visible**, embebida desde Chromium por `Page.startScreencast`;
- **barra de dirección**: escribe una URL, Enter o «Ir»;
- **scroll** con la rueda, mapeado a coordenadas CSS de la página;
- **handoff**: «Ceder al agente» abre un panel con URL, título, generación,
  elementos accionables, número de recibos y ruta del log.

```bash
cargo run -p shell -- https://ejemplo.com --log /tmp/recibos.jsonl
```

La captura de la sesión de prueba está en `target/shell.png` (ignorado por
git): se ve la compra de `shop.html`, la barra y el panel de handoff con
`generación 2 · recibos 0`.

## Arquitectura

```
hilo egui (eframe)            hilo motor (tokio)
  ShellApp  ── comandos ──▶   Chrome + screencast
            ◀── eventos ───   Frame | Snapshot | Handoff | Error
```

- El motor es el único dueño de `Chrome`. La UI nunca toca CDP.
- Los frames llegan en JPEG, se decodifican a RGBA en el hilo del motor y la UI
  los sube a una textura. Cada frame se confirma con `screencastFrameAck`, sin
  lo cual Chromium deja de enviar.
- Los snapshots reutilizan `Chrome::snapshot` y mantienen la generación, así
  que el handoff habla el mismo idioma que MCP (`@eN`, generación, recibos).
- `EngineHandle::shutdown` pide el cierre y espera a que el hilo del motor
  dropee `Chrome`, que mata su grupo de procesos.

## Decisión: la shell es de solo lectura

No reenvía clicks a la página. Reenviarlos ejecutaría un formulario sin pasar
por descriptor → política → tarjeta → recibo, que es exactamente lo que el
producto existe para impedir: un click sobre «Pagar» no puede ejecutar el pago
sin la tarjeta.

Para actuar desde la shell hace falta el paso siguiente: hit-test del punto →
elemento → ref `@eN` → `Session::act` → tarjeta dentro de la ventana. Eso
mantiene el invariante y convierte la shell en navegador usable, no en un visor.

## Verificación

- `cargo test -p shell`: el test del puente abre `shop.html` contra Chromium
  real y exige un `Snapshot` con elementos y al menos un `Frame` con píxeles
  RGBA y tamaño CSS.
- Prueba manual en Wayland/Hyprland: ventana de 941×1150, página renderizada,
  barra con la URL y panel de handoff abierto.
- El cierre por gestor de ventanas no quedó verificado en esta sesión; el
  apagado ordenado del motor está cubierto por `shutdown` en el puente.

## Límites

- Una sola pestaña.
- Sin teclado hacia la página y sin clicks (solo lectura).
- El panel del agente no está conectado en proceso: el handoff produce el mismo
  informe que MCP y el agente conduce por `navegador-mcp`.
- El demo de 60 segundos del gate de la fase 5 no está grabado.

## Siguiente

1. Hit-test y acciones con tarjeta dentro de la shell.
2. Pestañas y panel del agente en la misma ventana.
3. Grabar el demo de 60 s de punta a punta.
