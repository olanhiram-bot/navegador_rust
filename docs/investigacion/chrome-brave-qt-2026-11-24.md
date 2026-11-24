# Chrome, Brave y el candidato Qt en Wayland

El [hito de escritorio](https://github.com/olanhiram-bot/navegador_rust/issues/8) usa Chrome y Brave como referencias. El comparador se ejecutó en un Hyprland anidado con escala 1 y una salida de 941 × 1174. Se usaron Chrome 158.0.8127.36, Brave 158.1.100.58, GPUI en release y Qt WebEngine 6.11.2. Cada navegador usó datos locales aislados y la misma página con WebM VP9 de 640 × 360 a 60 fps en bucle. El comparador confirmó el avance del reproductor antes de medir CPU.

| Navegador | CPU durante video, perfil nuevo (%) | CPU con perfil reutilizado (%) | PSS 1 pestaña (MiB) | PSS 10 pestañas (MiB) |
|---|---:|---:|---:|---:|
| Chrome | 69.21 | 6.93 | 400.78 | 512.69 |
| Brave | 35.70 | 17.73 | 413.16 | 498.51 |
| Navegador GPUI | 219.05 | 204.29 | 570.81 | 680.62 |
| Navegador Qt | 33.05 | 31.38 | 303.92 | 370.34 |

Es una ronda exploratoria con ventanas de muestreo de 2 segundos. La CPU se expresa como porcentaje de un núcleo para el árbol de procesos. La memoria usa pestañas con contenido de texto y suma PSS del árbol. El comparador obtuvo señales de desplazamiento del motor en GPUI y Qt. Chrome y Brave agotaron el tiempo de espera de esa señal en el compositor anidado. La captura del compositor también agotó el tiempo de espera. Estos resultados sirven para priorizar el trabajo en la vista nativa. La aceptación requiere repetir en un escritorio con captura de presentación, completar las dos series de cinco rondas y verificar los recorridos públicos.

El candidato compiló con qmake6 y make. Guardó dos pestañas y la selección, restauró una cookie persistente y aisló esa cookie en modo efímero. Una fixture entre localhost y 127.0.0.1 comprobó el bloqueo predeterminado de almacenamiento de terceros y que la excepción activa afecta solo al sitio elegido. Una descarga lenta se canceló desde la ventana y una segunda descarga se guardó completa. El empaquetado local de Rust instaló la shell y el comando `navegador verify-log`. Pasaron 78 pruebas Rust.

La adopción requiere conservar el control y la aprobación del agente en la misma pestaña. El filtro experimental de Qt cubre almacenamiento de terceros. La lista mantenida de bloqueos de solicitudes sigue como tarea de privacidad. [Comparación #9](https://github.com/olanhiram-bot/navegador_rust/issues/9) · [Qt #10](https://github.com/olanhiram-bot/navegador_rust/issues/10) · [Privacidad #11](https://github.com/olanhiram-bot/navegador_rust/issues/11) · [Agente #12](https://github.com/olanhiram-bot/navegador_rust/issues/12)
