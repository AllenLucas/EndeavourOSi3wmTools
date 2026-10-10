#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
BIN_DIR="${HOME}/.local/bin"
APP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"
BIN_PATH="${BIN_DIR}/arch-tools"
DESKTOP_PATH="${APP_DIR}/arch-tools.desktop"
ICON_PATH="${ICON_DIR}/arch-tools.svg"
TAURI_MANIFEST="${PROJECT_DIR}/src-tauri/Cargo.toml"
TAURI_BINARY="${PROJECT_DIR}/src-tauri/target/release/arch-tools-desktop"

if ! command -v cargo >/dev/null 2>&1; then
  echo "错误：未安装 Rust/Cargo。Arch Linux 可运行：sudo pacman -S rust" >&2
  exit 1
fi
if ! command -v xrandr >/dev/null 2>&1; then
  echo "警告：未找到 xrandr。安装后才能管理 X11 显示器：sudo pacman -S xorg-xrandr" >&2
fi

missing=()
for library in webkit2gtk-4.1 javascriptcoregtk-4.1 gtk+-3.0 libsoup-3.0; do
  pkg-config --exists "$library" 2>/dev/null || missing+=("$library")
done
if ((${#missing[@]})); then
  echo "错误：缺少 Tauri Linux 构建依赖：${missing[*]}" >&2
  echo "Arch Linux 通常可运行：sudo pacman -S webkit2gtk-4.1 gtk3 base-devel" >&2
  exit 1
fi

echo "[1/3] 正在构建 Tauri release 桌面应用……"
cd "$PROJECT_DIR"
cargo build --release --manifest-path "$TAURI_MANIFEST"

echo "[2/3] 正在安装当前用户的桌面应用……"
rm -f \
  "${BIN_DIR}/arch-display-manager" \
  "${APP_DIR}/arch-display-manager.desktop" \
  "${ICON_DIR}/arch-display-manager.svg"
install -Dm755 "$TAURI_BINARY" "$BIN_PATH"
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
Exec=${BIN_PATH}
TryExec=${BIN_PATH}
Icon=arch-tools
Terminal=false
Categories=Settings;System;
Keywords=arch;tools;display;monitor;wallpaper;xrandr;
StartupNotify=true
SingleMainWindow=true
EOF
chmod 644 "$DESKTOP_PATH"

if [[ ! -x "$BIN_PATH" ]]; then
  echo "错误：应用可执行文件安装失败：$BIN_PATH" >&2
  exit 1
fi
if ! grep -Fqx "Exec=${BIN_PATH}" "$DESKTOP_PATH"; then
  echo "错误：桌面入口生成失败：$DESKTOP_PATH" >&2
  exit 1
fi

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "[3/3] 安装完成。"
echo
echo "从应用菜单搜索“Arch 工具箱”并点击即可启动。"
echo "桌面版不会打开终端或外部浏览器；关闭窗口后 Rust 进程随即结束。"
echo "也可在任意目录运行：${BIN_PATH}"
