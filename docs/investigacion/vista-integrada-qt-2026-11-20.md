# Vista integrada de Chromium con Qt WebEngine (2026-11-20)

Prueba exploratoria para comparar la vista actual por JPEG con una vista de
Chromium embebida en la ventana (Qt WebEngine 6.12 sobre Chromium 6.12), antes
de decidir si conviene reescribir la shell.

## Prototipo

Se compiló un programa mínimo de 40 líneas con `QWebEngineView`, barra de
dirección y `Atrás/Adelante`:

```bash
c++ -O2 -fPIC -pie -std=c++17 main.cpp -o navegador-vista \
    $(pkg-config --cflags --libs Qt6WebEngineWidgets)
```

`-fPIC -pie` es necesario en Arch: sin ellos el enlazado falla con
`copy relocation against non-copyable protected symbol _ZTI7QObject`.

## Verificación de entrada y de acciones del agente

Con un formulario local (`<form method=POST action=/submit>`), un clic real
enviado con XTest llegó a la página. Con `QTWEBENGINE_REMOTE_DEBUGGING` y
`Fetch.enable`, el agente observó método, destino y cuerpo del POST antes de que
saliera y pudo cancelarlo con `Fetch.failRequest`. Es decir: la vista integrada
no renuncia a la verificación por CDP.

## Latencia visible

Mismo método que la medición del `ROADMAP`: página local que cambia de color al
pulsar un enlace, Xvfb de 1150 × 900, cinco repeticiones por navegador,
alternando el orden, clic generado con XTest y detección del color en la
pantalla real.

| Navegador | Primer contenido, mediana | Clic a cambio visible, mediana |
|---|---:|---:|
| Proyecto en release, un Chromium por sesión | 393 ms | 74 ms |
| Qt WebEngine integrado | 493 ms | 549 ms |
| Brave 158.1.100.58 | 340 ms | 45 ms |
| Firefox 158.0.1 | 908 ms | 62 ms |

La cifra de Qt engaña: con `Page.captureScreenshot` por CDP, la página nueva ya
está pintada a los ~100 ms del clic; además, un cambio de estilo disparado por
un temporizador JS aparece en pantalla de inmediato. El retraso de ~550 ms está
en la presentación de la superficie nueva de Qt WebEngine bajo Xvfb, no en
Chromium ni en la red.

## Veredicto

- En este entorno, la vista integrada **no** mejora el tiempo de clic a cambio
  visible y, por tanto, no es evidencia suficiente para reescribir la shell.
- Sí confirma que una vista de Chromium real convive con el control del agente.
- Antes de adoptarla conviene medirla en un escritorio con compositor (Wayland o
  X11 real) y comparar también selección de texto, video y DPI.
