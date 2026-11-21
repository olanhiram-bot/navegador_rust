# Plan para la vista integrada de Chromium

Estado: decisión pendiente. Este documento reúne la evidencia y el plan; no
implica haberla implementado.

## Problema

La shell pinta la página como JPEG por CDP. Eso limita el producto en dos
frentes:

1. **Rendimiento.** Medición local (Linux, Xvfb, release): vídeo 130–270 % de un
   núcleo y animación a 60 fps 250–500 %; Brave, 15–19 %. Con la ventana sin
   foco baja a ~11 % porque se pausa el screencast.
2. **Compatibilidad de interfaz.** Todo lo que Chromium dibuja fuera del
   documento no llega: `<select>`, selector de fechas, subida de archivos,
   `alert/confirm`, autenticación HTTP, menú contextual nativo, PDF, avisos de
   contraseña, IME y árbol de accesibilidad.

## Evidencia ya recogida

- Prototipo de 40 líneas con Qt WebEngine: recibe clic real y el agente puede
  verificar y cancelar el POST por CDP. Bajo Xvfb presenta la superficie nueva
  ~550 ms después del clic; Chromium ya la tenía pintada a los ~100 ms.
- La vista por JPEG responde en ~74 ms de mediana en la misma prueba.
- Nota completa: `docs/investigacion/vista-integrada-qt-2026-11-20.md`.

## Opciones

### A. Qt WebEngine embebido (reescritura de la shell)

- A favor: vista nativa de Chromium, sin JPEG, controles propios visibles.
- En contra: reescribir la UI (egui → Qt), dependencia grande, y el agente
  seguiría por CDP porque Qt expone el mismo protocolo.
- Riesgo: alto; dos toolkits (Qt para la ventana, Rust para el motor).

### B. Incrustar la ventana de Chromium en la ventana de egui (X11)

- A favor: conserva toda la shell y el motor; la página y sus diálogos son los
  de Chromium; el screencast queda como respaldo. El agente no cambia: sigue
  hablando CDP con el mismo proceso.
- En contra: en Wayland nativo no se puede reparentar una superficie ajena; hay
  que ejecutar Chromium bajo XWayland y embeber su ventana X11, o usar
  `wlr-layer-shell`/subsuperficies (no soportado por Chromium).
- Riesgo: medio. Hay que resolver foco, teclado, DPI, posición y ciclo de vida.

### C. Seguir con JPEG y arreglar casos sueltos

- A favor: sin obra.
- En contra: no arregla `<select>`, archivos, PDF, IME ni el coste de CPU.

## Recomendación

**B, con la vista por JPEG como respaldo.** Chromium corre en un display X11
(Xvfb o XWayland) y su ventana se reparenta dentro del área de página de egui.
Cuando el reparentado no sea posible (Wayland puro sin XWayland, compositores
exóticos), la shell usa el screencast actual sin cambiar nada del motor.

## Desglose

1. **Spike (2–3 días).** Reparentar una ventana de Chromium en un contenedor X11
   embebido en egui. Medir: CPU, latencia clic→visible, teclado, DPI, resize.
2. **Integración (1 semana).** Mover el foco entre omnibox y página, ocultar la
   barra de Chromium, ajustar tamaño por pestaña y sincronizar pestañas
   (targets) con la tira de la shell.
3. **Diálogos y archivos (3–5 días).** Verificar `<select>`, subida de archivos,
   `alert/confirm`, HTTP auth y PDF con la vista nueva.
4. **Respaldo (2 días).** Interruptor en ajustes: integrada / screencast; pruebas
   de regresión con ambas.
5. **Accesibilidad e IME (3 días).** Comprobar lector de pantalla y entrada
   china/japonesa; documentar limitaciones si las hay.

## Criterios de aceptación

- Los 16 recorridos de `crates/shell/tests/bridge.rs` pasan con la vista
  integrada y con el respaldo JPEG.
- CPU en vídeo ≤ 2× Brave en la misma máquina.
- `<select>`, subir un archivo y un `alert()` funcionan sin salir de la ventana.
- El agente sigue pudiendo cancelar un POST antes de que salga (prueba existente).
- Nada de esto cambia el formato de recibos ni la política.

## Riesgos

- Wayland sin XWayland: no hay reparentado; el respaldo debe ser automático.
- Foco y atajos: la ventana embebida puede robar el teclado al omnibox.
- Empaquetado: Qt no hace falta en la opción B, pero sí X11/XWayland.
