# Resultado de la semana 1 — descriptor + arnés

Fecha: 2026-11-13 · Gate del MVP

## Resultado del gate

| Criterio | Resultado |
|---|---|
| 3/3 ataques detectados | ✅ denegados o abortados antes de salir |
| 0 falsos bloqueos en la tarea legítima | ✅ se aprueba y se ejecuta |
| Todo en CI | ✅ `.github/workflows/ci.yml` |
| Estabilidad | ✅ 3 corridas consecutivas, 12 tests, ~1.2 s |

```
test ataque_1_confused_deputy_se_deniega ... ok
test ataque_2_lies_in_the_loop_se_deniega ... ok
test ataque_3_sustitucion_se_aborta_antes_de_salir ... ok
test tarea_legitima_se_aprueba_y_se_ejecuta ... ok
```

## Qué hace cada ataque y qué lo detiene

### 1. Confused deputy (`fixtures/exfil.html`)

El texto visible dice "Pagar $1.00", el formulario apunta a otro origen
(`{{SINK}}/collect`) y el destinatario real es `attacker-acct-999`.

Lo detiene la política: `action_origin` no está en los orígenes permitidos y el
destinatario real no coincide con la intención. Se deniega **antes** de hacer
click, así que no hay petición.

### 2. Lies-in-the-loop (`fixtures/lie.html`)

El texto visible dice "Enviar $1.00 a Ana"; el `<input type=hidden>` lleva
`1000.00` y el destinatario real es `attacker-acct-999`.

Lo detienen dos cosas:

- el descriptor lee el valor real (`1000.00`), no la etiqueta;
- el chequeo de consistencia marca `VisibleAmountMismatch` (`$1.00` visible vs
  `1000.00` real).

La política deniega porque el importe real no coincide con la intención
autorizada.

### 3. Sustitución en el último instante (`fixtures/substitute.html`)

En el momento de aprobar, todo es legítimo: mismo origen, `to=ana`,
`amount=1.00`, etiqueta coherente. Al hacer click, un listener reescribe
`form.action`, el destinatario y el importe justo antes de serializar.

Lo detiene el **binding de ejecución**:

1. el motor re-extrae el descriptor y compara el hash con el aprobado;
2. hace un click real (`Input.dispatchMouseEvent`);
3. `Fetch.requestPaused` intercepta el POST **antes de que salga**;
4. `verify_wire` compara URL, método y campos con lo aprobado;
5. al no coincidir, `Fetch.failRequest` lo aborta.

El sink no recibe nada. Esto es lo que ningún grabador de sesiones puede dar:
prueba de que la acción ejecutada es la aprobada.

### Tarea legítima (`fixtures/legit.html`)

Mismo origen, `to=ana`, `amount=1.00`, etiqueta coherente. La política pide
aprobación (hash), la persona aprueba ese hash, el binding deja salir el POST y
el servidor recibe exactamente `to=ana&amount=1.00`.

## Hallazgo técnico que corrigió el plan

En el spike anterior, `Fetch` no interceptaba el envío del formulario cuando se
disparaba con `requestSubmit()` desde JavaScript. Con el flujo real del
ejecutor —`Fetch.enable` después de cargar + `Input.dispatchMouseEvent`— **sí lo
intercepta**. Consecuencia: el binding puede ser *pre-send* (abortar antes de
que la petición salga), no solo *post-send* (detectar y lamentar).

Sigue siendo cierto que habilitar `Fetch` antes de `Page.navigate` cuelga la
navegación. El orden correcto es: cargar → habilitar `Fetch` → click.

## Mapa del código

| Archivo | Responsabilidad |
|---|---|
| `crates/descriptor/src/lib.rs` | `ActionDescriptor`, JSON canónico + sha256, `Intent::evaluate`, `verify_wire`, importes visibles |
| `crates/engine-cdp/src/cdp.rs` | Cliente CDP sobre WebSocket (llamadas + eventos) |
| `crates/engine-cdp/src/chrome.rs` | Lanzar Chromium, navegar, extraer, despachar con binding |
| `crates/engine-cdp/src/extract.js` | Lectura del DOM en mundo aislado |
| `crates/navegador/src/server.rs` | Servidor de fixtures + sink cross-origin |
| `crates/navegador/tests/attacks.rs` | Arnés de los 3 ataques + tarea legítima |
| `crates/navegador/src/main.rs` | CLI `navegador audit` |

## Límites conocidos (siguen abiertos)

- El heurístico de importe/destinatario usa nombres de campo (`amount`, `to`,
  `recipient`, …). Sitios reales con nombres opacos necesitan refs semánticas.
- El chequeo de importe visible solo entiende `$`, `€` y `£`.
- El binding cubre la petición del documento principal; los iframes
  cross-origin van a otro target y aún no se adjuntan.
- No hay tarjeta visual ni recibo firmado: eso es la semana 2.

## Siguiente

Semana 2: Action Card en ventana nativa fuera del alcance de la página, Deny como
acción dominante, y recibo `ed25519` verificable offline.
