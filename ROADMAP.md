# Hoja de ruta del navegador

El primer objetivo es que el navegador sea una opción diaria en Linux. Una vez que cumpla ese nivel, la ventaja que buscamos es poder delegar tareas web, retomar el control de la misma pestaña y comprobar qué se envió con un recibo firmado. Los [resultados locales del prototipo](README.md#qué-demuestra-el-arnés) son el punto de partida.

## 1. Fluidez y compatibilidad

- [ ] **Medir una línea base frente a Firefox y Brave.** Usar la misma máquina, escala, resolución y una versión optimizada del proyecto. Registrar versiones, distinguir caché fría de caliente y ejecutar al menos cinco repeticiones por recorrido. Medir tiempo al primer contenido, latencia del clic al primer cambio visible, tirones al desplazar y memoria del árbol de procesos con una y diez pestañas. Usar YouTube, un documento de prueba y una web de noticias. Repetir el recorrido con una página local para separar la demora de la aplicación de la red.
- [ ] **Probar una vista integrada de Chromium.** Compararla con la vista actual por JPEG. Verificar clic, arrastre, selección, portapapeles, teclado, DPI, video con audio y navegación dentro de una misma página. Adoptarla cuando mejore la latencia visible y permita seguir verificando las acciones del agente.
- [ ] **Convertir los recorridos básicos en pruebas de regresión.** Cubrir búsqueda, apertura de videos, redirecciones, formularios, historial y recuperación tras un error. Usar fixtures para la ejecución habitual y recorridos públicos de solo lectura para verificar compatibilidad.
- [ ] **Medir y reducir el costo de las pestañas.** Registrar RAM y CPU en reposo y bajo carga. Evaluar reutilizar el proceso de navegador entre pestañas con aislamiento por perfil y suspender trabajo en segundo plano.

[Speedometer 3.1](https://browserbench.org/Speedometer3.1/about.html) complementa las mediciones de la ventana.

### Primera medición local

Se ejecutaron cinco rondas alternando el orden de los navegadores en Linux x86_64 con un Intel Core Ultra 5 235U y un Xvfb de 1150 × 900. Cada arranque usó un perfil nuevo. Un servidor HTTP en loopback mostró una página de color uniforme con un enlace a una segunda página de otro color. Se generó un clic con XTest tras dar foco a la ventana y se detectó el cambio de color en la pantalla real de Xvfb. El primer tiempo incluye lanzar el proceso. El segundo empieza justo antes de pulsar el botón del mouse.

| Navegador | Primer contenido, mediana | Clic a cambio visible, mediana |
|---|---:|---:|
| Proyecto en modo debug con Chromium 155.0.8059.84 | 1044 ms | 209 ms |
| Brave 158.1.100.58 | 917 ms | 95 ms |
| Firefox 158.0.1 | 2880 ms | 146 ms |

Esta medición inicial cubre un enlace local y la ventana gráfica. Quedan por medir la compilación optimizada, los sitios públicos, el desplazamiento, el video y el costo de varias pestañas. El arranque de un perfil nuevo incluye el trabajo de inicializar ese perfil.

## 2. Uso diario y seguridad

- [x] **Arrancar Chromium con el sandbox habilitado.** La prueba en Linux confirma el aislamiento mediante `chrome://sandbox/`. Si Chromium termina antes de abrir el puerto de control, el motor informa qué ejecutable falló y su código de salida.
- [ ] **Perfiles persistentes y recuperación de sesión.** Restaurar pestañas e inicios de sesión tras reiniciar. Mantener una opción de perfil efímero y demostrar que sus cookies se eliminan al cerrar.
- [ ] **Descargas y permisos por sitio.** Mostrar progreso, cancelación y destino de las descargas. Permitir revisar y revocar permisos de cámara, micrófono, ubicación y notificaciones.
- [ ] **Funciones de escritorio.** Añadir marcadores, historial, zoom por sitio, búsqueda dentro de página y accesibilidad por teclado. Revisar ventanas estrechas y escalas de pantalla distintas.
- [ ] **Distribución y actualizaciones.** Probar instalación, actualización del ejecutable y compatibilidad con versiones de Chromium que reciben parches.

## 3. Tareas delegadas que puedas verificar

- [ ] **Compartir la pestaña entre la ventana y el agente.** Conectar el panel al agente y verificar que el traspaso conserva URL, pestaña, scroll y estado del formulario.
- [ ] **Vincular la aprobación a la petición.** Cubrir envíos desde teclado, formularios y controles dinámicos. Verificar con fixtures que se detienen los envíos que cambian tras aprobarse y que cada envío autorizado produce su recibo.
- [ ] **Completar tres recorridos.** Comparar productos, rellenar un formulario y preparar una reserva para aprobación. Empezar con fixtures y continuar en entornos de prueba autorizados. Medir tasa de éxito, intervenciones y correspondencia entre lo aprobado y lo enviado.
- [ ] **Hacer los recibos fáciles de inspeccionar.** Mostrar destino, datos relevantes y resultado. Proteger la información sensible y verificar la firma sin conexión.

## 4. Privacidad competitiva

- [ ] **Bloquear rastreadores y separar el almacenamiento entre sitios.** Medir peticiones bloqueadas, compatibilidad y excepciones por sitio con un conjunto de pruebas.
- [ ] **Controlar qué puede leer y enviar el agente.** Permitir limitar sitios y datos por tarea, revisar el destino del contexto y comprobar qué sale del equipo.

Referencias de comparación: [Firefox Enhanced Tracking Protection](https://support.mozilla.org/en-US/kb/enhanced-tracking-protection-firefox-desktop), [Brave Shields](https://brave.com/privacy-features/) y [navegación con Brave Leo](https://brave.com/blog/ai-browsing/).
