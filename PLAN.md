# PLAN — Navegador auditable

Última actualización: 2026-11-13 · Estado: **fase 1 completa; siguiente: fase 2**

## Visión

Un navegador local y abierto donde agente y persona comparten la misma sesión, y donde
toda acción con consecuencias está atada a la acción real que se ejecuta y deja un recibo
verificable.

## Principios de ejecución

1. **Riesgo primero.** Se construye antes lo que puede invalidar el producto.
2. **Gate binario.** Cada fase termina con un criterio que pasa o no pasa. Si no pasa, se replantea.
3. **Sin UI hasta que el descriptor funcione.** La shell es reemplazable; el protocolo no.
4. **Un motor por fase.** Chromium ahora; Servo solo si los datos lo justifican.
5. **Cada acción consecuente deja evidencia.** Si no hay recibo, no se despacha.

## Repositorio

```
crates/
  descriptor/    # ActionDescriptor, canonical JSON, hash, policy
  engine-cdp/    # lanzador de Chromium + cliente CDP mínimo
  navegador/     # binario CLI + arnés de ataques
fixtures/        # páginas de ataque y tarea legítima
docs/            # investigación, tesis, plan
```

## Fases

### Fase 0 — Fundación (completada)

- Toolchain Rust, workspace, plan, fixtures.
- Gate: `cargo test` compila y corre.

### Fase 1 — Descriptor + arnés (semana 1) — **completada**

- Descriptor ground-truth desde DOM/estado de formulario en *isolated world*.
- Hash canónico + `generation`.
- Policy v0: orígenes permitidos, importe máximo, aprobación por hash.
- Arnés con 3 ataques + 1 tarea legítima.
- Gate: 3/3 ataques detectados, 0 falsos bloqueos, todo en CI. **Cumplido.**

Evidencia: `docs/producto/2026-11-13-resultado-semana-1.md`. El binding es
*pre-send*: `Fetch.requestPaused` aborta el POST sustituto antes de que salga.

### Fase 2 — Action Card + recibo (semana 2) — **siguiente**

- Ventana nativa mínima con Deny dominante y valores escapados.
- Binding por hash antes de despachar; `Network.postData` después.
- Recibo firmado (ed25519) y verificación offline.
- Gate: sustitución post-aprobación abortada 10/10; recibo verificado en otra máquina.

### Fase 3 — SessionCore + MCP (semanas 3–4)

- Trait `Engine`, refs `@eN` estables con generación, action log.
- Tools MCP: `open`, `snapshot`, `act`, `read`, `screenshot`, `handoff`.
- Gate: un host MCP completa un flujo de 3 pasos; 100% de acciones consecuentes con recibo.

### Fase 4 — Endurecer (semanas 5–6)

- Perfiles efímeros, sets de orígenes read-only/read-write, egress policy.
- 3 escenarios reales, métricas de falsos bloqueos, CI de regresión.

### Fase 5 — Shell (semanas 7–8)

- GPUI o egui: pestañas, omnibox, panel del agente, handoff.
- Gate: el demo de 60 segundos grabado de punta a punta.

### Fase 6 — Servo (post-MVP)

- Adaptador Servo detrás del mismo trait; sesiones de agente sin paint.
- Gate: coste por sesión ≤ 1/5 y ≥70% de tareas sin fallback.

## Fuera de alcance

Motor propio, fork de Chromium, extensiones, móvil, cloud, antidetect, CAPTCHA.

## Riesgos vivos

| Riesgo | Estado | Mitigación |
|---|---|---|
| AX no es ground truth | **Confirmado** | Descriptor desde DOM en isolated world |
| BrowserOS ya graba y audita | **Confirmado** | Diferenciar por binding + recibo, no por replay |
| Interceptar navegación con Fetch cuelga | **Confirmado** | `Network` + ejecutor propio |
| iframes cross-origin van a otro proceso | **Confirmado** | Adjuntar target del iframe o bloquear |
| Sitios reales resisten el descriptor | Abierto | Semana 5 con 3 escenarios reales |

## Cadencia

- Un commit por gate.
- Los ataques son tests de regresión permanentes.
- Si un gate falla dos veces, se para y se replantea el alcance.
