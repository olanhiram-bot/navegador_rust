# Navegador auditable

Un navegador local y abierto donde una persona y un agente comparten la misma
sesión, y donde toda acción con consecuencias queda atada a la acción que
realmente se ejecuta.

**Estado: fase 1 completa.** El descriptor ground-truth detecta los tres ataques
del MVP y la tarea legítima pasa sin falsos bloqueos.

## Cómo correrlo

```bash
cargo test --workspace          # unitarios + arnés de ataques (requiere Chromium)
cargo run --example smoke       # descriptor + dispatch contra un fixture
cargo run --bin navegador -- audit https://ejemplo.com "#pagar" 1.00 ana
```

`NAVEGADOR_CHROME` permite apuntar a un binario concreto. Por defecto se buscan
`chromium`, `chromium-browser` y `google-chrome-stable`.

## Estructura

```
crates/descriptor/   ActionDescriptor, hash canónico, intención, política, binding
crates/engine-cdp/   Lanzador de Chromium + cliente CDP + extracción en mundo aislado
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
| `legit.html` | transferencia real de `$1.00` a Ana | se aprueba y se ejecuta |

El punto clave: el árbol de accesibilidad solo ve la etiqueta que miente. El
descriptor se construye desde DOM, estado de formulario y origen resuelto, y el
binding compara la petición interceptada con lo aprobado.

## Documentos

- [Tesis de producto](docs/producto/2026-11-13-tesis-producto.md)
- [Plan de arranque del MVP](docs/producto/2026-11-13-plan-arranque-mvp.md)
- [Resultado de la semana 1](docs/producto/2026-11-13-resultado-semana-1.md)
- [Recomendación de arquitectura](docs/investigacion/2026-11-12-recomendacion-arquitectura.md)
- [Plan completo](PLAN.md)
