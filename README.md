# Arch Tools

面向 Arch Linux / EndeavourOS 的本机显示与壁纸管理工具。项目采用简单的前后端分离结构：

- **H5 + CSS + JavaScript**：只负责图形界面和用户交互
- **Rust + Axum**：提供本机 HTTP API，执行全部系统功能
- 前端资源编译进 Rust 可执行文件，不需要 Node.js、前端构建工具或额外的 GUI 容器

服务只监听 `127.0.0.1`，不会向局域网开放管理接口。

## 功能

### 投屏管理

- 检测 Linux、X11/Wayland、`DISPLAY` 和 `xrandr`
- 查询显示器连接状态、分辨率、刷新率、坐标、旋转和主显示器
- 拖拽排列显示器，支持边缘吸附和水平自动排列
- 后端根据当前硬件能力校验配置，外部命令不经过 Shell
- 防止关闭所有显示器
- 应用后 15 秒内确认，超时自动恢复
- 等待确认时关闭或离开页面，前端会请求立即恢复；即使浏览器未能发送请求，15 秒定时器仍会兜底恢复

目前仅支持修改 X11 显示配置。Wayland 会显示环境提示，但不会执行修改。

### 壁纸管理

- 使用 `feh` 设置 X11 壁纸，支持填充、拉伸、最大化、居中和平铺
- 使用 `yad` 选择本地图片，也可直接输入完整路径
- 在页面中预览不超过 12 MB 的图片
- 可将壁纸启动命令写入 i3 配置；默认路径为 `~/.config/i3/config`，可用 `I3_CONFIG` 覆盖
- 可在默认浏览器中打开 Pexels 壁纸库

## 架构

```text
浏览器中的 H5 / JavaScript
           │ fetch JSON
           ▼
127.0.0.1:8787 上的 Rust / Axum
           │
           ├── xrandr：显示器查询与配置
           ├── feh：设置壁纸
           ├── yad：选择图片
           └── xdg-open：打开外部页面
```

HTTP 服务和 H5 页面由同一个 `arch-tools` 进程提供。前端不直接访问系统命令或文件。

## 安装

安装依赖：

```bash
sudo pacman -S --needed rust xorg-xrandr feh yad xdg-utils
```

克隆并安装：

```bash
git clone git@github.com:AllenLucas/EndeavourOSi3wmTools.git
cd EndeavourOSi3wmTools
./install.sh
```

安装脚本只执行 `cargo build --release`，不需要 GTK/WebKitGTK 等桌面容器依赖。安装位置：

- `~/.local/bin/arch-tools`
- `~/.local/share/applications/arch-tools.desktop`
- `~/.local/share/icons/hicolor/scalable/apps/arch-tools.svg`

之后可在应用菜单搜索 **Arch 工具箱**。启动器会运行本机服务并打开默认浏览器。如果服务已经运行，再次点击只会重新打开页面。

也可以从终端启动：

```bash
arch-tools --open
```

停止服务：

```bash
pkill -x arch-tools
```

卸载：

```bash
./uninstall.sh
```

## 开发运行

从目标桌面用户的 X11 会话终端执行：

```bash
cargo run -- --open
```

不自动打开浏览器：

```bash
cargo run
# 然后访问 http://127.0.0.1:8787
```

环境变量：

- `ARCH_TOOLS_PORT=9000`：修改本机监听端口
- `ARCH_TOOLS_OPEN_BROWSER=1`：启动后打开浏览器
- `ARCH_TOOLS_XRANDR=/path/to/mock`：替换 xrandr 程序，便于测试
- `I3_CONFIG=/path/to/i3/config`：修改 i3 配置文件位置

## 测试

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
node --check web/app.js
```

## 安全约束

- HTTP 服务固定绑定 `127.0.0.1`
- 拒绝非回环 Host，降低 DNS rebinding 风险
- 修改请求要求 `application/json`，并校验浏览器 Origin
- 系统命令使用程序名和参数数组执行，不使用 `sh -c`
- 显示配置执行前会重新读取硬件状态并校验输出、模式、刷新率和主显示器
- 显示配置应用后会查询真实状态；失败或不一致时自动回滚
