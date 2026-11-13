# Plan de arranque del MVP — tras verificar los supuestos

Fecha: 2026-11-13 · Revisa `2026-11-13-tesis-producto.md` · Evidencia reproducible en `/tmp/nbresearch/mvp`

---

## 1. Qué verifiqué y qué salió

| Supuesto | Resultado | Evidencia |
|---|---|---|
| BrowserOS no tiene gating tipo VAC | **Confirmado, con matices** | Tiene `cdp_policy.rs` (allowlist explícita de métodos/parámetros CDP), fence de contenido no confiable en el prompt y audit/replay local completo. No tiene descriptor ground-truth, ni Action Card, ni binding de aprobación. |
| VAC es portable fuera de Chromium "de verdad" | **Sí** | La implementación de referencia es *"an Electron/Chromium shell driven by a Python agent over CDP"*, con flags de configuración. Es exactamente nuestra arquitectura. |
| CDP expone el ground truth necesario | **Sí, y con un hallazgo crítico** | Fixture con etiqueta visible "$1.00" y `<input type=hidden value="1000.00">`: el descriptor lee `1000.00`, `form.action=https://evil.example/collect` y detecta el desajuste. **El árbol de accesibilidad solo mostró el texto que miente** ("Aprobar pago de $1.00"). |
| Se puede atar la aprobación a lo que se ejecuta | **Sí** | Click real vía `Input.dispatchMouseEvent` → `POST /pay` con `postData="to=ana&amount=1.00"`. Cambiar el destino antes de enviar cambia el hash y el `postData`. `Fetch` permite pausar/abortar. |

### Consecuencia 1: el árbol de accesibilidad no es ground truth

Un agente que decide solo con AX es exactamente el que cae en *Lies-in-the-Loop*.
El descriptor debe construirse desde **DOM + estado de formulario + origen resuelto**,
no desde nombres accesibles. Esto también mejora al agente: deja de creerle a la etiqueta.

### Consecuencia 2: "recibos" ya no es el diferenciador

BrowserOS graba sesiones completas, con replay de DOM real y screenshots, en local.
Nuestro recibo tiene que ser otra cosa: **prueba criptográfica de que la acción ejecutada
es la aprobada**. La grabación es tabla de entrada; el binding no lo es.

### Consecuencia 3: el hueco se estrecha, pero sigue abierto

Nos quedan tres cosas que nadie tiene juntas:

1. Descriptor ground-truth + Action Card fuera del alcance de la página.
2. Binding de aprobación a la petición realmente despachada (TOCTOU).
3. Recibo firmado y verificable offline.

Y VAC dejó explícitamente pendiente la evaluación de *action substitution* bajo mutación
del DOM. Ese es nuestro terreno.

---

## 2. Cómo empezar: por el riesgo, no por las capas

No empezamos por el core ni por el MCP. Empezamos por el **descriptor y el arnés de ataques**,
porque si eso no funciona, no hay producto y hay que replantear antes de escribir 6 semanas
de infraestructura.

### Semana 1 — Descriptor + arnés (sin UI, sin MCP)

Objetivo: una CLI que abre un fixture, ejecuta la acción y emite un descriptor canónico,
con los tres ataques detectados en CI.

```
crates/descriptor/
  src/lib.rs        # ActionDescriptor, canonical_json, hash
  src/extract.rs    # CDP -> DOM/form/origen resuelto
  src/provenance.rs # user | plan | page
  tests/attacks.rs  # los 3 escenarios
fixtures/
  01-exfil.html
  02-lie-in-the-loop.html
  03-substitution.html
```

Descriptor v0 (basado en VAC + lo que CDP realmente da):

```rust
struct ActionDescriptor {
    verb: Verb,                 // Click | Submit | Type | Navigate | Download
    target: Target,             // backend_node_id, role, name, box
    document_origin: String,    // location.origin
    action_origin: Option<String>, // origen resuelto de form.action / href
    method: Option<String>,     // GET | POST
    recipient: Option<String>,  // campos to/recipient/iban/email
    amount: Option<f64>,        // campos numéricos de pago
    file: Option<String>,       // input[type=file]
    provenance: Provenance,     // de dónde salió cada parámetro material
    generation: u32,            // se invalida en navegación/DOM.documentUpdated
    hash: [u8; 32],             // sha256 del JSON canónico
}
```

**Gate de la semana 1**: los 3 fixtures se detectan 10/10, y una tarea legítima pasa 10/10.

### Semana 2 — Action Card + binding + recibo

- Ventana mínima (`egui`, después GPUI) que renderiza la tarjeta **fuera del proceso de la página**:
  Deny como acción dominante, valores escapados, banner si `action_origin != document_origin`
  o si `provenance == page`.
- Aprobación ligada a `hash`; al despachar se re-lee el descriptor y se compara.
- Defensa en profundidad: observar `Network.requestWillBeSent` y comparar `postData` con lo aprobado.
- Recibo: JSON firmado (`ed25519-dalek`) + `navegador verify receipt.json` offline.

**Gate de la semana 2**: sustitución post-aprobación abortada 10/10 y recibo verificado en otra máquina.

### Semanas 3–4 — SessionCore + MCP

- `Engine` trait, adaptador `engine-cdp`, `snapshot` con refs estables (`@eN` → `backendDOMNodeId` + generación).
- Tools MCP: `open`, `snapshot`, `act`, `read`, `screenshot`, `handoff`.
- `act` pasa **siempre** por descriptor → policy → card (si es consecuente) → dispatch → binding.

**Gate**: un host MCP completa un flujo de 3 pasos y el 100% de las acciones consecuentes
quedan con descriptor y recibo.

### Semanas 5–6 — Endurecer

- Perfiles efímeros, sets de orígenes read-only/read-write, límites de egress.
- 3 escenarios reales (no fixtures) + métricas de falsos bloqueos.
- CI con los ataques como tests de regresión.

---

## 3. Decisiones técnicas tomadas (y por qué)

| Decisión | Elección | Motivo |
|---|---|---|
| Cliente CDP | `chromiumoxide` para ciclo de vida; **CDP crudo** (`tokio-tungstenite`) para eventos finos | El spike se hizo con CDP crudo en ~80 líneas; `Fetch`/`Network` necesitan control total |
| Ground truth | DOM + `Runtime.callFunctionOn` sobre el nodo resuelto | AX no ve valores reales (probado) |
| Binding | re-lectura + hash **antes** de despachar, y `postData` **después** | Cubre TOCTOU y deja evidencia |
| UI de la tarjeta | `egui` primero, GPUI después | La tarjeta debe existir el día 15, no el día 60 |
| Firma | `ed25519-dalek` | Verificación offline, sin infraestructura |
| Motor | Chromium (CDP) | VAC de referencia ya lo usa así; Servo queda para fase 3 |

**Aviso técnico del spike**: interceptar la navegación principal con `Fetch` antes de
`Page.navigate` cuelga la sesión (el renderer queda esperando red). Habilitar `Fetch`
**después** de cargar, o usar `Network` + nuestro propio ejecutor. Los iframes cross-origin
van a proceso aparte: los eventos no llegan al target de la página.

---

## 4. Qué no se construye en el MVP

- Motor propio, Servo, CEF, GPUI, pestañas, historial, extensiones, sync, multi-ventana.
- Detección de prompt injection por modelo (es probabilística; la sustituimos por arquitectura).
- Antidetect, CAPTCHA, proxies, cloud.

---

## 5. Criterio para seguir o parar

Después de la semana 2:

- **Seguir** si 3/3 ataques bloqueados, 0 falsos bloqueos en 10 tareas legítimas y recibo verificable.
- **Replantear** si la tarjeta no puede renderizarse fuera del alcance de la página (con una
  ventana nativa separada debería ser trivial, pero se comprueba con código) o si el descriptor
  no puede formarse de forma fiable en sitios reales.
