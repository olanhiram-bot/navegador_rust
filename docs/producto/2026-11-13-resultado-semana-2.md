# Resultado de la semana 2 — tarjeta y recibo

Fecha: 2026-11-13 · Gate del MVP

## Resultado del gate

| Criterio | Resultado |
|---|---|
| Sustitución posterior a la aprobación abortada | ✅ 10/10 |
| Recibo firmado verificable offline | ✅ `navegador verify` |
| Verificación en otro proceso, sin estado compartido | ✅ solo el JSON |
| Recibo manipulado rechazado | ✅ exit 2 |
| La tarjeta existe como ventana nativa | ✅ egui, fuera del proceso de la página |
| Suite completa | ✅ 24 tests, 0 fallos, 0 procesos huérfanos |

## La tarjeta

`crates/card` separa el modelo (`CardView`) del renderizador. Eso permite
probarla sin pantalla y usar egui solo cuando hay display.

Propiedades que se cumplen y se testean:

- **Ground truth.** Muestra el importe real, el destinatario real y el origen
  real de la acción, no la etiqueta de la página.
- **Denegar primero.** `deny_first` es siempre cierto; Escape deniega; no hay
  atajo de teclado para aprobar.
- **Contenido no confiable, texto plano.** `clean()` colapsa saltos de línea,
  elimina caracteres de control y limita a 160 caracteres. La página no puede
  añadir filas, tapar valores ni inyectar marcado.
- **Aprobación ligada al hash.** El botón de aprobar devuelve el `hash_hex` del
  descriptor; la política solo acepta ese hash exacto.

Ejemplo real contra `fixtures/lie.html`:

```
┌─ Acción bloqueada ──────────────────────────────┐
    Acción:              submit
  ▸ Importe real:        1000.00
  ▸ Destinatario real:   attacker-acct-999
  ▸ Origen de la acción: http://127.0.0.1:8911
    Documento:           http://127.0.0.1:8911
    Se enviará:          amount=1000.00 · to=attacker-acct-999
  ⚠ avisos
    · la página muestra 1.00 pero se enviarán 1000.00
    · importe real 1000.00 distinto de la intención 1.00
    · destinatario real "attacker-acct-999" distinto de la intención "ana"
  ────────────────────────────────────────────
  [ Denegar ]  (predeterminada)
```

La ventana nativa se comprobó con `hyprctl clients`: aparece como
`Navegador — confirmar acción`, en el proceso del navegador, no en el renderer
de la página.

## El recibo

`crates/receipt` firma con ed25519 un payload canónico:

```
schema · issued_at_unix · descriptor · wire{url,method,body} · outcome · public_key
```

`navegador verify` comprueba, en este orden:

1. esquema;
2. firma sobre el JSON canónico;
3. que el descriptor coincide con su propio hash;
4. que la petición de `wire` coincide con los campos del descriptor.

Salida real:

```
RECIBO VÁLIDO
  emitido:      1794550658
  clave:        da65a4b286a9ba07bab71bb4317f35527a02419accfa6d3cb3c1ec5b8eb7fe10
  acción:       submit
  importe:      1.00
  destinatario: ana
  origen:       http://127.0.0.1:8911
  petición:     POST http://127.0.0.1:8911/pay
  hash:         904d97ce94f3c679b80c117da16b612b5523ab3b586fc4371ff3b0e7f3bf81f2
```

Manipular el cuerpo de la petición produce `RECIBO RECHAZADO: firma inválida`
y exit 2.

### Modelo de confianza (importante)

El recibo es autocontenido: lleva la clave pública dentro. Eso prueba
integridad y binding, **no identidad**. Para saber que lo emitió *tu* navegador
hay que comparar `public_key` con la clave que ese navegador te mostró antes.
Es el paso que faltaría para un despliegue real y está anotado como límite.

## Cambios técnicos que aparecieron al implementar

1. **`Fetch` debe desactivarse antes de navegar.** Si queda armado, el
   documento siguiente se pausa y `Page.navigate` nunca responde. Ahora
   `navigate()` desactiva la interceptación y `dispatch()` la arma justo antes
   del click: la ventana de intercepción es mínima.
2. **Chromium deja huérfanos si matas el proceso padre.** El motor ahora lanza
   Chromium en su propio grupo de procesos y lo mata completo en `Drop`.
   Además, el CLI ya no usa `process::exit` en medio del flujo (saltaba los
   destructores): devuelve `ExitCode`.
3. **eframe 0.37 cambió la API.** El trait `App` ahora implementa
   `fn ui(&mut self, ui: &mut Ui, frame: &mut Frame)`, no `update(ctx, frame)`.

## Cómo probarlo a mano

```bash
cargo test --workspace            # incluye el gate 10/10 y los recibos

cargo run -p navegador -- keygen  # semilla + clave pública
cargo run -p navegador --features gui -- \
  run https://ejemplo.com "#pagar" 10.00 ana --gui
cargo run -p navegador -- verify recibo.json
```

Los fixtures y el sink cross-origin viven en `crates/navegador/src/server.rs` y
los usan los tests; no hay comando `serve` todavía.

## Límites que siguen abiertos

- El heurístico de campo (`amount`, `to`, …) no cubre nombres opacos.
- Solo importes con `$`, `€`, `£`.
- Iframes cross-origin siguen sin adjuntarse.
- La tarjeta se muestra cuando hay aprobación; una acción denegada solo imprime
  el texto. En la fase 3 conviene mostrarla también, como evidencia.
- El recibo no está encadenado (no hay log append-only todavía).

## Siguiente

Fase 3: `SessionCore` + refs `@eN` estables + servidor MCP, con todo `act`
pasando por descriptor → política → tarjeta → dispatch → recibo.
