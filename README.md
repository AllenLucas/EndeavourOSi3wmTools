# Arch Tools

面向 Arch Linux 的轻量本机桌面工具，仅包含 **投屏管理** 和 **壁纸管理**。后端使用 Rust，桌面容器使用 Tauri 2，前端为无构建步骤的原生 HTML/CSS/JavaScript；同时保留 Axum 浏览器模式用于开发调试。

## 功能

### 投屏管理

- 检测 Linux、X11/Wayland、`DISPLAY` 和 `xrandr`
- 查询显示器连接状态、分辨率、刷新率、坐标、旋转和主显示器
- 拖拽排列显示器，支持边缘吸附和水平自动排列
- 后端按当前硬件状态校验配置，命令使用参数数组，不拼接 Shell
- 防止关闭所有显示器
- 应用后 15 秒内确认；超时或关闭桌面窗口时自动恢复旧布局

当前仅支持修改 X11 显示配置。Wayland 会显示环境提示，但不会执行修改。

### 壁纸管理

- 使用 `feh` 设置 X11 壁纸，支持填充、拉伸、最大化、居中和平铺
- 使用 `yad` 选择本地图片，也可直接输入完整路径
- 在界面中预览不超过 12 MB 的图片
- 可将壁纸启动命令写入 i3 配置；默认路径为 `~/.config/i3/config`，可用 `I3_CONFIG` 覆盖
- 可在默认浏览器中打开 Pexels 壁纸库

## 依赖

Tauri 桌面版的构建依赖：

```bash
sudo pacman -S rust xorg-xrandr webkit2gtk-4.1 gtk3 base-devel
```

壁纸功能的可选运行时依赖：

```bash
sudo pacman -S feh yad xdg-utils
```

缺少可选依赖时，相关操作会显示明确提示，不影响投屏管理。

## 安装桌面应用

```bash
./install.sh
```

脚本会构建 release 版本，并安装：

- 可执行文件：`~/.local/bin/arch-tools`
- 应用菜单入口：`~/.local/share/applications/arch-tools.desktop`
- 图标：`~/.local/share/icons/hicolor/scalable/apps/arch-tools.svg`

卸载：

```bash
./uninstall.sh
```

## 开发运行

请从目标桌面用户的 X11 会话终端运行，确保 `DISPLAY` 可用。

### Tauri 桌面版

```bash
cargo run --manifest-path src-tauri/Cargo.toml
```

### 浏览器调试模式

```bash
cargo run -- --open
```

浏览器模式仅监听 `127.0.0.1`。可用以下环境变量调整行为：

- `ARCH_TOOLS_PORT=9000`：修改监听端口
- `ARCH_TOOLS_OPEN_BROWSER=1`：启动后打开浏览器
- `ARCH_TOOLS_XRANDR=/path/to/mock`：替换 xrandr 程序，便于测试

## 测试

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo check --manifest-path src-tauri/Cargo.toml
```

## 安全约束

- Tauri 模式通过 IPC 调用后端，不开放 HTTP 端口
- 浏览器模式固定绑定本机回环地址
- 系统命令通过程序和参数数组执行，不使用 `sh -c`
- 显示配置在执行前会重新读取硬件状态并校验输出、模式、刷新率和主显示器
- 浏览器模式的修改请求要求 JSON Content-Type，并校验浏览器 Origin
