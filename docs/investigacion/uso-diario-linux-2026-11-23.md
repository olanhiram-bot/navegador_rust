# Verificación de uso diario en Linux

El [hito de uso diario](https://github.com/olanhiram-bot/navegador_rust/issues/1) reúne los recorridos de entrada, formularios, reproducción, descargas e instalación.

## Interacción y regresiones

En una ventana GPUI instalada en un prefijo temporal bajo Wayland, el mouse enfocó un campo y seleccionó un párrafo por arrastre. Ctrl+C y Ctrl+V copiaron el texto exacto al campo. Un clic en un enlace local ejecutó el enrutador de la página. En una ventana de 800 × 850, la barra de direcciones abrió una URL local y la página mostró un campo editable. Las pruebas del puente verifican coordenadas CSS y selección con factores de captura de 1, 1.25 y 2. También verifican los píxeles y un enlace a escala 2.

La prueba de aprobación cambia el nombre del botón y un campo del formulario. La aprobación caducada deja cero solicitudes. Cancelar una nueva tarjeta también deja cero. Una aprobación posterior envía el formulario una vez con el valor actualizado. La prueba de descargas cancela una transferencia y deja terminar otra que transmite sin `Content-Length`.

Las 77 pruebas del workspace pasaron en release. Este comando repite la verificación con Chromium instalado.

```sh
NAVEGADOR_HEADLESS=1 cargo test --release --locked --workspace --all-targets -- --test-threads=1
```

Google cargó la portada, recibió la búsqueda y redirigió a una verificación de tráfico. La prueba se detuvo en esa pantalla. YouTube cargó la portada sin historial, aceptó la consulta «Blender Big Buck Bunny», abrió el resultado y lo reprodujo hasta 30.96 s desde el navegador instalado. Chromium informó audio decodificado y el control de silencio estaba desactivado. Dos clics en el reproductor confirmaron la pausa y la reanudación. Alt+Izquierda volvió a los resultados en una segunda prueba.

La instalación a un prefijo temporal inició la ventana bajo Wayland. Al volver a abrirla, restauró dos pestañas de prueba. Un arranque posterior con `--efimero` conservó los archivos de sesión, historial y marcadores del perfil de prueba. `chrome://sandbox/` informó namespaces y filtros seccomp activos.

En un contenedor nuevo de Arch, `makepkg --noextract --skipinteg` compiló y empaquetó una copia local de trabajo con los cambios de distribución. `pacman -U` instaló el paquete, `navegador-shell --version` respondió y `desktop-file-validate` aceptó su entrada. La versión final se compiló e instaló directamente en el prefijo del usuario en el equipo de prueba.

## Costo de video

Cinco rondas con orden rotativo en Hyprland anidado sobre Wayland, con una superficie de 941 × 1174. Los navegadores abrieron un perfil nuevo y la misma página HTTP en loopback. La página reproduce un WebM VP9 de 640 × 360 a 60 fps, en bucle y silenciado. El proyecto usó la compilación de escritorio instalada anteriormente como base y la compilación optimizada durante este trabajo como candidata, ambas con Chromium 155.0.8059.84 en modo headless. Brave y Firefox usaron Wayland. Qt WebEngine 6.11.2 se compiló desde el [spike](https://github.com/olanhiram-bot/navegador_rust/tree/5988de03f4e0d32171a8ecb7bc6c172ce3e1dc62/qt). Su ventana se abrió y permaneció visible bajo el compositor durante la prueba.

Después de 2.5 segundos de arranque, se tomaron dos muestras de `utime + stime` de `/proc/PID/stat` separadas por cuatro segundos. El porcentaje expresa tiempo de CPU acumulado del árbol de procesos en unidades de un núcleo. La PSS suma `Pss` en `/proc/PID/smaps_rollup` al final de cada muestra.

| Aplicación | CPU por ronda (%) | CPU mediana (%) | PSS mediana (MiB) |
|---|---|---:|---:|
| GPUI instalada | 228.8, 80.8, 90.3, 163.7, 158.9 | 158.9 | 526.2 |
| GPUI candidata | 165.5, 76.3, 170.9, 150.9, 156.2 | 156.2 | 518.2 |
| Brave 158.1.100.58 | 12.5, 11.0, 13.5, 26.4, 33.6 | 13.5 | 502.9 |
| Firefox 158.0.1 | 13.0, 11.5, 15.7, 158.0, 231.4 | 15.7 | 764.8 |
| Qt WebEngine 6.11.2 | 8.7, 9.2, 11.2, 18.7, 19.4 | 11.2 | 327.8 |

GPUI ahora se despierta con eventos del motor y decodifica JPEG directamente a RGBA. La muestra de video quedó lejos de la meta de reducir CPU un 30 %. El decodificador recibió alrededor de 60 fps en tres de las cinco rondas de la candidata y menos en las otras dos. Hubo carga de compilación durante parte de la medición. Sigue pendiente medir los frames presentados, la latencia del clic, el desplazamiento y el arranque frío y caliente. La prueba de Qt confirma el arranque visible en Wayland. La decisión de cambiar de vista sigue abierta hasta comprobar los requisitos de interacción y aprobación en esa ventana.

En otras cinco rondas alternadas, la versión instalada restauró una y diez pestañas con la misma página local de texto. Tras esperar a que Chromium publicara todas las pestañas y dejar pasar dos segundos, la PSS mediana del árbol fue de 472.7 MiB con una pestaña y 585.4 MiB con diez. Las dos pruebas de memoria usan contenidos distintos. La primera mide video y la segunda, texto.

El ajuste de escala tiene en cuenta los píxeles lógicos del host y la escala de página emulada. El decodificador conserva el formato de píxeles que usa la interfaz de egui y la ventana GPUI convierte a BGRA antes de presentar. Chromium documenta sus [metadatos de screencast](https://chromedevtools.github.io/devtools-protocol/tot/Page/#type-ScreencastFrameMetadata) y el [protocolo de descargas](https://chromedevtools.github.io/devtools-protocol/tot/Browser/#event-downloadProgress).
