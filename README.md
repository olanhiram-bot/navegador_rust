# Navegador auditable

Un navegador local y abierto donde una persona y un agente comparten la misma
sesión, y donde toda acción con consecuencias queda atada a la acción que
realmente se ejecuta.

**Estado: fase 3 completa; fase 4 en curso.** El descriptor ground-truth
detecta los ataques, la tarjeta muestra el valor real antes de aprobar, la
sustitución posterior a la aprobación se aborta 10/10 y cada acción ejecutada
deja un recibo firmado que se verifica offline. Un host MCP completa un flujo de
`open` → `snapshot` → `act` por stdio. La fase 4 añade política de orígenes
read-only/read-write, perfiles efímeros y 3 escenarios locales con 0 falsos
bloqueos; falta validarlos en sitios reales. La fase 5 ya tiene una **shell
mínima**: pestañas, página visible por screencast CDP, barra de dirección con
búsqueda, scroll, navegación por enlaces, campos de texto con teclado,
atrás/adelante, acciones sobre botones con tarjeta modal y recibo, y panel del
agente con generación, refs y handoff. Todas las pestañas comparten un único
Chromium, con perfil persistente y restauración de la sesión. Falta el demo de
60 s grabado.

## Instalación

```bash
packaging/install.sh              # binario en ~/.local/bin y entrada de escritorio
packaging/install.sh /otro/prefijo
navegador-shell --version
```

El instalador compila con `--locked`; para actualizar, vuelve a ejecutarlo. No
toca archivos del sistema y usa el Chromium instalado, que recibe sus parches
con el sistema.

## Cómo correrlo

```bash
cargo test --workspace --all-targets   # unitarios, arnés de ataques y gate MCP (requiere Chromium)

# shell mínima: página visible, barra de dirección y handoff
cargo run -p shell -- https://ejemplo.com --log /tmp/recibos.jsonl

# perfil persistente y restauración de pestañas (por defecto)
#   perfil:   ~/.local/share/navegador/perfil
#   sesión:   ~/.local/state/navegador/sesion.json
# --efimero no guarda nada y borra el perfil al cerrar; --perfil DIR elige otro
# directorio; --sin-restaurar abre una pestaña en lugar de las de la última vez.
cargo run -p shell -- --efimero https://ejemplo.com

# atajos: Ctrl+T pestaña, Ctrl+W cerrar, Ctrl+Shift+T deshacer cierre,
# Ctrl+R recargar, Ctrl+Q salir, Ctrl+L barra de direcciones,
# Ctrl+F buscar en la página, Ctrl++ / Ctrl+- / Ctrl+0 zoom del sitio,
# Ctrl+D marcador, Ctrl+Shift+B marcadores, Ctrl+H historial, Ctrl+, ajustes
# clic central y clic derecho: abrir enlace aparte, copiar enlace, navegar
# descargas: ~/Downloads o $NAVEGADOR_DESCARGAS
# permisos por sitio y descargas: menú ☰ de la barra

# recomendado para sitios con captcha (Chromium headful en display virtual)
# Arch: sudo pacman -S xorg-server-xvfb
# NAVEGADOR_HEADLESS=1 fuerza el modo headless (CI)

# servidor MCP por stdio, con log de recibos encadenado
cargo run -p mcp -- --log /tmp/recibos.jsonl --max-amount 20

# auditar sin ejecutar: tarjeta + descriptor + decisión
cargo run -p navegador -- audit https://ejemplo.com "#pagar" 1.00 ana

# ejecutar con aprobación explícita y recibo
cargo run -p navegador --features gui -- run https://ejemplo.com "#pagar" 1.00 ana --gui
cargo run -p navegador -- run https://ejemplo.com "#pagar" 1.00 ana --yes --out recibo.json

# verificar el recibo en cualquier máquina, sin estado compartido
cargo run -p navegador -- verify recibo.json

# gestionar la identidad local
cargo run -p navegador -- keygen
```

Códigos de salida: `0` correcto · `2` denegado por política o recibo inválido ·
`3` denegado por la persona · `4` acción bloqueada por binding.

`--gui` requiere `--features gui` y un display. Sin él, `--yes` aprueba el hash
del descriptor de forma no interactiva (útil para CI y scripts).

`NAVEGADOR_CHROME` permite apuntar a un binario concreto. Por defecto se buscan
`chromium`, `chromium-browser` y `google-chrome-stable`. Si `Xvfb` está en
`PATH`, la shell lanza Chromium headful dentro de un display virtual; así los
sitios no lo detectan como navegador automatizado. `NAVEGADOR_HEADLESS=1`
fuerza headless.

## Limitaciones conocidas

- Los botones que solo ejecutan JavaScript, sin formulario ni destino, se
  bloquean por política: no se puede demostrar qué petición van a hacer. Los
  enlaces, formularios y envíos con Enter sí pasan por tarjeta y recibo.
- Las pestañas que abre la web (`window.open`, `target=_blank`) se adoptan como
  pestañas de la shell, pero los diálogos nativos de Chromium (selectores,
  subida de archivos, alertas) no se ven en la vista por screencast.
- Sin Widevine no hay DRM; las notificaciones concedidas no llegan al escritorio
  porque Chromium corre en un display virtual.

## Estructura

```
crates/descriptor/   ActionDescriptor, hash canónico, intención, política, binding
crates/engine/       Trait Engine + Snapshot + refs @eN con generación
crates/engine-cdp/   Lanzador de Chromium + cliente CDP + extracción en mundo aislado
crates/card/         Modelo de la tarjeta (testeable sin pantalla) + render egui
crates/receipt/      Recibo ed25519 firmado, verificación offline y log encadenado
crates/session/      SessionCore: refs, política de orígenes, tarjeta, dispatch y recibo
crates/mcp/          Servidor MCP por stdio (open, snapshot, act, read, screenshot, handoff)
crates/shell/        Shell egui: pestañas, screencast, omnibox, tarjeta y panel del agente
crates/navegador/    Servidor de fixtures, CLI y arnés de ataques
fixtures/            exfil, lie-in-the-loop, sustitución y tarea legítima
docs/                investigación, tesis y plan
```

## Qué demuestra el arnés

| Escenario | Qué hace la página | Qué lo detecta |
|---|---|---|
| `exfil.html` | envía el pago a otro origen | origen de acción no permitido |
| `lie.html` | muestra `$1.00`, envía `1000.00` | importe real ≠ intención + inconsistencia visible |
| `substitute.html` | reescribe el formulario en el click | binding: `Fetch` aborta el POST antes de salir |
| `legit.html` | transferencia real de `$1.00` a Ana | se aprueba, se ejecuta y deja recibo |

El punto clave: el árbol de accesibilidad solo ve la etiqueta que miente. El
descriptor se construye desde DOM, estado de formulario y origen resuelto, y el
binding compara la petición interceptada con lo aprobado.

## El recibo

El recibo lleva el descriptor, la petición real (`wire`), el resultado y la clave
pública; la firma cubre el JSON canónico de todo eso. `verify` comprueba, en
orden: esquema, firma, hash del descriptor y binding petición↔descriptor.

Es autocontenido: prueba integridad y binding, **no identidad**. Para saber que
lo emitió *tu* navegador hay que fijar su clave pública. Es un límite
documentado, no un fallo oculto.

## Documentos

- [Tesis de producto](docs/producto/2026-11-13-tesis-producto.md)
- [Plan de arranque del MVP](docs/producto/2026-11-13-plan-arranque-mvp.md)
- [Resultado de la semana 1](docs/producto/2026-11-13-resultado-semana-1.md)
- [Resultado de la semana 2](docs/producto/2026-11-13-resultado-semana-2.md)
- [Resultado de la semana 3](docs/producto/2026-11-13-resultado-semana-3.md)
- [Resultado de la semana 4](docs/producto/2026-11-13-resultado-semana-4.md)
- [Shell mínima](docs/producto/2026-11-13-shell-minima.md)
- [Recomendación de arquitectura](docs/investigacion/2026-11-12-recomendacion-arquitectura.md)
- [Plan completo](PLAN.md)
