# Próximas tareas del navegador auditable

## Estado y alcance

Propuesta de planificación para revisión del usuario y del host. No se han guardado archivos, ejecutado comandos, implementado cambios ni iniciado trabajadores desde esta consola. Los IDs T-001 a T-004 son provisionales hasta que el host compruebe el inventario del consumidor y los reserve.

Petición del usuario: preparar tareas para ver qué podemos hacer. Tamaño: descubrimiento de proyecto con una primera tanda acotada.

Fuentes de intención: [PLAN.md](../../PLAN.md), [shell mínima](../producto/2026-11-13-shell-minima.md) y [resultado de fase 4](../producto/2026-11-13-resultado-semana-4.md). Este documento concreta esa dirección; no sustituye sus gates ni declara nuevas decisiones de producto aceptadas.

## Modelo del proyecto

El producto es un navegador local para una persona y un agente. SessionCore coordina descriptor, política, aprobación por hash, despacho y recibo. Chromium proporciona ejecución y observación mediante CDP. La shell egui posee una sesión por pestaña. El servidor MCP actual crea otra sesión; no controla la sesión de la ventana.

DOCUMENTADO: las fases 1–3 están completas según los resultados históricos. La fase 4 conserva pendiente la validación manual en tres sitios reales. La fase 5 conserva pendiente el demo grabado de 60 segundos.

OBSERVADO: existen pruebas del puente de shell para frames, pestañas, historial y pago aprobado con recibo en `crates/shell/tests/bridge.rs`. Su presencia no acredita que pasen en esta revisión.

OBSERVADO: no existe `tasks/` en la copia inspeccionada. No se encontraron archivos AGENTS.md mediante la búsqueda disponible. Historial, IDs retirados, modificaciones pendientes, propietario del checkout, candidatos y actividad del consumidor canónico no se han evaluado.

## Hallazgos que determinan la prioridad

1. **Entrada de Enter fuera de la cadena auditable.** `crates/shell/src/app.rs::special_key` traduce Enter a una tecla CDP. `crates/shell/src/engine.rs::handle_command` llama directamente a `Session::key` y después a `refresh_after_enter`. El propio comentario indica que Enter puede enviar un formulario. `crates/session/src/lib.rs::key` delega al transporte y `crates/engine-cdp/src/chrome.rs::key` emite `Input.dispatchKeyEvent`. No aparece una llamada a `act` en esa ruta. INFERIDO, sin reproducción: un formulario con envío implícito puede producir una petición sin aprobación ni recibo. T-001 incorpora la reproducción y una mitigación local acotada.
2. **Estado de agente engañoso.** `crates/shell/src/app.rs::agent_panel` muestra `conectado (MCP)` al activar una variable local. El documento de shell declara explícitamente que no existe un canal MCP hacia la ventana. `McpServer::ensure_session` crea su propia sesión. T-002 corrige la representación; no construye un transporte nuevo.
3. **Validación externa sin resolver.** El resultado de semana 4 distingue correctamente los tres fixtures locales de los tres escenarios reales pendientes. No se conocen sitios, cuentas de prueba ni acciones autorizadas. T-003 mantiene ese gate pendiente.
4. **Demo sin grabar.** El documento de shell y PLAN.md conservan el gate de 60 segundos. T-004 lo mantiene como trabajo de la persona, separado de los tests y de la validación externa.

## Objetivos y límites

Objetivos existentes: conservar el binding de aprobación y los recibos; validar escenarios reales; demostrar la shell mediante un video de 60 segundos.

Objetivos de ingeniería derivados, por decisión del planificador: impedir el envío implícito mediante Enter en el puente de shell y evitar que la UI afirme una conexión inexistente.

No objetivos, por decisión del planificador: implementar ahora sockets, conectar un proveedor de agente, añadir Servo, cambiar firmas o formatos de recibo, incorporar controles nuevos o desarrollar una política global de red.

Antiobjetivos, por decisión del planificador: emitir recibos ficticios para compensar envíos sin gate; confundir un bloqueo local con conexión de agente; presentar fixtures como sitios reales; declarar los gates completos sin su prueba específica.

## Decisiones de diseño

- **T-001:** bloquear Enter dirigido a la página en el receptor de comandos de shell, antes de llamar al transporte. Reconocer `key = Enter`, `code = Enter` o `NumpadEnter`, o `virtual_key = 13`, también con modificadores. Informar a la persona de que debe usar el botón y su tarjeta. Es una mitigación temporal elegida por el planificador, con pérdida explícita del envío y salto de línea mediante Enter dentro de la página. Enter en la omnibox sigue navegando. Se conserva el texto compuesto y las demás teclas. No se elige todavía un submitter implícito: su resolución y binding requieren un cambio mayor. Filtrar solo en egui sería insuficiente porque el puente acepta comandos directamente.
- **T-002:** conservar la inspección local y su bloqueo de interacción, pero nombrarlos como tales. No introducir una máquina de estados de conexión para un transporte inexistente. La UI debe indicar que MCP usa otra sesión.
- **T-003:** no seleccionar servicios ni conceder permisos por defecto. La persona debe delimitar los tres escenarios y sus acciones sin gasto ni efectos sobre terceros.
- **T-004:** grabar con fixtures locales después de las correcciones de shell. No depende de T-003: un demo local no prueba compatibilidad con sitios reales. No simular un agente conectado.

## Orden y dependencias

- T-001: Now, prioridad 1000. Primera implementación propuesta: mitigación de Enter.
- T-002: Now, prioridad 2000. Implementación independiente: estado honesto del panel.
- T-003: Needs decision, prioridad 3000. Validación real, propiedad de la persona, sin autorización live.
- T-004: Needs decision, prioridad 4000. Grabación manual; depende de T-001 y T-002 y requiere acceso a pantalla y captura.

T-001 y T-002 no tienen dependencia funcional entre sí. Comparten README.md y el documento de shell: pueden producir conflictos de integración, y no se promete ejecución paralela sin conflictos. T-003 y T-004 comparten PLAN.md; eso no crea una dependencia de producto. No se añaden recursos exclusivos para serializar archivos.

## Verificación y revisión del contrato

Todas las pruebas indicadas en las tareas son futuras o no ejecutadas. La ejecución requiere Rust y Chromium según README.md; no se presupone que estén disponibles aquí. CI ya declara `cargo test --workspace --all-targets` y compilación de shell.

Revisión del planificador como implementador, sin subagente:

- La ruta insegura tiene una mitigación definida y un fixture realizable dentro de T-001; no necesita sitios externos.
- T-002 no requiere transporte ni credenciales. Los detalles reversibles de presentación y del test de egui quedan al implementador dentro del contrato.
- Los inputs externos se conservan exclusivamente en T-003 y T-004. No se inventa autorización ni se reemplaza su prueba manual con una prueba offline.
- Los tests existentes de pago y de historial se conservan, en particular el recibo inicial `Some(0)` y la verificación de cadena.
- No hay generadores ni cambios de dependencias previstos para T-001 y T-002. Si aparece un requisito fuera del conjunto declarado, se devuelve al coordinador antes de ampliar alcance.

Riesgos residuales: bloquear Enter no cubre handlers JavaScript de texto, foco o clicks, ni tráfico arbitrario de la página. La política actual cubre acciones, no todo el egreso. Tampoco se afirma que la ruta observada se haya reproducido: el primer trabajo de T-001 es esa reproducción. Los tests deben distinguir un problema de instalación de Chromium de un resultado del producto.

## Handoff pendiente

El host debe revisar y validar este lote contra el consumidor real antes de proponer su aplicación. No se ejecutaron `autowork capabilities`, prepare, validate, prune ni why: las operaciones de shell no están disponibles en esta consola. No se conoce el estado de preparación, selección, admisión, runtime, entrega a main ni propiedad activa. No se afirma TOOLCHAIN_MISMATCH ni NO_STORE como resultado de una herramienta no ejecutada.

Primera tarea recomendada una vez aplicada y comprobada la selección: T-001. Modo propuesto para una futura orden explícita: ejecución de la tarea en la sesión actual, no servicio continuo. Esta planificación no inicia ninguno de los dos modos.
