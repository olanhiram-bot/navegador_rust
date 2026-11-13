# Navegador auditable

Un navegador local y abierto donde una persona y un agente comparten la misma
sesión, y donde toda acción con consecuencias queda atada a la acción que
realmente se ejecuta.

**Estado: fase 2 completa.** El descriptor ground-truth detecta los ataques, la
tarjeta muestra el valor real antes de aprobar, la sustitución posterior a la
aprobación se aborta 10/10 y cada acción ejecutada deja un recibo firmado que se
verifica offline.

## Cómo correrlo

```bash
cargo test --workspace          # unitarios + arnés de ataques (requiere Chromium)

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
`chromium`, `chromium-browser` y `google-chrome-stable`.

## Estructura

```
crates/descriptor/   ActionDescriptor, hash canónico, intención, política, binding
crates/engine-cdp/   Lanzador de Chromium + cliente CDP + extracción en mundo aislado
crates/card/         Modelo de la tarjeta (testeable sin pantalla) + render egui
crates/receipt/      Recibo ed25519 firmado y verificación offline
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
- [Recomendación de arquitectura](docs/investigacion/2026-11-12-recomendacion-arquitectura.md)
- [Plan completo](PLAN.md)
