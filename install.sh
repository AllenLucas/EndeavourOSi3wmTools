#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
BIN_DIR="${HOME}/.local/bin"
APP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"
BIN_PATH="${BIN_DIR}/arch-tools"
DESKTOP_PATH="${APP_DIR}/arch-tools.desktop"
ICON_PATH="${ICON_DIR}/arch-tools.svg"
BINARY="${PROJECT_DIR}/target/release/arch-tools"

if ! command -v cargo >/dev/null 2>&1; then
  echo "错误：未安装 Rust/Cargo。Arch Linux 可运行：sudo pacman -S rust" >&2
  exit 1
fi
if ! command -v xrandr >/dev/null 2>&1; then
  echo "警告：未找到 xrandr。安装后才能管理 X11 显示器：sudo pacman -S xorg-xrandr" >&2
fi
if ! command -v xdg-open >/dev/null 2>&1; then
  echo "警告：未找到 xdg-open。安装 xdg-utils 后才能自动打开管理页面。" >&2
fi

printf '%s\n' "[1/3] 正在构建 Rust release 服务……"
cd "$PROJECT_DIR"
cargo build --release

printf '%s\n' "[2/3] 正在安装当前用户的应用……"
rm -f \
  "${BIN_DIR}/arch-display-manager" \
  "${APP_DIR}/arch-display-manager.desktop" \
  "${ICON_DIR}/arch-display-manager.svg"
install -Dm755 "$BINARY" "$BIN_PATH"
install -Dm644 "${PROJECT_DIR}/assets/arch-tools.svg" "$ICON_PATH"
mkdir -p "$APP_DIR"
cat >"$DESKTOP_PATH" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=Arch Tools
Name[zh_CN]=Arch 工具箱
Comment=Manage displays and wallpapers on Arch Linux
Comment[zh_CN]=管理 Arch Linux 显示器与壁纸
Exec=${BIN_PATH} --open
TryExec=${BIN_PATH}
Icon=arch-tools
Terminal=false
Categories=Settings;System;
Keywords=arch;tools;display;monitor;wallpaper;xrandr;
StartupNotify=true
EOF
chmod 644 "$DESKTOP_PATH"

if [[ ! -x "$BIN_PATH" ]]; then
  echo "错误：应用可执行文件安装失败：$BIN_PATH" >&2
  exit 1
fi
if ! grep -Fqx "Exec=${BIN_PATH} --open" "$DESKTOP_PATH"; then
  echo "错误：桌面入口生成失败：$DESKTOP_PATH" >&2
  exit 1
fi

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

printf '%s\n\n' "[3/3] 安装完成。"
printf '%s\n' "从应用菜单搜索“Arch 工具箱”，应用会启动本机 Rust 服务并打开浏览器页面。"
printf '%s\n' "也可在终端运行：${BIN_PATH} --open"
