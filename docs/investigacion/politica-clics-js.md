# Política para clics que solo ejecutan JavaScript

Estado: decisión pendiente. Hoy la política los **deniega**.

## Problema

Un botón como `<button onclick="cargarMas()">` no declara formulario ni
destino. El descriptor no puede resolver el origen real de la acción y la
política responde:

```
Deny { reasons: ["no se pudo resolver el origen real de la acción"] }
```

Consecuencia: «cargar más», toggles, menús y buena parte de las SPA no
funcionan. El navegador es más seguro, pero deja de ser compatible.

## Opciones

### 1. Mantener la denegación

- A favor: nunca sale una petición sin aprobación.
- En contra: rompe sitios; el usuario tiene que abrirlos fuera.

### 2. Permitir el clic y auditar después

- Se ejecuta el clic, se observan las peticiones y se firma un recibo de lo que
  salió, avisando en la barra de estado.
- A favor: compatible.
- En contra: la petición salió **antes** de que la persona la viera; el recibo
  es post-hoc.

### 3. Clic, retención y aprobación (recomendada)

1. Se arma `Fetch.enable` y se dispara el clic.
2. Si en ~1,5 s no hay ninguna petición, era una interacción local: se apaga
   `Fetch` y se avisa «interacción local, sin recibo».
3. Si hay una petición, **se queda retenida** en CDP (`Fetch.requestPaused`).
   Con la petición retenida se construye la tarjeta: destino, método, cuerpo y
   origen real.
4. Al aprobar se continúa la petición y se firma el recibo con el cuerpo
   observado. Al denegar se aborta con `Fetch.failRequest`.
5. Si el clic dispara más de una petición, se retienen todas y la aprobación
   cubre el conjunto; cualquier petición que no coincida con lo aprobado se
   aborta.

A favor: compatible y con aprobación **antes** de que salga nada.
En contra: hay que ampliar la máquina de estados de la sesión (petición
retenida, aprobación de un conjunto, limpieza si la pestaña navega o se cierra).

## Diseño técnico

- `engine-cdp`: `interact(&descriptor) -> Interaction` que hace clic con Fetch
  armado y devuelve `Local` o `Held { requests: Vec<HeldRequest> }`.
- `engine-cdp`: `release(requests, allow)` que continúa o aborta y apaga Fetch.
- `session`: estado `held: Option<HeldAction>`; `act` devuelve
  `ApprovalRequired` con un descriptor construido desde la petición observada;
  `approve_held(hash)` y `deny_held()` cierran el ciclo.
- `shell`: la tarjeta ya existe; solo cambia de dónde viene el descriptor.

## Criterios de aceptación

- Un botón con `onclick` que hace `fetch("/api", {method: "POST"})` no envía nada
  antes de aprobar; al aprobar llega el cuerpo exacto y queda recibo.
- Un botón que solo cambia el DOM funciona sin tarjeta.
- Si se cierra la pestaña con una petición retenida, se aborta.
- Las pruebas de formularios, enlaces y Enter siguen pasando.

## Estimación

3–5 días de trabajo más pruebas. Es el cambio pendiente de mayor impacto en
compatibilidad después de la vista integrada.
