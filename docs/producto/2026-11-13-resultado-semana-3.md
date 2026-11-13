# Resultado de la semana 3 — SessionCore + MCP

Fecha: 2026-11-13 · Gate del MVP

## Resultado del gate

| Criterio | Resultado |
|---|---|
| Un host MCP completa un flujo de 3 pasos | ✅ `open` → `snapshot` → `act` por stdio |
| 100% de acciones consecuentes con recibo | ✅ una acción ejecutada, un recibo `seq 0` |
| El recibo está en el log encadenado | ✅ `receipt::verify_file` = 1, `prev = genesis` |
| La petición aprobada sale una sola vez | ✅ un POST `to=ana&amount=1.00` |
| La aprobación está atada al hash | ✅ segundo `act` con `approve` |
| Las refs caducan con la generación | ✅ tras ejecutar, generación 2 |

El gate vive en `crates/mcp/tests/gate.rs` y habla JSON-RPC de verdad con el
binario `navegador-mcp` (stdin/stdout), contra Chromium y los fixtures.

## SessionCore

`crates/session` es la única puerta entre agente y navegador. Toda acción
consecuente sigue la misma cadena:

```
ref @eN → descriptor ground-truth → política → tarjeta → dispatch → recibo
```

Decisiones que quedan en el tipo, no en la disciplina del llamante:

- **`act` no acepta un selector ni una coordenada**, solo una ref `@eN` de un
  snapshot vivo. Antes de despachar, el motor re-extrae el descriptor y se
  comprueba que el elemento sigue siendo el mismo (tag, nombre y posición con
  tolerancia de 2 px). Si la página lo reescribió, la ref es obsoleta.
- **La generación se pasa en `act`** (opcional). Si el host dice `generation: 1`
  y la sesión va por la 2, se rechaza: sin eso, `@e1` de una generación vieja
  apuntaría silenciosamente al `@e1` de la nueva.
- **Cualquier intento de despacho invalida el snapshot.** La página puede haber
  navegado o cambiado el formulario; el host pide uno nuevo.
- **Solo `Sent` produce recibo.** El recibo se firma, se verifica antes de
  anexarlo y se guarda en el log encadenado. Si la firma no verifica, no se
  escribe evidencia rota: el error sube. Un `Blocked` no deja recibo porque no
  hubo acción consecuente.
- **La tarjeta se construye también cuando se deniega**, para que quede la
  evidencia de qué se intentó (límite anotado en la semana 2).

La política de la sesión fija `provenance`, `max_amount` y `require_approval`;
el importe y el destinatario autorizados se pasan en cada `act`. Los orígenes
permitidos son los documentos visitados en la sesión más los que se configuren
explícitamente; el exfiltrador sigue denegándose porque su acción sale a otro
origen.

## El servidor MCP

`crates/mcp` implementa el subconjunto necesario de MCP por stdio (JSON-RPC
delimitado por líneas): `initialize`, `ping`, `tools/list`, `tools/call` y
notificaciones. Seis herramientas:

| Tool | Qué hace |
|---|---|
| `open` | navega y devuelve el snapshot con refs `@eN` |
| `snapshot` | refresca las refs sin cambiar de generación |
| `act` | descriptor → política → tarjeta → dispatch → recibo |
| `read` | texto visible, sin interpretar |
| `screenshot` | PNG como contenido de imagen MCP |
| `handoff` | URL, generación, elementos, recibos y ruta del log |

`act` es deliberadamente de dos fases. La primera llamada devuelve
`outcome: "approval_required"` con el `hash` exacto y la tarjeta; la segunda,
con `approve: "<hash>"`, ejecuta. Si la página cambió entre ambas, el hash ya no
cubre el descriptor y la política vuelve a pedir aprobación. Es la misma
propiedad que la tarjeta nativa, expuesta al host.

Ejemplo de respuesta ejecutada:

```json
{
  "outcome": "executed",
  "receipt_seq": 0,
  "hash": "…",
  "public_key": "…",
  "wire": { "url": "http://127.0.0.1:PORT/pay", "method": "POST", "body": "amount=1.00&to=ana" },
  "card": "┌─ Confirmar acción …"
}
```

## Cómo probarlo

```bash
cargo test --workspace --all-targets   # incluye el gate MCP (requiere Chromium)

# el servidor MCP habla por stdio; el log por defecto es ./navegador-recibos.jsonl
cargo run -p mcp -- --log /tmp/recibos.jsonl --max-amount 20
```

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"open","arguments":{"url":"https://ejemplo.com"}}}
```

## Cambios técnicos que aparecieron al implementar

1. **`Intent.verb` pasó a `Option<Verb>`** en el commit anterior y dejó los
   tests de `descriptor`, `card`, `navegador` y `main.rs` sin compilar. El
   workspace no cargaba por `crates/mcp`/`crates/session` a medias; ambos
   arreglos entran aquí.
2. **`Chrome::read_text` devolvía texto plano y lo parseaba como JSON.** Ahora
   la expresión JS devuelve `JSON.stringify(...)`, igual que `extract` y
   `snapshot`. El bug estaba oculto porque `read` no existía antes de MCP.
3. **`ReceiptLog` no exponía su ruta**; `Session::handoff` la necesita para que
   la persona pueda auditar el log.

## Límites que siguen abiertos

- El hash de aprobación lo transporta el host: MCP no puede probar que fue una
  persona quien lo aprobó. El recibo prueba integridad y binding, no identidad;
  el paso que falta sigue siendo fijar la clave pública del navegador.
- iframes cross-origin siguen sin adjuntarse.
- El heurístico de campo (`amount`, `to`, …) no cubre nombres opacos.
- El servidor MCP abre una sola sesión y una sola pestaña.
- No hay cancelación: un `dispatch` bloqueado espera hasta su deadline.

## Siguiente

Fase 4: perfiles efímeros, sets de orígenes read-only/read-write, egress
policy y 3 escenarios reales con métricas de falsos bloqueos.
