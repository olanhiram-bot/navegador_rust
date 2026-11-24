# Hoja de ruta del navegador

El primer objetivo es que el navegador sea una opción diaria en Linux. Una vez que cumpla ese nivel, la ventaja que buscamos es poder delegar tareas web, retomar el control de la misma pestaña y comprobar qué se envió con un recibo firmado. Los [resultados locales del prototipo](README.md#qué-demuestra-el-arnés) son el punto de partida.

El [hito de uso diario](https://github.com/olanhiram-bot/navegador_rust/issues/1) coordina la entrega. El [registro de verificación del 23 de noviembre](docs/investigacion/uso-diario-linux-2026-11-23.md) contiene las regresiones y la medición de video en Wayland.

El [hito competitivo](https://github.com/olanhiram-bot/navegador_rust/issues/8) añade Chrome y Brave a la comparación de escritorio. El [sondeo de Wayland](docs/investigacion/chrome-brave-qt-2026-11-24.md) registra CPU, PSS y las comprobaciones de la variante [Qt experimental](qt/README.md).

## 1. Fluidez y compatibilidad

- [ ] **Medir una línea base frente a Firefox y Brave.** Usar la misma máquina, escala, resolución y una versión optimizada del proyecto. Registrar versiones, distinguir caché fría de caliente y ejecutar al menos cinco repeticiones por recorrido. Medir tiempo al primer contenido, latencia del clic al primer cambio visible, tirones al desplazar y memoria del árbol de procesos con una y diez pestañas. Usar YouTube, un documento de prueba y una web de noticias. Repetir el recorrido con una página local para separar la demora de la aplicación de la red.
- [x] **Probar una vista integrada de Chromium.** Compararla con la vista actual por JPEG. Verificar clic, arrastre, selección, portapapeles, teclado, DPI, video con audio y navegación dentro de una misma página. Adoptarla cuando mejore la latencia visible y permita seguir verificando las acciones del agente. Resultado: [la vista de Qt WebEngine recibe el clic real y el agente sigue pudiendo verificar y cancelar el POST](docs/investigacion/vista-integrada-qt-2026-11-20.md), pero bajo Xvfb presenta la superficie nueva ~550 ms después del clic, frente a 74 ms de la vista por JPEG. No se adopta todavía; falta medirla en un escritorio con compositor.
- [ ] **Convertir los recorridos básicos en pruebas de regresión.** Cubrir búsqueda, apertura de videos, redirecciones, formularios, historial y recuperación tras un error. Usar fixtures para la ejecución habitual y recorridos públicos de solo lectura para verificar compatibilidad.
- [x] **Medir y reducir el costo de las pestañas.** Todas las pestañas comparten un solo proceso Chromium y un solo perfil desde 2026-11-20. Medición en Linux/x86_64 con una página local y compilación optimizada: **383 MiB con una pestaña y 495 MiB con diez** (PSS de todo el árbol, incluido el display virtual), ~12 MiB por pestaña extra. CPU con la ventana enfocada: **video entre 130 % y 270 % de un núcleo** y **animación a 60 fps entre 250 % y 500 %** según la carga de la máquina (Brave, 15–19 % en ambos casos); al perder el foco la ventana baja a **~11 %** porque se pausa el screencast. El gasto está en capturar, codificar y decodificar JPEG a 60 fps; la vista integrada de Chromium lo eliminaría. `everyNthFrame: 2` reduce el coste a la mitad pero en este Chromium deja de emitir frames, así que no se usa.

[Speedometer 3.1](https://browserbench.org/Speedometer3.1/about.html) complementa las mediciones de la ventana.

### Primera medición local

Se ejecutaron cinco rondas alternando el orden de los navegadores en Linux x86_64 con un Intel Core Ultra 5 235U y un Xvfb de 1150 × 900. Cada arranque usó un perfil nuevo. Un servidor HTTP en loopback mostró una página de color uniforme con un enlace a una segunda página de otro color. Se generó un clic con XTest tras dar foco a la ventana y se detectó el cambio de color en la pantalla real de Xvfb. El primer tiempo incluye lanzar el proceso. El segundo empieza justo antes de pulsar el botón del mouse.

| Navegador | Primer contenido, mediana | Clic a cambio visible, mediana |
|---|---:|---:|
| Proyecto en modo debug con Chromium 155.0.8059.84 | 1044 ms | 209 ms |
| Brave 158.1.100.58 | 917 ms | 95 ms |
| Firefox 158.0.1 | 2880 ms | 146 ms |

Esta medición inicial cubre un enlace local y la ventana gráfica. Quedan por medir los sitios públicos, el desplazamiento, el video y la memoria con varias pestañas. El arranque de un perfil nuevo incluye el trabajo de inicializar ese perfil.

### Segunda medición local (optimizada)

Se repitió el mismo recorrido con el proyecto en `--release`, un solo proceso Chromium para la sesión y la vista integrada de Qt como candidata. La carga de la máquina no fue idéntica a la primera medición, así que los valores absolutos solo se comparan dentro de esta tabla.

| Navegador | Primer contenido, mediana | Clic a cambio visible, mediana |
|---|---:|---:|
| Proyecto en release, un Chromium por sesión | 393 ms | 74 ms |
| Qt WebEngine integrado | 493 ms | 549 ms |
| Brave 158.1.100.58 | 340 ms | 45 ms |
| Firefox 158.0.1 | 908 ms | 62 ms |

En la compilación optimizada, el clic tarda casi la mitad que en la medición inicial con `dev`. La shell nueva sobre GPUI ya pinta la interfaz con la GPU, pero los frames de la página siguen llegando por JPEG: el coste de CPU no cambia hasta que se adopte la vista integrada. El detalle de la vista integrada, incluida la verificación del POST por CDP, está en [la nota de la prueba](docs/investigacion/vista-integrada-qt-2026-11-20.md).

## 2. Uso diario y seguridad

- [x] **Arrancar Chromium con el sandbox habilitado.** La prueba en Linux confirma el aislamiento mediante `chrome://sandbox/`. Si Chromium termina antes de abrir el puerto de control, el motor informa qué ejecutable falló y su código de salida.
- [x] **Perfiles persistentes y recuperación de sesión.** La shell usa `~/.local/share/navegador/perfil` y comparte un único Chromium entre pestañas, así que las cookies se comparten y los inicios de sesión sobreviven al reinicio. Al arrancar sin URL restaura las pestañas de la última sesión (`~/.local/state/navegador/sesion.json`). `--efimero` desactiva perfil, restauración y guardado, y borra el perfil al cerrar; `--perfil DIR` y `--sin-restaurar` ajustan el comportamiento. Un candado sobre el perfil avisa si otra ventana ya lo está usando. Las pruebas cubren cookies compartidas entre pestañas, cookies que sobreviven al reinicio y el borrado del perfil efímero.
- [x] **Descargas y permisos por sitio.** Chromium deja las descargas en `~/Downloads` (o `$NAVEGADOR_DESCARGAS`) con `Browser.setDownloadBehavior`; la shell avisa al empezar y al terminar y lista las descargas de la sesión con «Abrir» y «Carpeta». Los permisos de notificaciones, ubicación, cámara y micrófono se conceden o deniegan por origen desde el menú ☰ → «Permisos de este sitio»; se guardan en `ajustes.json` y se aplican al cargar la página. Pruebas: una descarga real llega a la carpeta y avisa, y conceder/denegar notificaciones cambia lo que ve la página. El panel ahora muestra eventos de progreso de Chromium y permite cancelar por descarga. La prueba local cancela una transferencia mientras termina otra sin longitud anunciada.
- [ ] **Funciones de escritorio.** Añadir marcadores, historial, zoom por sitio, búsqueda dentro de página y accesibilidad por teclado. Revisar ventanas estrechas y escalas de pantalla distintas. Desde 2026-11-20 hay búsqueda en página (Ctrl+F, con contador y anterior/siguiente), zoom por sitio persistido en `~/.local/state/navegador/ajustes.json` (Ctrl++ / Ctrl+- / Ctrl+0), marcadores (Ctrl+D, lista con Ctrl+Shift+B), historial persistente en `historial.jsonl` (Ctrl+H, con filtro y borrado) y Tab/Shift+Tab llegan a los campos de la página. El teclado va a la página tras navegar. También hay menú contextual (clic derecho), clic central para abrir enlaces en segundo plano, deshacer cierre (Ctrl+Shift+T), ajustes con motor de búsqueda (Ctrl+,) y sugerencias de historial, marcadores y pestañas en la barra de direcciones. Las pestañas que abre la web (`window.open`, `target=_blank`) se adoptan como pestañas y una pestaña caída se recarga. Falta revisar ventanas estrechas y escalas distintas.
- [x] **Distribución y actualizaciones.** `packaging/install.sh [PREFIJO]` compila con `--locked`, instala binario, entrada de escritorio e icono en `PREFIJO` (por defecto `~/.local`) y con `--predeterminado` registra http/https con `xdg-mime`. `packaging/PKGBUILD` construye `navegador-git` y valida con `makepkg --printsrcinfo`. `navegador-shell --version` informa la versión. Chromium es el del sistema y recibe sus parches con él. Falta probar el paquete en una instalación limpia.

## 3. Tareas delegadas que puedas verificar

- [ ] **Compartir la pestaña entre la ventana y el agente.** Conectar el panel al agente y verificar que el traspaso conserva URL, pestaña, scroll y estado del formulario.
- [ ] **Vincular la aprobación a la petición.** Cubrir envíos desde teclado, formularios y controles dinámicos. Los clics que solo ejecutan JavaScript se pulsan como interacción local (sin tarjeta) para no romper los sitios; falta el diseño de retener la petición y aprobarla antes de que salga, descrito en [la nota de política](docs/investigacion/politica-clics-js.md) (3–5 días). Verificar con fixtures que se detienen los envíos que cambian tras aprobarse y que cada envío autorizado produce su recibo. Desde 2026-11-20 un Enter en un campo de formulario abre la misma tarjeta y deja recibo que un clic en su botón, y una prueba comprueba que no sale ninguna petición antes de aprobar; faltan los controles que envían sin formulario ni botón.
- [x] **Completar tres recorridos.** Comparar productos, rellenar un formulario y preparar una reserva para aprobación. Los tres están cubiertos con fixtures en `tres_recorridos_locales_comparar_rellenar_y_reservar`: el importe de la tarjeta coincide con el total visible, el formulario envía el nombre escrito y la reserva envía fecha y personas, cada uno con recibo. Falta repetirlos en entornos de prueba autorizados y medir tasa de éxito e intervenciones.
- [ ] **Hacer los recibos fáciles de inspeccionar.** Mostrar destino, datos relevantes y resultado. Proteger la información sensible y verificar la firma sin conexión.

## 4. Privacidad competitiva

- [ ] **Bloquear rastreadores y separar el almacenamiento entre sitios.** Medir peticiones bloqueadas, compatibilidad y excepciones por sitio con un conjunto de pruebas.
- [ ] **Controlar qué puede leer y enviar el agente.** Permitir limitar sitios y datos por tarea, revisar el destino del contexto y comprobar qué sale del equipo.

Referencias de comparación: [Firefox Enhanced Tracking Protection](https://support.mozilla.org/en-US/kb/enhanced-tracking-protection-firefox-desktop), [Brave Shields](https://brave.com/privacy-features/) y [navegación con Brave Leo](https://brave.com/blog/ai-browsing/).
