#!/usr/bin/env bash
# Instala la shell en el perfil del usuario, sin tocar el sistema.
# Uso: packaging/install.sh [PREFIJO] [--predeterminado]
#   PREFIJO por defecto: ~/.local
#   --predeterminado registra el navegador para http/https con xdg-mime
set -euo pipefail

prefix="$HOME/.local"
predeterminado=0
for arg in "$@"; do
  case "$arg" in
    --predeterminado) predeterminado=1 ;;
    *) prefix="$arg" ;;
  esac
done
if [[ "$prefix" != /* ]]; then
  prefix="$PWD/$prefix"
fi
here="$(cd "$(dirname "$0")/.." && pwd)"

cd "$here"
cargo build --release --locked -p shell-gpui -p navegador

install -Dm755 "${CARGO_TARGET_DIR:-$here/target}/release/navegador-gpui" "$prefix/bin/navegador-shell"
install -Dm755 "${CARGO_TARGET_DIR:-$here/target}/release/navegador" "$prefix/bin/navegador"
install -Dm644 "$here/packaging/navegador.desktop" \
  "$prefix/share/applications/navegador.desktop"
install -Dm644 "$here/packaging/navegador.svg" \
  "$prefix/share/icons/hicolor/scalable/apps/navegador.svg"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
fi

if [ "$predeterminado" = "1" ]; then
  if command -v xdg-mime >/dev/null 2>&1; then
    for scheme in x-scheme-handler/http x-scheme-handler/https text/html; do
      xdg-mime default navegador.desktop "$scheme" || true
    done
    echo "Registrado como navegador predeterminado para http/https."
  else
    echo "xdg-mime no está disponible; no se cambió el navegador predeterminado." >&2
  fi
fi

echo "Instalado en $prefix/bin/navegador-shell"
echo "Abre «Navegador» en el lanzador o ejecuta: $prefix/bin/navegador-shell"
