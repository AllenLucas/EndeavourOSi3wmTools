# 投屏管理与壁纸管理重写技术文档

> 本文基于当前项目实际代码整理，目标是让另一台设备上的 AI agent 可以据此独立重写“投屏管理”和“壁纸管理”两个模块。文档中的“当前实现”描述现有行为；“重写要求”描述建议保持的外部契约和安全约束。

## 1. 范围与现状

本项目是 Arch Linux 本机工具箱，核心语言为 Rust，桌面容器为 Tauri 2，前端为原生 HTML/CSS/JavaScript。两个目标模块均只针对 X11：

- 投屏管理通过 `xrandr` 读取和修改显示器输出。
- 壁纸管理通过 `feh` 设置壁纸，通过 `yad` 选择图片，通过 `xdg-open` 打开 Pexels；可选将启动命令保存到 i3 配置。

Wayland 当前不修改显示器；壁纸模块也依赖 X11 桌面工具链，未实现 Wayland 壁纸后端。

当前代码位置：

| 责任 | 文件 |
| --- | --- |
| 投屏数据结构 | `src/model.rs` |
| xrandr 调用、输出解析、参数生成 | `src/xrandr.rs` |
| 投屏业务流程、校验、回滚、HTTP handler | `src/api.rs` |
| 壁纸命令、预览、i3 持久化 | `src/wallpaper.rs` |
| Axum 浏览器模式路由 | `src/main.rs` |
| Tauri IPC 命令和窗口关闭回滚 | `src-tauri/src/main.rs` |
| 前端 IPC/fetch 适配、显示器编辑、拖拽、壁纸操作 | `web/app.js` |
| 页面结构 | `web/index.html` |
| 页面样式 | `web/style.css` |
| 统一错误类型 | `src/error.rs` |

## 2. 技术栈

### 2.1 后端

- Rust 2024 edition。
- Tokio：异步文件、进程和定时任务。
- `serde`：请求和响应数据序列化（Axum 负责 JSON 传输）。
- Axum 0.8：浏览器兼容模式的 HTTP API。
- Tauri 2：桌面端 IPC 命令和窗口生命周期。
- `thiserror`：错误枚举和错误文本。
- `tracing`：日志。
- Linux 外部程序：`xrandr`、`feh`、`yad`、`xdg-open`。

### 2.2 前端

- 原生 HTML/CSS/JavaScript，无前端框架和构建步骤。
- 运行在 Tauri WebView 中时，使用 `window.__TAURI__.core.invoke`。
- 运行在浏览器模式时，使用 `fetch` 访问 `127.0.0.1` 上的 Axum API。
- 前端通过同一套 `api(path, options)` 函数屏蔽 Tauri IPC 和 HTTP 两种传输方式。

### 2.3 系统依赖

必需或按模块安装：

```text
rust
xorg-xrandr   # 投屏管理
feh           # 设置壁纸
yad           # 图形文件选择器
xdg-utils     # 打开浏览器
GTK3/WebKitGTK # Tauri 窗口运行环境
```

当前 README 给出的 Arch 安装命令为：

```bash
sudo pacman -S rust xorg-xrandr webkit2gtk-4.1 gtk3 base-devel
sudo pacman -S feh yad xdg-utils
```

运行进程必须继承目标桌面会话的环境，尤其是 `DISPLAY`。`xrandr` 还需要能够访问对应的 X11 display/socket。

## 3. 总体架构

```text
HTML/JS UI
   |
   | Tauri invoke（桌面）或 fetch（浏览器调试）
   v
传输适配层 api()
   |
   +--> Tauri command: displays/apply/... 或 wallpaper_set/...
   |       |
   |       +--> 复用 src/api.rs
   |       +--> 复用 src/xrandr.rs / src/wallpaper.rs
   |
   +--> Axum: /api/displays/... 或 /api/wallpaper/...
           |
           +--> 复用同一业务代码

xrandr / feh / yad / xdg-open
```

投屏状态由 `AppState` 持有：

- `xrandr: XrandrManager`：封装外部命令。
- `pending: Arc<Mutex<Option<PendingChange>>>`：最多同时存在一个等待确认的显示配置。
- `operation: Arc<Mutex<()>>`：串行化显示配置应用、确认、恢复和超时任务。

壁纸没有持久化的服务端状态；每次设置都是独立请求。图片路径和 i3 配置路径来自请求或环境变量，不保存到应用数据库。

## 4. 投屏管理：数据模型

所有字段名经过 serde `rename_all = "camelCase"` 后对外使用 camelCase。

### 4.1 `Display`

```json
{
  "name": "HDMI-1",
  "connected": true,
  "enabled": true,
  "primary": false,
  "currentMode": {
    "width": 1920,
    "height": 1080,
    "refreshRate": 60.0
  },
  "position": { "x": 1920, "y": 0 },
  "rotation": "normal",
  "physicalSize": { "widthMm": 340, "heightMm": 190 },
  "modes": [
    {
      "width": 1920,
      "height": 1080,
      "refreshRates": [
        { "value": 60.0, "current": true, "preferred": true }
      ],
      "current": true,
      "preferred": true
    }
  ]
}
```

含义：

- `name`：xrandr output 名称，例 `eDP-1`、`HDMI-1`。
- `connected`：物理输出是否 connected。断开的输出也会保留在查询结果中。
- `enabled`：当前是否存在活动几何信息。断开输出必然为 false。
- `primary`：xrandr header 是否带 `primary`。
- `currentMode`：当前活动模式；未启用时为 null。
- `position`：当前几何坐标；未启用时为 null。
- `rotation`：`normal`、`left`、`right`、`inverted`。
- `physicalSize`：从 xrandr 输出中的 `Nmm x Nmm` 解析，可能为 null。
- `modes`：该输出支持的分辨率集合；相同分辨率的多条模式会合并刷新率。
- `current`/`preferred` 刷新率通过 xrandr 模式 token 中的 `*`/`+` 判断。

### 4.2 `DisplayConfiguration`

这是前端草稿和 apply/validate 请求使用的配置：

```json
{
  "output": "HDMI-1",
  "enabled": true,
  "mode": {
    "width": 1920,
    "height": 1080,
    "refreshRate": 60.0
  },
  "x": 1920,
  "y": 0,
  "primary": false,
  "rotation": "normal"
}
```

`mode` 可以为 null，表示使用 `xrandr --auto`；非 null 时生成 `--mode WIDTHxHEIGHT`，如果 `refreshRate` 非 null 再生成 `--rate RATE`。

### 4.3 其他响应

`GET /api/displays`：

```json
{
  "displays": [/* Display[] */],
  "pending": null
}
```

`pending` 不为 null 时：

```json
{ "token": "...", "expiresAt": 1710000000 }
```

`POST /api/displays/validate`：

```json
{
  "valid": true,
  "errors": [],
  "warnings": [],
  "displays": [/* 当前真实状态 */]
}
```

`POST /api/displays/apply`：

```json
{
  "success": true,
  "confirmationRequired": true,
  "token": "...",
  "expiresAt": 1710000000,
  "message": "配置已应用，请在 15 秒内确认",
  "displays": [/* 应用并重新查询后的状态 */]
}
```

确认和恢复响应：

```json
{
  "success": true,
  "message": "已保留当前显示设置",
  "displays": [/* 最新真实状态 */]
}
```

## 5. 投屏管理：xrandr 读取和解析

### 5.1 环境检查

`XrandrManager::session_type()` 优先读取 `XDG_SESSION_TYPE`；若没有该变量但存在 `DISPLAY`，回退为 `x11`，否则为 `unknown`。

`ensure_usable()` 的顺序：

1. 如果 session type 是 `wayland`，返回“不支持会话”。
2. 执行 `${binary} --version` 检查 xrandr 是否可用。
3. 失败返回未安装。

xrandr 可执行文件默认是 `xrandr`，测试时可通过以下环境变量替换：

```text
ARCH_TOOLS_XRANDR=/path/to/mock
ADM_XRANDR=/path/to/mock
```

优先级是 `ARCH_TOOLS_XRANDR`，其次 `ADM_XRANDR`，最后 `xrandr`。

### 5.2 查询命令

执行：

```bash
xrandr --query
```

调用使用 `tokio::process::Command` 的参数数组，不经过 shell。非零退出时读取 stderr；识别 `cannot open display` 和 `unable to open display` 为 display 不可用，其余归为查询失败。

### 5.3 解析规则

典型输入：

```text
Screen 0: current 2944 x 1080, maximum 8192 x 8192
 eDP-1 connected primary 1920x1080+0+0 (normal left inverted right x axis y axis) 309mm x 174mm
    1920x1080     60.01*+  59.93
 HDMI-1 connected 1024x768+1920+0 (normal left inverted right x axis y axis) 340mm x 190mm
    1920x1080     60.00 +
    1024x768      75.03*  70.07  60.00
 DP-1 disconnected (normal left inverted right x axis y axis)
```

解析算法：

1. 只将非空白开头、第二个 token 为 `connected` 或 `disconnected` 的行识别为 output header。
2. 在 header 中寻找类似 `1920x1080+0+0` 的几何 token，解析宽、高、带正负号的 x/y。
3. header 中出现 `primary` 即设置 `primary=true`。
4. 几何存在即认为 `enabled=true`，否则 false。
5. 解析 header 中括号前、几何之后的独立 rotation token。未写 rotation 时默认为 normal。
6. 解析括号后的物理尺寸 `Nmm x Nmm`。
7. 解析缩进的模式行。分辨率 token 允许后接 `i`、`p`、`+`、`-` 等标记；刷新率读取数字前缀，忽略异常 token。
8. 同一输出中相同宽高的 mode 合并，把刷新率按 0.01 Hz 内视为同一个值，并合并 current/preferred 标记。
9. 对 left/right 旋转：xrandr header 的活动几何已经是旋转后的宽高，但 mode 行是面板原生方向。解析完成后用旋转后的尺寸匹配当前 mode，再将 `currentMode` 的宽高交换回原生方向。

必须支持负坐标读取，因为真实多屏布局可能有负坐标；当前“写入配置”的校验则限制坐标不能为负，见后文边界说明。

## 6. 投屏管理：xrandr 写入

### 6.1 参数生成

`build_xrandr_args(configurations)` 生成参数数组，不做 shell 拼接。顺序非常重要：

1. 先遍历所有 enabled 输出。
2. 再遍历所有 disabled 输出。
3. 这样先启用目标显示器，再关闭其他显示器，降低短暂黑屏风险。

启用输出的参数形态：

```text
--output OUTPUT
--mode WIDTHxHEIGHT       # mode 非 null
--rate RATE               # refreshRate 非 null
--auto                    # mode 为 null
--pos XxY
--rotate normal|left|right|inverted
--primary                 # primary=true 时
```

禁用输出：

```text
--output OUTPUT --off
```

刷新率格式化为最多 3 位小数，并删除末尾 0，例如 `60.000` -> `60`。

示例：

```bash
xrandr \
  --output HDMI-1 --mode 1920x1080 --rate 60 --pos 0x0 --rotate normal --primary \
  --output VGA1 --off
```

重写时必须保留：

- 永远使用进程参数数组，不使用 `sh -c`。
- output 名称来自后端验证过的真实已连接 output，而不是直接信任任意字符串。
- 先配置 enabled，后处理 disabled。
- 同时传递位置、旋转、主显示器和模式信息。

## 7. 投屏管理：请求校验

`validate_configuration` 会把请求与一次新的真实 xrandr 查询结果比较。

错误条件：

1. 配置数组为空。
2. output 重复。
3. output 不在当前已连接输出集合中。
4. x 或 y 小于 0。
5. x 或 y 大于 32767。
6. disabled 输出被标记为 primary。
7. 指定的分辨率不存在于该 output 的 mode 列表。
8. 指定刷新率不是该分辨率支持的刷新率，或不是 finite/positive。
9. `mode=null` 但该 output 没有任何 mode。
10. 请求漏掉任何一个当前已连接 output。
11. 所有输出都 disabled。
12. enabled 输出不是恰好一个 primary。

警告条件：

- 两个 enabled 输出具有相同 x/y。后端允许，这可能表示镜像或重叠布局。

刷新率比较容差是 `0.05 Hz`。显示器模式相同分辨率下的刷新率比较在解析合并时使用约 `0.01 Hz`。

注意：当前校验不检查矩形之间的一般重叠，只检查相同坐标；不检查最大 framebuffer、不检查 xrandr 的屏幕边界，也不校验 rotation 字符串以外的额外语义，因为 rotation 使用 Rust enum 反序列化。

## 8. 投屏管理：应用、实测验证与回滚

`POST /api/displays/apply` 的完整流程：

1. 获取全局 `operation` 锁。
2. 如果已有 pending 配置，拒绝并返回 conflict；必须先确认或恢复。
3. `ensure_usable()` 检查 X11/xrandr。
4. 查询当前 displays，得到真实基线 `before`。
5. 使用基线执行校验；不要只相信前端之前读取的列表。
6. 从基线生成 `previous: Vec<DisplayConfiguration>`，作为恢复快照。
7. 执行 xrandr apply。
8. 如果 apply 失败，立即尝试用 `previous` apply 恢复：
   - 恢复成功：返回“应用失败，已恢复原设置”。
   - 恢复失败：记录两个错误并返回 internal。
9. apply 成功后重新执行 `xrandr --query`。
10. 用 `verify_applied_configuration` 比较请求与真实状态：enabled、primary、坐标、rotation、分辨率、刷新率都要一致；还必须至少有一个 enabled output。
11. 实测验证失败时，执行 previous 回滚；回滚失败则返回 internal。
12. 验证成功后生成 token，并设置当前 Unix 秒时间戳 + 15 秒作为 `expiresAt`。
13. 保存唯一 pending：`{ token, expires_at, previous }`。
14. 启动 15 秒 Tokio 定时任务。
15. 返回新状态和 confirmation 信息。

### 8.1 确认

`POST /api/displays/confirm` 请求：

```json
{ "token": "...", "expiresAt": 1710000000 }
```

只实际使用 token；expiresAt 用于协议形状和前端倒计时。要求 token 与当前 pending 相同，然后清除 pending，不再回滚，并重新查询返回真实状态。

### 8.2 手动恢复

`POST /api/displays/revert` 要求 token 正确，先取出 pending，再把 previous 直接交给 xrandr，查询并返回新状态。

### 8.3 超时恢复

定时任务等待 15 秒后获取 operation 锁：

- token 仍是当前 pending：取出 previous 并应用。
- token 已被确认/手动恢复或替换：什么也不做。
- 自动恢复失败：记录错误日志。当前不会再尝试第二次恢复。

### 8.4 窗口关闭恢复

Tauri 收到 `CloseRequested` 时调用 `prevent_close()`，异步执行 `AppState::revert_pending_on_shutdown()`，完成后 `app.exit(0)`。因此窗口关闭不会绕过尚未确认的布局恢复。

浏览器模式没有同等的进程关闭钩子；服务进程被强制终止时无法保证回滚。

## 9. 投屏管理：前端行为

前端状态：

```text
state = {
  system: SystemStatus | null,
  displays: Display[],
  draft: DisplayConfiguration[],
  pending: PendingInfo | null,
  timer: interval id | null
}
```

页面加载时并行请求 system status 和 displays；draft 只包含 connected outputs，并从当前真实状态复制。

功能：

- 每个已连接输出可设置启用、分辨率、刷新率、x/y、旋转、primary。
- 启用一个没有 mode 的输出时自动选 preferred mode 和 preferred rate。
- 关闭当前 primary 时前端自动把 primary 转给另一个 enabled output；后端仍会再次校验。
- “水平自动排列”使用每个输出的 preferred mode，从 x=0、y=0 开始水平排列，第一台 enabled output 为 primary；旋转 90 度时使用交换后的宽度推进 x。
- 布局预览根据 mode 和 rotation 计算矩形尺寸，显示相对最小 x/y 的缩略图。
- 拖拽使用 Pointer Events；拖动时按缩放比例换算为逻辑像素，结束后取整、限制到非负坐标，并向其他显示器边缘吸附。
- `normalizeDraftCoordinates()` 会把 enabled 集合的最小 x/y 平移到 0/0。
- 应用成功后显示确认条并每 250ms 更新倒计时。
- 倒计时到 0 后刷新 displays，并提示已恢复。
- pending 存在时禁止编辑、验证、应用和自动排列。

重写 UI 时，不能只实现“调用 xrandr”按钮；必须保留确认条和超时恢复提示，否则用户可能在错误布局下失去桌面。

## 10. 投屏 API 清单

### 浏览器模式 Axum

| 方法 | 路径 | 请求 | 作用 |
| --- | --- | --- | --- |
| GET | `/api/system/status` | 无 | 返回 OS、会话、DISPLAY、xrandr 可用性和可修改性 |
| GET | `/api/displays` | 无 | 查询显示器和 pending |
| POST | `/api/displays/validate` | `{displays: DisplayConfiguration[]}` | 只校验，不修改 |
| POST | `/api/displays/apply` | `{displays: DisplayConfiguration[]}` | 应用并进入确认阶段 |
| POST | `/api/displays/confirm` | `PendingInfo` | 保留当前布局 |
| POST | `/api/displays/revert` | `PendingInfo` | 恢复旧布局 |

### Tauri 命令

| 命令 | 参数 |
| --- | --- |
| `system_status` | 无 |
| `displays` | 无 |
| `validate_displays` | `{request: DisplayConfigurationRequest}` |
| `apply_displays` | `{request: DisplayConfigurationRequest}` |
| `confirm_displays` | `{request: PendingInfo}` |
| `revert_displays` | `{request: PendingInfo}` |

Tauri 命令将业务错误转换成字符串；HTTP 模式则返回：

```json
{
  "error": {
    "code": "bad_request|not_found|conflict|service_unavailable|internal_error",
    "message": "中文错误信息"
  }
}
```

## 11. 壁纸管理：数据模型和调用链

### 11.1 请求

`WallpaperRequest`：

```json
{
  "path": "/home/user/Pictures/wallpaper.jpg",
  "mode": "fill",
  "persist": true
}
```

字段：

- `path`：图片完整路径，必须是普通文件。
- `mode`：默认 `fill`，可选 `fill`、`scale`、`max`、`center`、`tile`。
- `persist`：是否改写 i3 配置，默认 false；当前 UI 默认勾选 true。

模式到 feh 参数：

| mode | feh 参数 | 行为 |
| --- | --- | --- |
| `fill` | `--bg-fill` | 填充并保持比例，可能裁剪 |
| `scale` | `--bg-scale` | 拉伸铺满 |
| `max` | `--bg-max` | 最大化但不裁剪 |
| `center` | `--bg-center` | 居中 |
| `tile` | `--bg-tile` | 平铺 |

任何其他 mode 返回 bad request。

### 11.2 设置壁纸流程

`wallpaper::set`：

1. `PathBuf::from(request.path)`。
2. `path.is_file()`，不存在返回 not found。
3. 将 mode 映射成固定白名单参数。
4. 通过 PATH 检查 `feh` 是否存在。
5. 执行：

```bash
feh --bg-fill /home/user/Pictures/wallpaper.jpg
```

使用 Tokio `Command` 参数数组，stdin 为 null；不使用 shell。

6. feh 非零退出时读取 stderr，返回 conflict。
7. `persist=true` 时改写 i3 配置。
8. 读取图片预览，返回 data URL。
9. 返回成功消息、规范化显示路径和 preview。

### 11.3 壁纸响应

```json
{
  "success": true,
  "message": "壁纸已设置并保存到 i3 配置",
  "path": "/home/user/Pictures/wallpaper.jpg",
  "preview": "data:image/jpeg;base64,..."
}
```

`select` 取消时不是 HTTP 错误，而是：

```json
{
  "success": false,
  "message": "已取消选择壁纸",
  "path": null,
  "preview": null
}
```

### 11.4 预览生成

预览限制为 12 MiB：

```text
MAX_PREVIEW_BYTES = 12 * 1024 * 1024
```

流程：

1. 异步读取 metadata，超过限制返回 bad request。
2. 异步读取完整文件。
3. 根据扩展名选择 MIME：jpg/jpeg、png、webp、gif；其他使用 `application/octet-stream`。
4. 使用 `spawn_blocking` 做 Base64 编码。
5. 拼接 `data:{mime};base64,{encoded}`。

当前实现没有真正验证文件内容是否为图片，也没有解码、缩放或压缩图片；只根据扩展名提供 MIME。

重要现状：`feh` 已经设置成功后，如果图片超过 12 MiB 或预览读取失败，整个请求仍会返回错误。因此重写时应明确选择：要保持当前兼容行为，还是把预览错误降级为 warning，避免“实际已设置但 UI 显示失败”。

## 12. 壁纸文件选择器

`POST /api/wallpaper/select`：

1. 检查 `yad` 是否在 PATH。
2. 执行：

```bash
yad --file --title=选择壁纸 --image-filter=图片 --width=900 --height=620
```

3. 正常退出时读取 stdout，去除首尾空白，转成路径。
4. 路径必须是文件。
5. 生成预览并返回 path/preview；此操作不会调用 feh，也不会修改 i3。
6. 退出码 1 或 252 视为用户取消，返回 `success=false`；其他失败读取 stderr 并返回 unavailable。

前端将返回 path 填入输入框，预览放入 `<img>`。用户也可以不使用 yad，直接输入完整路径。

## 13. 壁纸 i3 持久化

配置路径选择：

1. 如果设置了 `I3_CONFIG`，使用该值。
2. 否则使用 `$HOME/.config/i3/config`。
3. 如果没有 HOME 且没有 I3_CONFIG，返回无法确定路径。

命令格式：

```text
exec --no-startup-id sleep 1 && feh --bg-fill '/home/user/Pictures/wallpaper.jpg'
```

路径通过单引号包裹；单引号本身替换为 ` '\'' ` 形式，以便作为 shell 命令文本写入配置时不破坏命令。

修改算法：

1. 读取整个配置文本。
2. 按行拆分。
3. 查找第一条“非注释行且包含 `feh --bg-`”的行。
4. 保留该行原有前导空白，整行替换成新的命令。
5. 如果没有找到，则追加空行、`# set wallpaper` 和新命令。
6. 如果原文件以换行结尾，更新后保留结尾换行。
7. 将更新内容写入同目录临时文件，继承原文件权限，再通过 rename 原子替换配置；失败时清理临时文件。

它只识别包含精确子串 `feh --bg-` 的 active 行，不会解析完整 i3 配置语法；应保持“注释行不替换、第一条 active feh wallpaper 行原地替换”的兼容规则。

## 14. 壁纸外部搜索

`POST /api/wallpaper/search` 调用：

```bash
xdg-open https://www.pexels.com/zh-cn/
```

stdin/stdout/stderr 全部接到 null，并立即返回成功；它不下载图片，也不把 Pexels 内容导入应用。失败返回 unavailable。

## 15. 壁纸 API 清单

### 浏览器模式

| 方法 | 路径 | 请求 | 作用 |
| --- | --- | --- | --- |
| POST | `/api/wallpaper/set` | `WallpaperRequest` | 调用 feh，可持久化并返回预览 |
| POST | `/api/wallpaper/select` | 空 JSON 或无 body | 打开 yad 并返回选择结果 |
| POST | `/api/wallpaper/search` | 空 JSON 或无 body | 用 xdg-open 打开 Pexels |

### Tauri 命令

| 命令 | 参数 |
| --- | --- |
| `wallpaper_set` | `{request: WallpaperRequest}` |
| `wallpaper_select` | 无 |
| `wallpaper_search` | 无 |

## 16. 浏览器模式与桌面模式差异

浏览器模式由 `src/main.rs` 启动 Axum，固定监听 `127.0.0.1`，默认端口 8787；可用 `ARCH_TOOLS_PORT` 修改。请求通过 Origin/Content-Type guard。浏览器模式适合开发和 mock xrandr。

Tauri 模式由 `src-tauri/src/main.rs` 启动单窗口应用，不开放 HTTP 端口。前端调用 IPC，Tauri command 直接调用 Rust 业务函数。桌面模式关闭窗口时会执行投屏 pending 回滚。

重写时推荐保持“业务层与传输层分离”：

```text
DisplayService / WallpaperService
  -> Tauri command adapter
  -> HTTP handler adapter
```

不要让 xrandr 或 feh 逻辑只存在于某一个 HTTP handler 中，否则桌面模式和浏览器模式会产生行为漂移。

## 17. 错误和安全要求

必须保持的安全约束：

- 外部命令使用程序名 + 参数数组，不使用 `sh -c`，不要拼接执行 shell。
- xrandr output、mode、rotation、refresh rate 必须经过后端重新读取的真实能力集合校验。
- 不允许把前端传入的任意 output 直接当成可信资源。
- 壁纸 mode 必须使用固定白名单映射到 feh 参数。
- 图片 path 需要 `is_file()` 检查。
- i3 配置写入时必须正确 quote 路径；至少处理单引号。
- Tauri CSP 当前允许 `img-src data:`，因为预览是 data URL；重写 Tauri 配置时不能删除这一点。
- 浏览器兼容模式只能监听回环地址，不能暴露到局域网。
- 显示器应用期间使用互斥锁，避免两个并发 apply 交叉覆盖 pending/回滚快照。
- apply 后必须重新查询并验证真实状态，不要把 xrandr 进程返回成功当作配置一定生效。
- 永远拒绝关闭所有显示器，且要求恰好一台已启用主显示器。

## 18. 已有测试和重写测试清单

当前单元测试覆盖：

- xrandr output、连接状态、primary、mode、刷新率、物理尺寸和坐标解析。
- 负坐标读取。
- left/right 旋转的 native mode 尺寸处理。
- 异常刷新率 token 不丢失整行 mode。
- xrandr 参数数组生成，验证没有 shell 插值。
- `/etc/os-release` 安全 fallback。
- 合法显示配置。
- 拒绝全部关闭。
- 拒绝未知 mode 和未知 output。

建议重写时至少补充：

1. `xrandr` 输出缺少 geometry、缺少 mode、空 stderr、非 UTF-8 stderr。
2. output 名称含特殊字符时仍只作为参数传递，不发生 shell 执行。
3. 多个相同分辨率刷新率的合并和容差。
4. 所有 rotation 值以及 rotation 下的当前 mode 匹配。
5. 重复 output、漏 output、两个 primary、零 primary、关闭 primary、负坐标、32768 坐标、无 mode。
6. apply 失败后的恢复成功和恢复失败。
7. apply 成功但实测不一致时的恢复。
8. pending token 错误、重复确认、确认/恢复与超时竞争。
9. Tauri 窗口关闭触发恢复。
10. wallpaper mode 白名单和所有 feh 参数映射。
11. 文件不存在、feh 缺失、feh 返回 stderr、yad 取消码 1/252。
12. 12 MiB 边界、未知扩展名 MIME、Base64 padding。
13. i3 配置中注释掉的 feh 行不替换；active 行替换；无 active 行追加；原有结尾换行保留；路径包含单引号。
14. `I3_CONFIG` 优先级和 HOME 缺失。
15. xdg-open 失败处理。

推荐用一个可执行 mock xrandr 脚本配合临时 PATH/环境变量测试命令交互，而不是依赖真实显示器。

## 19. 重写步骤建议

给其他 AI agent 的推荐实现顺序：

1. 先实现纯数据结构和 serde JSON 契约。
2. 实现 `xrandr --query` 的纯函数解析器，并用固定样本测试。
3. 实现纯函数 `build_xrandr_args` 和配置校验。
4. 实现 xrandr 进程适配层，统一命令错误映射。
5. 实现显示器 apply/verify/rollback/pending 状态机。
6. 接入 HTTP 和 Tauri 两种 adapter。
7. 再实现 wallpaper service：mode 映射、feh、预览、yad、i3 持久化、xdg-open。
8. 最后实现前端状态：查询、草稿、拖拽、吸附、确认倒计时和壁纸预览。
9. 运行格式化、单元测试、clippy，并用 mock xrandr 做端到端手工验证。

推荐保留的常量和默认值：

```text
CONFIRM_SECONDS = 15
RATE_TOLERANCE = 0.05
MAX_PREVIEW_BYTES = 12 * 1024 * 1024
DEFAULT_I3_CONFIG = ~/.config/i3/config
PEXELS_URL = https://www.pexels.com/zh-cn/
```

## 20. 重写时需要明确的当前实现取舍

以下不是遗漏，而是重写 agent 必须有意识处理的现状：

- 当前第一版只支持 X11，Wayland 直接提示不可修改；不要误宣称支持 Wayland。
- 当前写入校验拒绝负坐标，但读取解析支持负坐标。这是为了 UI 和安全约束的现有限制；如果新版本要支持常见的左侧屏幕布局，应同步修改 UI 坐标限制、校验、自动布局和吸附算法。
- 当前 snapshot 会保存真实负坐标，但 shutdown/timeout 的内部回滚绕过普通请求校验，直接调用 xrandr。这是因为回滚优先于用户输入校验；重写应保留这一原则。
- 当前 wallpaper preview 是完整图片 Base64，不是缩略图，12 MiB 限制主要是防止 IPC/HTTP 响应过大。生产重写可改成缩略图，但要明确改变协议或保持 `preview` data URL 兼容。
- 当前 `persist_i3_command` 以字符串规则识别 feh 行，不是完整 i3 parser。
- 当前自动排列只做水平排列，不处理碰撞、不计算物理尺寸、不支持镜像模式专用 UX。
- 当前前端选择“自动 / 推荐”时发送 `mode=null`，xrandr 使用 `--auto`；选择具体分辨率时会自动带 preferred 刷新率。
- 当前所有 connected output 都必须出现在 apply 请求中，不能只提交用户改动的一个输出。

## 21. 验收标准

重写版本至少应满足：

- 在 X11 + xrandr 环境能准确列出 connected/disconnected outputs、mode、刷新率、rotation、坐标和 primary。
- 后端拒绝未知输出、未知 mode、非法刷新率、全部关闭和非法 primary 组合。
- 应用失败、实测不一致、15 秒超时、用户点击恢复、Tauri 关闭窗口都能恢复旧布局。
- 用户确认后新布局保持，后续可以继续编辑。
- 壁纸可用五种显示方式调用 feh；yad 取消不会被当成系统故障。
- 可直接输入路径或通过 yad 选图，成功后前端显示预览。
- 持久化只修改 i3 中第一条 active `feh --bg-*` 行，或没有时追加启动项。
- 任意外部命令都不通过 shell 拼接执行。
- 浏览器模式仅绑定 loopback，Tauri 模式不监听 HTTP 端口。
- `cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings` 通过，并有 mock xrandr 测试。
