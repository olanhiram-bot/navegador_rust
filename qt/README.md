# Navegador Qt experimental

Esta variante usa la vista nativa de Qt WebEngine. Requiere Qt 6.8 o posterior, qmake6, make y un escritorio Linux. Compílala desde la raíz del repositorio.

```sh
source_dir="$PWD/qt"
build_dir="$(mktemp -d)"
(cd "$build_dir" && qmake6 "$source_dir/navegador-qt.pro" && make)
"$build_dir/navegador-qt" --efimero
```

En un arranque normal usa su propio perfil y restaura las pestañas HTTP y HTTPS al volver a abrir la ventana. `--efimero` guarda las cookies y los permisos en memoria. `--sin-restaurar` abre una sesión nueva con el perfil persistente. `NAVEGADOR_DESCARGAS` permite elegir la carpeta inicial del diálogo de descarga.

La ventana ofrece navegación con teclado, búsqueda dentro de la página, zoom por pestaña, confirmación de permisos por sitio y progreso y cancelación por descarga. El menú **Privacidad** permite autorizar almacenamiento de terceros para el origen actual. La protección predeterminada usa el filtro nativo de Qt WebEngine. [Documentación del filtro](https://doc.qt.io/QT-6/qwebenginecookiestore.html#setCookieFilter)

Para pruebas locales del agente, `QTWEBENGINE_REMOTE_DEBUGGING=127.0.0.1:9333` habilita CDP en loopback. La decisión de integrar el agente y distribuir Qt depende de la prueba de paridad y del comparador. [Paridad Qt #10](https://github.com/olanhiram-bot/navegador_rust/issues/10)
