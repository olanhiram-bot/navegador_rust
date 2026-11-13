# Navegadores web: panorama GitHub (corte 2026-11-13)

Investigación previa a decidir arquitectura para un navegador propio.
Todo dato de estrellas/actividad fue consultado vía API de GitHub el **2026-11-13**;
los porcentajes de conformidad vienen del dashboard público de Blitz (WPT, 2026-11-12/13)
y de cifras publicadas por cada proyecto.

---

## 0. TL;DR — lo que hay que retener

1. **En Rust sí existe ecosistema de navegador completo.** Servo (38.5k★) está activo,
   publica en crates.io (`servo 0.8.0`, nov-2026), tiene WebView API, LTS y ~75.8% de WPT.
   Blitz (4.4k★) es la alternativa modular HTML/CSS sin JS. Hay además 3–4 motores nuevos
   (Gosub, Moli, Obscura) y una camada de motores "headless para agentes IA".
2. **En Zig hay exactamente un motor serio: Lightpanda** (42k★, AGPL-3.0, v1.0.3).
   Pero **no renderiza píxeles a propósito**: es DOM + JS (V8) + CDP, sin layout visual.
3. **Ningún navegador gráfico usable está escrito en Zig.** Lo más cercano es
   *Turtle* (motor from scratch + V8, pre-alpha, solo macOS) y *Arix* (13★, motor
   from scratch sin JS ni red). Ambos son pruebas de existencia, no productos.
4. **Zig gana en interoperabilidad con C y control de memoria; pierde en ecosistema.**
   Para un motor propio da igual el lenguaje; para *reutilizar* piezas (parser HTML,
   CSS, fuentes, accesibilidad) Rust tiene 10 años de ventaja.
5. **La categoría caliente en 2026 no es "navegador de escritorio", es "navegador
   headless para agentes IA"**: Lightpanda (Zig), Obscura y Moli (Rust), agent-browser
   (Rust CLI sobre Chrome). Si el objetivo es automatización/scraping, el diseño cambia
   por completo (no necesitas paint).
6. **Escribir un motor desde cero es viable pero caro**: Turtle = 2 años de una persona
   con ayuda de agentes IA para Acid3 = 100 y ~70% de webs. Llegar a "navegador diario"
   es un proyecto de 5+ años o de un equipo financiado (Ladybird).
7. **Licencias mandan**: Servo MPL-2.0 (ok para embeber), Blitz Apache-2.0, Ladybird BSD-2,
   Lightpanda **AGPL-3.0** (problemático si cierras el producto).

---

## 1. Cómo se descompone un navegador (para no comparar peras con manzanas)

Un "navegador" en GitHub puede ser cinco cosas distintas:

| Capa | Qué hace | Ejemplos |
|---|---|---|
| **Motor de render** | HTML → DOM → estilo → layout → paint | Servo, Blitz, LibWeb, Gecko |
| **Motor headless** | DOM + JS + red, **sin paint** | Lightpanda, Obscura, Moli |
| **Shell / WebView** | Ventana + pestañas + URL bar + motor del SO | Tauri/wry, webview, CEF, Ultralight |
| **Navegador completo** | Motor + UI + red + storage + DevTools | Ladybird, Chrome, Firefox |
| **Automatización** | Conduce otro navegador vía CDP/WebDriver | Playwright, agent-browser |

Confundir capas es el error #1 al leer rankings de estrellas: `agent-browser` (50k★)
no es un motor, es un CLI que descarga Chrome.

---

## 2. Panorama global (referencia, no Rust/Zig)

| Proyecto | Lenguaje | Licencia | Estado 2026 | Nota |
|---|---|---|---|---|
| Chromium / Blink | C++ | BSD-3 | Producción | 95.2% WPT; monolito imposible de embeber barato |
| Gecko / Firefox | C++ + Rust (Stylo, WebRender) | MPL-2.0 | Producción | 90.3% WPT |
| WebKit | C++ | BSD-2/LGPL | Producción | 88.9% WPT; base de Safari y WebKitGTK/WPE |
| **Ladybird** | C++ **→ Rust** | BSD-2 | Alpha (2026) | 67k★; 90.2% WPT; desde feb-2026 porta subsistemas a Rust (LibJS ya es 100% Rust, Wasm, canvas, IndexedDB, URL) |
| NetSurf | C | GPL-2 | Mantenimiento | Motor propio, muy ligero, sin JS moderno |
| Dillo | C | GPL-3 | Revivido | Motor propio minimalista, sin JS |
| Flow (Ekioh) | C++ | Propietaria | Comercial | Embebible, más maduro que Servo/Ladybird |
| Ultralight | C++ | Comercial | Activo | HTML/CSS/JS ligero para apps |
| CEF | C++ | BSD-3 | Activo | Chromium embebido; binarios enormes |
| Sciter | C++ | Comercial | Activo | HTML/CSS propio para UI de apps |
| Carbonyl | C++ | BSD-3 | Estancado (2024) | Chromium en terminal |
| Browsh | Go | MIT | Mantenimiento | Firefox headless → texto |
| Lynx / w3m / links | C | GPL | Maduros | Navegadores de texto, sin CSS moderno |

**Lección:** los tres motores dominantes suman millones de líneas y décadas.
Los intentos independientes que sobreviven (NetSurf, Dillo, Ladybird) son
o muy limitados o financiados.

---

## 3. Rust

### 3.1 Motores (el núcleo de la decisión)

| Proyecto | ★ | Licencia | Actividad | Estado real |
|---|---:|---|---|---|
| [servo/servo](https://github.com/servo/servo) | 38.5k | MPL-2.0 | commits 2026-11-13 | v0.8.0 (nov-2026). `servo` en crates.io, LTS semestral, WebView API documentada (doc.servo.org), WebGL/WebGPU, WebDriver, accesibilidad vía AccessKit. **75.8% WPT** (873k/1.15M en el dashboard de Blitz); Servo reporta 1.9M subtests / 2.13M (89.3%) con su propio conteo. ~487 PRs/mes, ~8k USD/mes de donaciones. |
| [DioxusLabs/blitz](https://github.com/DioxusLabs/blitz) | 4.4k | Apache-2.0 | 2026-11-13 | Beta. HTML/CSS modular (flex, grid, tablas, selectores, variables, formularios, AccessKit) sobre `blitz-dom` + Taffy + Vello/wgpu. **Sin JS, sin red, sin storage** (por diseño). 10.9% WPT. Ideal como renderer embebido, no como navegador. |
| [gosub-io/gosub-engine](https://github.com/gosub-io/gosub-engine) | 3.7k | MIT | 2026-11-11 | Motor embebible async: multi-zona/pestaña, red propia, sandbox de procesos, V8, backends Cairo/Skia/Vello, SVG, pipeline de layout con Taffy. Muy ambicioso, aún inmaduro. |
| [lexmount/moli](https://github.com/lexmount/moli) | 23.6k | Apache-2.0 | 2026-11-13 | Creado ago-2026, 92 MB de Rust, workspace de ~70 crates (`moli-dom`, `moli-layout`, `moli-paint`, `moli-renderer-v8`, forks de stylo/parley). Headless con layout/paint **on-demand** (screenshots/PDF), CDP + WebDriver BiDi. 647 issues abiertos: prometedor y ruidoso. |
| [h4ckf0r0day/obscura](https://github.com/h4ckf0r0day/obscura) | 34.2k | Apache-2.0 | 2026-11-13 | Creado abr-2026. Headless "antidetect": `obscura-dom`, `obscura-net`, `obscura-js` (V8), `obscura-render` opcional con Taffy + tiny-skia + ab_glyph. CDP + MCP. Mismo patrón que Moli: motor propio pequeño + V8. |
| [versotile-org/verso](https://github.com/versotile-org/verso) | 5.4k | Apache-2.0 | **Archivado 2025-10** | Navegador sobre Servo (multi-ventana, UI en el propio motor). Murió por falta de fondos al no seguir el ritmo de Servo. Sus mejoras se integraron en Servo. **Caso de estudio obligatorio.** |
| [twilco/kosmonaut](https://github.com/twilco/kosmonaut) | 1.7k | MIT | 2025-11 | Motor educativo con serie de blog; CSS/layout parcial, sin JS. |
| [mbrubeck/robinson](https://github.com/mbrubeck/robinson) | 1.7k | MIT | 2014 | "Toy rendering engine" del blog *Let's build a browser engine*. Referencia didáctica. |
| [lucid-softworks/browser](https://github.com/lucid-softworks/browser) | 131 | MIT | 2026-09 | Motor from scratch multiplataforma (red, HTML/CSS, layout). |
| [ZeroX-DG/moon](https://github.com/ZeroX-DG/moon) | 435 | MIT | 2026-05 | Navegador hobby desde 2020 (Rust + WGSL). |
| [PhotonBrowser/photon](https://github.com/PhotonBrowser/photon) | 194 | BSD-2 | 2026-11 | Navegador nuevo, poca documentación aún. |
| [tathyagarg/harbor](https://github.com/tathyagarg/harbor) | 20 | MIT | 2026-10 | Rust + Zig, wgpu/winit, parser HTML casi spec, TTF propio. Didáctico. |
| [konippi/servo-fetch](https://github.com/konippi/servo-fetch) | 164 | MIT | 2026-11 | Servo como librería → Markdown/JSON. Ejemplo real de embedding. |
| [project-oxi/oxibrowser](https://github.com/project-oxi/oxibrowser) | 49 | — | 2026-11 | Headless CDP sobre Servo. |

### 3.2 Shells / WebViews

| Proyecto | ★ | Licencia | Qué aporta |
|---|---:|---|---|
| [tauri-apps/tauri](https://github.com/tauri-apps/tauri) | 112.9k | MIT/Apache-2.0 | App de escritorio con webview del SO; runtime `wry`. |
| [tauri-apps/wry](https://github.com/tauri-apps/wry) | 5.1k | MIT/Apache-2.0 | WebView multiplataforma (WebKitGTK / WKWebView / WebView2). |
| [webview/webview](https://github.com/webview/webview) | 14.3k | MIT | Versión C/C++; base de `zig-webview`. |
| `webkit2gtk` (crate) | — | MIT | Bindings directos a WebKitGTK: control total de pestañas/UI. |

**Trade-off estructural (2026):** Tauri/wry sigue dependiendo de 3 motores del SO
(fragmentación de CSS/JS y ciclos de update distintos). La promesa de Verso
("un solo motor, Servo, en todas partes") **no se cumplió**; lo que sí llegó fue
`cargo add servo`, es decir, embeber el motor tú mismo y pagar el coste de binario.

### 3.3 Piezas reutilizables (versiones reales en crates.io, 2026-11-13)

| Área | Crate | Versión | Autor/uso |
|---|---|---|---|
| HTML | `html5ever` | 0.40.1 | Servo; parser spec-compliant, ~149M descargas |
| CSS parser | `cssparser` | 0.38.0 | Servo |
| Selectores | `selectors` | 0.43.0 | Servo/Stylo |
| Estilo | `stylo` (repo) / `servo_arc` | — | Firefox + Servo comparten motor de estilo |
| Layout | `taffy` | 0.15.1 | Dioxus; flex/grid/block |
| Render GPU | `vello` | 0.12.0 | Linebender, compute-based 2D |
| Render GPU | `webrender` | 0.70.1 | Servo/Firefox |
| Texto | `parley` 0.13 / `cosmic-text` 0.19 | — | shaping + bidi + layout de líneas |
| Shaping | `rustybuzz` 0.20 | — | port de HarfBuzz |
| Fuentes | `swash` 0.2.10 | — | rasterizado |
| SVG | `resvg` 0.50 | — | Linebender |
| JS | `boa_engine` 0.22 | — | JS puro en Rust, sin JIT, ~80% test262 |
| JS | `rquickjs` 0.14 | — | QuickJS embebido |
| JS | `deno_core` 0.413 / `v8` 152 | — | V8 real (lo que usan Moli/Obscura) |
| CSS tools | `lightningcss`, `oxc` | — | minificado/parseo, no motor |
| Ventana | `winit` 0.31-beta | — | base de casi todo |
| GPU | `wgpu` 30 | — | Vulkan/Metal/DX12/WebGPU |
| Framebuffer | `softbuffer` 0.4.8 | — | CPU fallback |
| Accesibilidad | `accesskit` 0.25 | — | árbol a11y multiplataforma |
| Red | `reqwest` 0.13, `hickory-resolver` 0.26 | — | HTTP/2-3, DNS |

**Conclusión Rust:** puedes construir desde "solo UI + webview" hasta "motor propio"
sin salir del ecosistema, y cada pieza tiene mantenimiento real.

---

## 4. Zig

### 4.1 Motores y componentes

| Proyecto | ★ | Licencia | Qué es | Madurez |
|---|---:|---|---|---|
| [lightpanda-io/browser](https://github.com/lightpanda-io/browser) | 42.0k | **AGPL-3.0** | Motor headless from scratch: Zig 0.17 + V8 + `zigdom` + html5ever (FFI Rust) + libcurl. CDP, WebDriver BiDi, MCP, `fetch`. **Sin motor gráfico**: `captureScreenshot` devuelve placeholder. 1.0.3 (nov-2026), 1.7M subtests WPT, 10k commits, ~20 MB/instancia vs ~180 MB Chrome. | **Producción** (para automatización) |
| [Arcelyth/arix](https://github.com/Arcelyth/arix) | 13 | MIT | Motor gráfico from scratch en Zig: parser HTML (testeado contra html5lib), CSS (selectores, cascada, computed styles), DOM, fuentes (matching + shaping), layout block/inline, paint (fondos, bordes, texto). **Sin red ni JS.** | Alpha temprana, muy activo |
| [chadwain/zss](https://github.com/chadwain/zss) | 50 | — | CSS parser + layout + renderer en Zig (2020). | Estancado |
| [chadwain/rem](https://github.com/chadwain/rem) | 147 | — | Parser HTML en Zig. | Mantenimiento |
| [SmallThingz/zhtml](https://github.com/SmallThingz/zhtml) | 4 | — | Parser HTML rápido (>1 GiB/s), razonablemente conforme. | Nuevo |
| [zig-utils/zig-js](https://github.com/zig-utils/zig-js) | 33 | — | Motor JS puro en Zig (v0.0.1). | Experimental |
| `zjs` / `quickjs-zig` / fork de Kiesel | <10 | — | JS embebible en Zig (varios intentos). | Experimental |
| [ryanhair/laffy](https://github.com/ryanhair/laffy), [Addonay/zafi](https://github.com/Addonay/zafi) | 0 | — | Ports de Taffy (flex/grid/block). | Muy nuevos |
| [mlugg/zig-fontmanager](https://github.com/mlugg/zig-fontmanager), [ziglibs/fontaine](https://github.com/ziglibs/fontaine) | 15/37 | — | Texto vía FreeType/HarfBuzz. | Limitados |
| [happystraw/zig-webview](https://github.com/happystraw/zig-webview) | 12 | MIT | Bindings a `webview/webview`. | 0.1.0 |
| [webui-dev/zig-webui](https://github.com/webui-dev/zig-webui) | 845 | MIT | WebUI como GUI con backend Zig. | Beta |
| [thechampagne/webview-zig](https://github.com/thechampagne/webview-zig) | 199 | — | Otro binding a webview. | Mantenimiento |
| [capy-ui/capy](https://github.com/capy-ui/capy) | 2.7k | MIT | UI nativa multiplataforma (no web). | Activo |
| [david-vanderson/dvui](https://github.com/david-vanderson/dvui) | 1.8k | MIT | GUI inmediata en Zig. | Activo |

Fuera de GitHub pero relevante:

- **Turtle** (`code.intellios.ai/cwbrowser`) — motor completo en Zig + V8, pre-alpha 0.01.4,
  solo macOS/arm64. HTML/CSS/layout/paint propios, Core Text, Metal. Acid3 = 100,
  html5test 425, ~70% de webs según su autor. 2 años de trabajo en solitario.
- **Harbor** — motor didáctico Rust+Zig (también en GitHub, 20★).

### 4.2 Huecos reales de Zig (a día de hoy)

- No hay un motor gráfico usable en producción (ni de lejos un Servo/Blitz).
- No hay motor JS maduro (zig-js está en 0.0.1; todos acaban embebiendo V8/QuickJS).
- Accesibilidad: sin equivalente a AccessKit; habría que hablar con AT-SPI/NSAccessibility a mano.
- Shaping de texto: hay que ir a HarfBuzz/FreeType por C.
- Ecosistema de paquetes mucho más joven; `build.zig.zon` sin la madurez de crates.io.
- **Ventaja real:** interop C de primera clase (v8, harfbuzz, freetype, skia, webkitgtk),
  allocators explícitos por arena (encaja con ciclos de vida de página), binarios pequeños,
  compilación rápida y control total del layout de memoria.

---

## 5. Comparativa directa para *nuestro* caso

| Criterio | Rust | Zig |
|---|---|---|
| Reutilizar motor existente | **Servo, Blitz, Gosub** | Solo Lightpanda (headless, AGPL) |
| Reutilizar parser/layout/fuentes | **Excelente** (html5ever, taffy, parley, resvg) | Malo (ports inmaduros o FFI a C) |
| Shell rápida (ventana + webview) | **Tauri/wry, webkit2gtk** | zig-webview / zig-webui (básicos) |
| Motor propio desde cero | Bueno (Rust + crates) | **Bueno también** (Turtle lo demuestra) |
| Iteración/compilación | Lenta en proyectos grandes | **Muy rápida** |
| Seguridad de memoria | **Garantizada** (sin GC) | Manual (con `defer`/allocators) |
| Empleabilidad / comunidad | **Mucho mayor** | Nicho |
| Licencias del ecosistema | MPL-2.0 / Apache-2.0 | AGPL-3.0 (Lightpanda) |

**Veredicto:** para *embeber o reutilizar*, Rust gana sin discusión.
Para *motor propio educativo* con objetivo de rendimiento y control, Zig es una elección
defendible y Turtle/Arix prueban que se puede; el coste es escribir todo.
Híbrido interesante: **motor propio en Zig + V8 por C ABI** (lo que hacen Lightpanda y Turtle),
o **UI/shell en Rust + motor propio en un crate aparte**.

---

## 6. La categoría que está explotando: navegadores headless para agentes

Ranking por estrellas (2026-11-13):

| Proyecto | ★ | Lenguaje | Naturaleza |
|---|---:|---|---|
| [vercel-labs/agent-browser](https://github.com/vercel-labs/agent-browser) | 50.4k | Rust | CLI que descarga y conduce Chrome (no es motor) |
| [lightpanda-io/browser](https://github.com/lightpanda-io/browser) | 42.0k | Zig | Motor propio sin render |
| [h4ckf0r0day/obscura](https://github.com/h4ckf0r0day/obscura) | 34.2k | Rust | Motor propio + render opcional |
| [lexmount/moli](https://github.com/lexmount/moli) | 23.6k | Rust | Motor propio + layout/paint on-demand |
| Playwright / Puppeteer | — | TS/JS | Automatización sobre motores reales |

Patrón común: **DOM + JS (V8) + CDP, sin paint**. Es ~10× más barato que Chrome y
suficiente para scraping/agentes. Si el objetivo del proyecto es ese, no hace falta
un motor gráfico: hace falta DOM correcto, V8 y CDP.

Señal de calidad: los tres "motores nuevos" (Obscura abr-2026, Moli ago-2026)
crecieron a 20–34k★ en meses y usan exactamente el mismo stack (html5ever/cssparser/
taffy/tiny-skia/V8). Es el *stack canónico* de un motor nuevo en 2026.

---

## 7. Riesgos y licencias

| Riesgo | Detalle |
|---|---|
| **MPL-2.0 (Servo)** | Copyleft a nivel de archivo: puedes enlazar y distribuir tu app, pero cambios a archivos de Servo deben publicarse. Aceptable para casi todo. |
| **AGPL-3.0 (Lightpanda)** | Si ofreces el servicio por red, debes liberar el código. Inviable para producto cerrado sin licencia comercial. |
| **Servo pre-1.0** | El propio equipo advierte: renderizar contenido *conocido/confiable* ≠ navegar contenido *arbitrario*. Sin garantía de seguridad. |
| **Verso archivado** | No hay runtime Tauri-Servo mantenido; si eliges Servo, lo embebes tú. |
| **Motores nuevos (Moli/Obscura)** | Mucho código generado rápido; 647 issues abiertos en Moli; auditabilidad y seguridad sin demostrar. |
| **WebView del SO** | Fragmentación WebKitGTK/WKWebView/WebView2; dependes del ciclo de updates del sistema. |
| **Zig 0.17** | Sin estabilidad de lenguaje garantizada; los proyectos se rompen entre versiones. |

---

## 8. Rutas posibles (esfuerzo estimado, 1 dev con agentes IA)

| Ruta | Qué es | Tiempo a "usable" | Riesgo | Cuándo elegirla |
|---|---|---|---|---|
| **A. Shell + WebView** | Tauri/wry o webkit2gtk: pestañas, URL, historial, descargas, adblock | 1–3 semanas | Bajo | Quieres un navegador *ya*, con la web real |
| **B. Embedding Servo** | `cargo add servo` + WebView API + UI propia (winit/egui/tauri) | 1–3 meses | Medio | Quieres motor único en todas las plataformas y control total |
| **C. Embedding Blitz** | `blitz-dom` + Taffy + Vello; traer tu propio JS/red | 2–6 semanas | Medio | Render de HTML/CSS en una app, no navegar la web |
| **D. Motor propio (Rust o Zig)** | Red, HTML, CSS, layout, paint, texto; JS después | 6–18 meses a "lee webs simples"; 3–5 años a "web moderna" | Alto | Objetivo de aprendizaje/investigación, no producto |
| **E. Híbrido** | Motor propio para modo lector/offline + Servo/WebView como fallback | 2–6 meses | Medio | Quieres identidad propia sin morir en el intento |

**Recomendación pragmática:** empezar por **B** (Servo embebido) si el objetivo es
"navegador de verdad con tecnología propia", o por **D** con alcance explícito de
"motor de investigación" si el objetivo es aprender. **A** solo si el objetivo es
tener producto en semanas. La peor opción es D sin recortar alcance: es donde mueren
los proyectos (Verso, Kosmonaut).

**Si el objetivo es Zig:** el camino con más probabilidad de éxito es
"**Arix + html5ever/taffy por FFI + V8 por C ABI**" (el stack de Turtle/Lightpanda),
no reimplementar JS.

---

## 9. Fuentes principales

- Servo: <https://servo.org>, <https://servo.org/wpt>, crates.io `servo`, Igalia (2026-11-11).
- Blitz: <https://blitz.is>, <https://blitz.is/wpt/focus-areas/servo> (comparativa WPT 2026-11-12).
- Ladybird: <https://ladybird.org/newsletter/2026-10-31/>, <https://ladybird.org/posts/adopting-rust/>.
- Lightpanda: <https://lightpanda.io/docs/core-concepts/architecture-overview>, <https://lightpanda.io/blog/posts/lightpanda-1-0>.
- Verso (archivado): README del repo + análisis 2026-07-17.
- Obscura / Moli: READMEs y `Cargo.toml` de sus repos.
- Turtle: <https://code.intellios.ai/cwbrowser/> + paper.
- Datos de repos: GitHub REST API, consultados 2026-11-13 (ver `datos-github-2026-11-13.tsv`).
