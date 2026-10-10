#!/usr/bin/env bash
set -euo pipefail

rm -f \
  "${HOME}/.local/bin/arch-tools" \
  "${HOME}/.local/share/applications/arch-tools.desktop" \
  "${HOME}/.local/share/icons/hicolor/scalable/apps/arch-tools.svg" \
  "${HOME}/.local/bin/arch-display-manager" \
  "${HOME}/.local/share/applications/arch-display-manager.desktop" \
  "${HOME}/.local/share/icons/hicolor/scalable/apps/arch-display-manager.svg"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "${HOME}/.local/share/applications" >/dev/null 2>&1 || true
fi

echo "Arch 工具箱已卸载。"
