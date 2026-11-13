# ADR-001: Navegador para agentes y personas

Estado: propuesta
Fecha: 2026-11-12
Contexto previo: `docs/investigacion/2026-11-12-navegadores-rust-zig.md`

---

## 1. La tesis

**No competimos por el motor. Competimos por la sesión.**

Un navegador "para agentes y personas" no es un navegador con un botón de IA.
Es un navegador donde **la misma sesión viva** puede ser conducida por un agente
y observada/intervenida por un humano, sin recargar, sin duplicar estado y sin
perder cookies, scroll ni formularios a medio llenar.

Eso obliga a tres decisiones estructurales:

1. **Un core de sesión agnóstico del motor** (navegar, snapshot, input, screenshot, attach view).
2. **Dos superficies sobre el mismo core**: UI humana y protocolo de agente.
3. **Paint bajo demanda**: el agente no necesita píxeles; la persona sí. El motor
   siempre construye DOM y layout; el compositor solo se activa cuando hay una
   vista adjunta o se pide screenshot.

El punto 3 es lo que hace viable económicamente el modo agente (decenas de MB por
sesión en vez de cientos) sin renunciar al modo humano. Es exactamente lo que
Moli vende como "structure first, pixels on demand", pero aplicado a las dos caras.

---

## 2. Decisión

**Shell nativo en Rust + abstracción de motor + dos motores secuenciados.**

| Fase | Motor por defecto | Por qué |
|---|---|---|
| **1. Producto (0–3 meses)** | **Chromium vía CDP** | Compatibilidad real: si una persona no puede entrar a su banco, no hay producto. CDP ya es el estándar de agentes (Playwright/Puppeteer). |
| **2. Diferenciación (3–9 meses)** | **Servo** para sesiones de agente | Sin paint, ~1 binario, Rust, sin tax de Chromium; WPT 75.8% es suficiente para la mayoría de tareas de extracción/automatización. |
| **3. Madurez (9–18 meses)** | Datos → ¿Servo también para personas? | Solo si la compatibilidad medida lo permite. Si no, el estado final es legítimo: Chromium para humanos, Servo para agentes, una sola UI. |

Implementación: un trait interno `Engine` con dos adaptadores.

```rust
trait Engine {
    fn create_session(&self, profile: Profile) -> SessionId;
    fn navigate(&self, s: SessionId, url: Url) -> Result<()>;
    fn snapshot(&self, s: SessionId, mode: SnapshotMode) -> Snapshot; // a11y | dom | text
    fn input(&self, s: SessionId, action: InputAction) -> Result<()>;
    fn screenshot(&self, s: SessionId, opts: ShotOpts) -> Result<Image>; // fuerza paint
    fn attach_view(&self, s: SessionId, surface: SurfaceId) -> Result<()>; // enciende paint
}
```

Regla dura: **ninguna feature de producto puede depender de una API específica de
Chromium o de Servo**. Si la necesita, se modela en el trait o no entra.

### Por qué no un solo motor desde el día 1

- **Solo Chromium** → no hay diferenciación técnica ni control de coste por sesión;
  compites con Chrome + Playwright, que ya existen y son gratis.
- **Solo Servo** → 75.8% WPT y pre-1.0: demasiados sitios rotos para "personas".
  Servo mismo advierte que embeber contenido confiable ≠ navegar contenido arbitrario.
- **Dos motores desde el día 1** → duplica el coste de ingeniería antes de tener usuario.

La secuencia captura el valor de ambos sin pagar los dos costes a la vez.

---

## 3. Arquitectura

```
┌──────────────────────────────────────────────────────────────┐
│  Superficies                                                  │
│  ┌────────────────────┐        ┌───────────────────────────┐  │
│  │ UI humana          │        │ Protocolo de agente       │  │
│  │ winit + egui/Slint │        │ MCP + CDP + HTTP local    │  │
│  └─────────┬──────────┘        └─────────────┬─────────────┘  │
└────────────┼─────────────────────────────────┼────────────────┘
             │                                 │
      ┌──────▼─────────────────────────────────▼──────┐
      │  Session Core (Rust)                          │
      │  · ciclo de vida, perfiles, historial         │
      │  · snapshot semántico con refs estables       │
      │  · cola de acciones + action log (replay)     │
      │  · policy engine (qué puede hacer un agente)  │
      │  · attach/detach de vista (paint on demand)   │
      └──────┬─────────────────────────┬──────────────┘
             │                         │
      ┌──────▼────────┐        ┌───────▼──────────┐
      │ Chromium      │        │ Servo            │
      │ adaptador CDP │        │ adaptador WebView│
      │ (humano+agente│        │ (agente / ligero)│
      └───────────────┘        └──────────────────┘
```

### Capas y crates

| Capa | Elección | Notas |
|---|---|---|
| Ventana/input | `gpui` + `gpui-component` (candidato) o `winit`+`egui` | ver §10 |
| Motor compat | `chromiumoxide` (CDP) o CEF | CEF si quieres control de render embebido; CDP si puedes depender de Chrome/Chromium |
| Motor agente | `servo 0.8` (WebView API) | mismo `servo` que ya está en crates.io |
| Protocolo agente | `rmcp` (MCP) + fachada CDP | MCP para hosts LLM; CDP para Playwright/Puppeteer |
| Snapshot semántico | AccessKit (Servo) / CDP `Accessibility` (Chromium) → esquema propio | refs `@e1..n` estables por navegación |
| Persistencia | `rusqlite` | perfiles, historial, logs de acciones, caché de red |
| Red | `reqwest`/`hyper` + `hickory-resolver` | para el modo Servo y para políticas de egress |
| Cifrado/credenciales | `rustls` + keyring del SO | vault ligado a origen |

### Superficie de agente (lo que realmente se usa)

No expongas solo CDP. Un agente necesita **verbos semánticos**:

```json
{"tool":"snapshot","filter":"interactive","format":"tree"}
→ [{"ref":"@e1","role":"textbox","name":"Email"},
   {"ref":"@e2","role":"button","name":"Continue"}]

{"tool":"act","ref":"@e2","action":"click"}
{"tool":"read","format":"markdown"}          // página como texto
{"tool":"screenshot","full_page":true}       // fuerza paint
{"tool":"wait","until":{"text":"Inbox"}}
{"tool":"handoff","reason":"login_required"} // pide humano en la misma sesión
```

- **Refs estables** por documento, invalidados en navegación/`pushState`.
- **Action log** append-only → replay, auditoría y tests deterministas.
- **Diff de árbol** entre pasos: el agente no relee 50k nodos, lee el delta.
- **Handoff** es la killer feature: el agente se detiene, el humano resuelve el
  login/2FA/CAPTCHA en la misma sesión, y el agente continúa. Ningún competidor
  headless puede hacerlo.

---

## 4. Seguridad (el riesgo real)

Un navegador de agentes es una **ejecución de instrucciones no confiables con
credenciales del usuario**. Diseño mínimo:

1. **Contenido de página = datos, nunca instrucciones.** El prompt del agente va por
   un canal separado; el texto de la web se etiqueta como untrusted en cada paso.
2. **Policy engine por sesión**: `read-only`, `click`, `type`, `purchase`.
   Acciones irreversibles requieren confirmación humana explícita.
3. **Perfiles efímeros por defecto** en sesiones de agente (cero estado compartido),
   y perfiles persistentes solo con consentimiento.
4. **Vault de credenciales ligado a origen**: nunca autofill a un origen distinto
   del que guardó el secreto; el agente no puede leer el valor, solo usarlo.
5. **Egress policy**: allowlist/denylist de dominios, bloqueo de `file://` y de
   metadata de cloud, límite de descargas.
6. **Aislamiento de proceso** por sesión (Chromium ya lo hace; Servo habrá que
   añadirlo) y límites de memoria/CPU.
7. **Auditoría**: cada acción con timestamp, origen, ref y captura del árbol previo.

---

## 5. Roadmap con hitos medibles

### Fase 1 — "Navegador usable" (4–6 semanas)

- Ventana con pestañas, URL bar, atrás/adelante/recargar, historial.
- Motor: Chromium por CDP.
- Sesión de agente por MCP: `open`, `snapshot`, `act`, `read`, `screenshot`.
- Perfiles (normal + efímero), descargas, find-in-page.
- **Hito:** un agente completa "buscar un vuelo y devolver precio/hora" en ≥80% de
  intentos; un humano navega 20 sitios top sin romper nada.

### Fase 2 — "Sesión compartida" (6–10 semanas)

- `attach_view` / `detach_view`: la misma sesión cambia entre agente y humano.
- Handoff con pausa/reanudación y notificación en UI.
- Action log + replay; grabación de sesión.
- Policy engine + confirmaciones para acciones irreversibles.
- **Hito:** 100% de handoffs reanudan sin perder estado; replay reproduce ≥95% de
  sesiones grabadas.

### Fase 3 — "Servo para agentes" (8–12 semanas)

- Adaptador Servo detrás del mismo trait.
- Sesiones de agente sin paint; medir MB/sesión y latencia de snapshot.
- Fallback automático a Chromium por lista de features no soportadas.
- **Hito:** coste por sesión de agente ≤1/5 del de Chromium; ≥70% de las tareas
  de la fase 1 completadas en Servo sin fallback.

### Fase 4 — "Producto" (continuo)

- Actualizaciones firmadas, telemetría opt-in, extensibilidad (skills/tools de agente).
- Decidir motor por defecto con datos de la fase 3.
- **Hito:** retención semanal >30% en humanos; >50% de sesiones de agente sin
  intervención humana.

---

## 6. Métricas que gobiernan las decisiones

| Métrica | Objetivo fase 3 |
|---|---|
| MB por sesión de agente | ≤40 MB |
| Tiempo a primer snapshot | ≤300 ms |
| Tareas completadas (suite tipo WebVoyager/WebArena) | ≥70% |
| Handoffs que reanudan sin pérdida | 100% |
| Fallback Servo→Chromium | ≤30% |
| Coste de infra por 1k sesiones/h | 5× mejor que Chrome headless |

---

## 7. No-objetivos (explícitos)

- **No** escribir un motor de render propio. Ni Rust ni Zig. No es el moat.
- **No** forkear Chromium. Es un compromiso de años y no se puede mantener solo.
- **No** competir en WPT con Chrome.
- **No** hacer CDP la única API pública: CDP es un detalle de implementación.
- **No** extensiones de Chrome en v1.
- **No** modo incógnito "de verdad" como promesa de marketing sin auditoría.

---

## 8. Alternativas descartadas

| Alternativa | Por qué no |
|---|---|
| Tauri/wry como producto | WebView del SO: sin CDP, sin headless, 3 motores distintos; sirve para apps, no para un navegador. |
| Blitz como motor principal | Sin JS ni red; 10.9% WPT. Excelente como renderer embebido, no como navegador. |
| Lightpanda como base | AGPL-3.0 y sin paint: te bloquea el modo humano y el modelo de negocio. |
| Motor propio | 2 años para "70% de webs" (Turtle). Correcto como investigación, no como producto. |
| Electron | Es Chrome con esteroides de memoria; no hay control de sesión ni de paint. |

---

## 9. Riesgos principales

| Riesgo | Mitigación |
|---|---|
| Prompt injection desde páginas | contenido untrusted + policy engine + confirmaciones |
| Chromium: tamaño, codecs, updates | CEF/Chromium estable + pipeline de updates firmados; codecs opcionales |
| Servo pre-1.0 e inseguro para contenido arbitrario | usarlo solo en sesiones de agente con egress policy; fallback a Chromium |
| Doble mantenimiento de motores | trait mínimo + suite de conformidad compartida (misma batería de tareas) |
| "Otro navegador más" | foco en sesión compartida y handoff, no en features de navegador |
| Coste de UI nativa | empezar con egui/Slint; no bloquear en pixel-perfect |

---

## 10. Addendum (2026-11-13): ¿y GPUI?

GPUI pasó de "el framework interno de Zed" a una opción real de producto.

| Pieza | Versión | Estado |
|---|---|---|
| `gpui` (Zed, upstream) | 0.2.2 (2026-10-30) | Publicado en crates.io, **Apache-2.0**, pre-1.0 con breaking changes declarados |
| `gpui-component` / `gpui-kit` (Longbridge) | 0.9.1 | ~20.5k★, 557k descargas: tabs, dock, tablas, editor, temas |
| `gpui-fast` (Longbridge) | 0.1.14 | Composición de vistas nativas: WKWebView, WebView2 y **WebKitGTK vía child window X11 o subsurface Wayland** |
| `gpui_interop` (bite-gpui) | 0.2.1 | Superficies GPU externas zero-copy (Vulkan dma-buf en Linux); experimental, ~1k descargas |
| `gpui-ce` (Community Edition) | 0.3.0 | Fork comunitario con soporte de browser |

### Por qué encaja en un navegador

1. **Es el único toolkit Rust con UI de calidad tipo Zed** (texto, IME, dock, command palette).
2. **GPU-first**: en Linux usa wgpu; el contenido web ya viene como textura o vista nativa.
3. **`gpui-fast` resuelve justo el problema difícil**: meter WebKitGTK dentro de la ventana
   respetando overlays, menús y sombras de GPUI.
4. **`gpui_interop` abre la puerta a CEF/Servo por textura compartida** (dma-buf), que es
   el camino para "una sola sesión Chromium" con UI propia.

### Por qué no lo pondría como cimiento hoy

- **Pre-1.0 con breaking changes explícitos**; fijar versión y aislarlo en `crates/app`.
- **Accesibilidad débil**: GPUI exige que cada control aporte su semántica a11y; un
  navegador necesita lector de pantalla. Riesgo de producto, no técnico.
- **Un solo proveedor** (Zed) + forks (gpui-ce, bite-gpui) para funciones clave.
- **No decide el motor**: sigue siendo el problema de siempre.

### Las dos arquitecturas posibles de contenido

| Variante | Contenido | Ventaja | Coste |
|---|---|---|---|
| **A. Rápida** | `gpui-fast` + WebKitGTK | Días; motor del SO; overlay GPUI encima | WebKitGTK ≠ Chromium; handoff live solo vía mismo motor |
| **B. Coherente** | CEF off-screen → `gpui_interop` → GPU de GPUI + CDP | Un solo Chromium para humano y agente; sesión compartida real | CEF OSR + input forwarding + textura compartida; semanas y fragilidad |

Recomendación: **A para v1** (validar producto), **B como apuesta de plataforma** si el
handoff live demuestra ser la killer feature.

### Regla

`crates/app` no puede ser dueño de la lógica. La shell consume `SessionCore` por su trait;
si mañana se cambia GPUI por egui/Slint, el producto sigue funcionando.
