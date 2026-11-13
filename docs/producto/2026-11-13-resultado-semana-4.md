# Resultado de la semana 4 — endurecer

Fecha: 2026-11-13 · Gate de la fase 4

## Resultado

| Criterio | Resultado |
|---|---|
| Perfiles efímeros por instancia | ✅ directorio temporal propio; se borra al cerrar |
| Orígenes read-only / read-write | ✅ `OriginPolicy::Allowlist` |
| Egress: destino no declarado | ✅ se deniega antes del click; 0 peticiones |
| 3 escenarios legítimos (compra, perfil, reserva) | ✅ 3/3 ejecutados con recibo |
| Falsos bloqueos en esos escenarios | ✅ 0/3 |
| Log encadenado de los 3 recibos | ✅ `ReceiptLog::verify` |

Salida real de la métrica:

```
[métrica] ejecutadas 3/3 · falsos bloqueos 0/3
```

Los escenarios son fixtures locales realistas servidos por `FixtureServer`
(`shop.html`, `settings.html`, `booking.html`), no sitios en producción. La
validación contra sitios reales queda como paso manual: no se puede automatizar
sin red ni ejecutar transacciones reales desde CI.

## Política de orígenes

`session::OriginPolicy` separa dos permisos que antes eran uno:

```rust
pub enum OriginPolicy {
    /// Cualquier documento visitado se lee; las acciones solo van a orígenes
    /// ya visitados (comportamiento del MVP).
    Visited,
    /// Solo los declarados: `read` para abrir y leer, `write` para actuar.
    Allowlist { read: Vec<String>, write: Vec<String> },
}
```

Propiedades que se testean:

- Un origen en `read` y no en `write` **se abre y se lee**, pero `act` se
  deniega con «origen de acción no permitido» y no se hace click.
- Un origen fuera de `read` **no se abre**: `open` devuelve
  `OriginNotReadable`.
- El destino de la acción es la lista de egreso: aunque la página sea legible,
  si el formulario apunta a un origen que no está en `write`, la acción se
  deniega antes de salir y el sink no recibe nada.
- El valor por defecto sigue siendo `Visited`, para no romper el CLI ni el gate
  MCP.

En el servidor MCP:

```bash
navegador-mcp \
  --read-origin https://tienda.example \
  --read-origin https://banco.example \
  --write-origin https://tienda.example
```

Declarar cualquiera de las dos listas activa la allowlist. `--write-origin` es
obligatorio para que una acción pueda enviar datos: sin él, la sesión es de
solo lectura para actuar.

## Perfiles efímeros

`Chrome::launch` ya usaba `tempfile::TempDir` como `--user-data-dir`. Ahora se
expone `Chrome::profile_dir()` y un test comprueba tres cosas: el perfil vive
en el temporal del sistema, desaparece al dropear el motor y dos instancias no
comparten directorio. No hay cookies ni almacenamiento que sobreviva a la
sesión.

## Límites que siguen abiertos

- **La política de egreso cubre acciones, no todo el tráfico.** Subrecursos,
  beacons y `fetch` de la página no pasan por la lista; para eso haría falta
  interceptación permanente con una política de red, que hoy no está.
- **Sitios reales sin validar.** Los escenarios son locales; las métricas de
  falsos bloqueos en producción siguen pendientes y no se pueden correr en CI.
- iframes cross-origin siguen sin adjuntarse.
- El recibo no está encadenado a una clave fijada por el usuario: prueba
  integridad y binding, no identidad.

## Siguiente

Cerrar la fase 4 con 3 escenarios reales en manual y, después, la fase 5:
shell con pestañas, omnibox y panel del agente sobre `SessionCore`.
