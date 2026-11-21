#!/usr/bin/env bash
# Instala la shell en el perfil del usuario, sin tocar el sistema.
# Uso: packaging/install.sh [PREFIJO]   (por defecto ~/.local)
set -euo pipefail

prefix="${1:-$HOME/.local}"
here="$(cd "$(dirname "$0")/.." && pwd)"

cargo build --release --locked -p shell

install -Dm755 "$here/target/release/navegador-shell" "$prefix/bin/navegador-shell"
install -Dm644 "$here/packaging/navegador.desktop" \
  "$prefix/share/applications/navegador.desktop"

echo "Instalado en $prefix/bin/navegador-shell"
echo "Abre «Navegador» en el lanzador o ejecuta: $prefix/bin/navegador-shell"
