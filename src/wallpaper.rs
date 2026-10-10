use std::{
    env,
    path::{Path, PathBuf},
    process::Stdio,
};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::error::AppError;

const PEXELS_URL: &str = "https://www.pexels.com/zh-cn/";
const DEFAULT_I3_CONFIG: &str = ".config/i3/config";

#[derive(Debug, Deserialize)]
pub struct WallpaperRequest {
    pub path: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub persist: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperResponse {
    pub success: bool,
    pub message: String,
    pub path: Option<String>,
    pub preview: Option<String>,
}

fn default_mode() -> String {
    "fill".to_string()
}

fn home_path(relative: &str) -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(relative))
}

fn i3_config_path() -> Option<PathBuf> {
    env::var_os("I3_CONFIG")
        .map(PathBuf::from)
        .or_else(|| home_path(DEFAULT_I3_CONFIG))
}

fn mode_flag(mode: &str) -> Option<&'static str> {
    match mode {
        "fill" => Some("--bg-fill"),
        "scale" => Some("--bg-scale"),
        "max" => Some("--bg-max"),
        "center" => Some("--bg-center"),
        "tile" => Some("--bg-tile"),
        _ => None,
    }
}

const MAX_PREVIEW_BYTES: u64 = 12 * 1024 * 1024;

async fn preview_data_url(path: &Path) -> Result<String, AppError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| AppError::Unavailable(format!("无法读取图片预览：{error}")))?;
    if metadata.len() > MAX_PREVIEW_BYTES {
        return Err(AppError::BadRequest(
            "图片超过 12 MB，无法生成预览".to_string(),
        ));
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| AppError::Unavailable(format!("无法读取图片预览：{error}")))?;
    let mime = match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    };
    // Encoding several megabytes is CPU-bound; keep it off the async workers.
    let encoded = tokio::task::spawn_blocking(move || base64_encode(&bytes))
        .await
        .map_err(|error| AppError::Internal(format!("生成图片预览失败：{error}")))?;
    Ok(format!("data:{mime};base64,{encoded}"))
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as usize;
        let b = chunk.get(1).copied().unwrap_or(0) as usize;
        let c = chunk.get(2).copied().unwrap_or(0) as usize;
        result.push(TABLE[a >> 2] as char);
        result.push(TABLE[((a & 3) << 4) | (b >> 4)] as char);
        result.push(if chunk.len() > 1 {
            TABLE[((b & 15) << 2) | (c >> 6)] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            TABLE[c & 63] as char
        } else {
            '='
        });
    }
    result
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Locate `command` on `PATH` without going through a shell.
fn command_exists(command: &str) -> bool {
    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|directory| {
            let candidate = directory.join(command);
            candidate.is_file()
        })
    })
}

pub async fn set(request: WallpaperRequest) -> Result<WallpaperResponse, AppError> {
    let path = PathBuf::from(&request.path);
    if !path.is_file() {
        return Err(AppError::NotFound(format!(
            "找不到壁纸文件：{}",
            path.display()
        )));
    }
    let flag = mode_flag(&request.mode)
        .ok_or_else(|| AppError::BadRequest("不支持的壁纸显示模式".to_string()))?;
    if !command_exists("feh") {
        return Err(AppError::Unavailable(
            "系统中未找到 feh，无法设置壁纸".to_string(),
        ));
    }

    let output = Command::new("feh")
        .args([flag, &request.path])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| AppError::Unavailable(format!("无法执行 feh：{error}")))?;
    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::Conflict(if reason.is_empty() {
            "feh 设置壁纸失败".to_string()
        } else {
            reason
        }));
    }

    if request.persist {
        persist_i3_command(flag, &path).await?;
    }
    let preview = Some(preview_data_url(&path).await?);
    Ok(WallpaperResponse {
        success: true,
        message: if request.persist {
            "壁纸已设置并保存到 i3 配置"
        } else {
            "壁纸已设置"
        }
        .to_string(),
        path: Some(path.display().to_string()),
        preview,
    })
}

pub async fn select() -> Result<WallpaperResponse, AppError> {
    if !command_exists("yad") {
        return Err(AppError::Unavailable(
            "系统中未找到 yad，无法打开文件选择器".to_string(),
        ));
    }
    let output = Command::new("yad")
        .args([
            "--file",
            "--title=选择壁纸",
            "--image-filter=图片",
            "--width=900",
            "--height=620",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| AppError::Unavailable(format!("无法打开文件选择器：{error}")))?;
    if !output.status.success() {
        return match output.status.code() {
            Some(1 | 252) => Ok(WallpaperResponse {
                success: false,
                message: "已取消选择壁纸".to_string(),
                path: None,
                preview: None,
            }),
            _ => {
                let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
                Err(AppError::Unavailable(if reason.is_empty() {
                    format!("文件选择器异常退出：{}", output.status)
                } else {
                    format!("文件选择器启动失败：{reason}")
                }))
            }
        };
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err(AppError::NotFound("选择的文件不存在".to_string()));
    }
    Ok(WallpaperResponse {
        success: true,
        message: "已选择壁纸".to_string(),
        path: Some(path.display().to_string()),
        preview: Some(preview_data_url(&path).await?),
    })
}

async fn persist_i3_command(flag: &str, path: &Path) -> Result<(), AppError> {
    let config = i3_config_path()
        .ok_or_else(|| AppError::Unavailable("无法确定 i3 配置文件路径".to_string()))?;
    let content = tokio::fs::read_to_string(&config)
        .await
        .map_err(|error| AppError::Unavailable(format!("无法读取 i3 配置：{error}")))?;
    let command = format!(
        "exec --no-startup-id sleep 1 && feh {flag} {}",
        shell_quote(&path.display().to_string())
    );
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    // Skip commented-out lines so a stale `# exec ... feh --bg-fill` is not
    // mistaken for the active wallpaper entry.
    if let Some(line) = lines
        .iter_mut()
        .find(|line| !line.trim_start().starts_with('#') && line.contains("feh --bg-"))
    {
        let indentation = line
            .chars()
            .take_while(|character| character.is_whitespace())
            .collect::<String>();
        *line = format!("{indentation}{command}");
    } else {
        lines.push(String::new());
        lines.push("# set wallpaper".to_string());
        lines.push(command);
    }
    let mut updated = lines.join("\n");
    if content.ends_with('\n') {
        updated.push('\n');
    }
    write_atomically(&config, updated.as_bytes())
        .await
        .map_err(|error| AppError::Unavailable(format!("无法保存 i3 配置：{error}")))
}

async fn write_atomically(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let temporary = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".{file_name}.arch-tools-{}.tmp",
            std::process::id()
        ));

    let result = async {
        tokio::fs::write(&temporary, content).await?;
        if let Ok(metadata) = tokio::fs::metadata(path).await {
            tokio::fs::set_permissions(&temporary, metadata.permissions()).await?;
        }
        tokio::fs::rename(&temporary, path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

pub async fn open_search() -> Result<(), AppError> {
    Command::new("xdg-open")
        .arg(PEXELS_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| AppError::Unavailable(format!("无法打开默认浏览器：{error}")))
}
