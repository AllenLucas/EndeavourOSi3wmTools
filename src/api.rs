use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{Json, extract::State};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::{
    error::AppError,
    model::{
        ApplyResponse, Display, DisplayConfiguration, DisplayConfigurationRequest,
        DisplaysResponse, ModeSelection, OperationResponse, PendingInfo, SystemStatus,
        ValidationResponse,
    },
    xrandr::{XrandrError, XrandrManager},
};

pub const CONFIRM_SECONDS: u64 = 15;

pub async fn wallpaper_set(
    Json(request): Json<crate::wallpaper::WallpaperRequest>,
) -> Result<Json<crate::wallpaper::WallpaperResponse>, AppError> {
    Ok(Json(crate::wallpaper::set(request).await?))
}

pub async fn wallpaper_select() -> Result<Json<crate::wallpaper::WallpaperResponse>, AppError> {
    Ok(Json(crate::wallpaper::select().await?))
}

pub async fn wallpaper_search() -> Result<Json<crate::wallpaper::WallpaperResponse>, AppError> {
    crate::wallpaper::open_search().await?;
    Ok(Json(crate::wallpaper::WallpaperResponse {
        success: true,
        message: "已在默认浏览器中打开 Pexels 壁纸搜索".to_string(),
        path: None,
        preview: None,
    }))
}

const RATE_TOLERANCE: f64 = 0.05;

#[derive(Debug, Clone)]
struct PendingChange {
    token: String,
    expires_at: u64,
    previous: Vec<DisplayConfiguration>,
}

#[derive(Clone)]
pub struct AppState {
    xrandr: XrandrManager,
    pending: Arc<Mutex<Option<PendingChange>>>,
    operation: Arc<Mutex<()>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            xrandr: XrandrManager::new(),
            pending: Arc::new(Mutex::new(None)),
            operation: Arc::new(Mutex::new(())),
        }
    }

    /// Restore an unconfirmed layout before the Rust service exits.
    pub async fn revert_pending_on_shutdown(&self) -> Result<bool, AppError> {
        let _operation = self.operation.lock().await;
        let previous = self
            .pending
            .lock()
            .await
            .take()
            .map(|change| change.previous);
        let Some(previous) = previous else {
            return Ok(false);
        };
        self.xrandr.apply(&previous).await.map_err(map_xrandr)?;
        warn!("unconfirmed display configuration reverted during shutdown");
        Ok(true)
    }
}

pub async fn system_status(State(state): State<AppState>) -> Json<SystemStatus> {
    let os = XrandrManager::os_release();
    let session_type = XrandrManager::session_type();
    let xrandr_available = state.xrandr.is_available().await;
    let display = XrandrManager::display_environment();
    let pending_confirmation = state.pending.lock().await.is_some();
    let query = if xrandr_available && !session_type.eq_ignore_ascii_case("wayland") {
        state.xrandr.query().await.ok()
    } else {
        None
    };
    let connected_displays = query
        .as_ref()
        .map(|items| items.iter().filter(|display| display.connected).count());
    let can_modify = xrandr_available
        && !session_type.eq_ignore_ascii_case("wayland")
        && display.is_some()
        && query.is_some();
    let message = if !xrandr_available {
        "未找到 xrandr，请安装 xorg-xrandr".to_string()
    } else if session_type.eq_ignore_ascii_case("wayland") {
        "当前为 Wayland 会话，第一版只完整支持 X11".to_string()
    } else if display.is_none() {
        "缺少 DISPLAY 环境变量，无法访问 X11 会话".to_string()
    } else if !can_modify {
        "无法读取显示器，请检查 X11 权限与 DISPLAY 环境".to_string()
    } else {
        "显示器服务可用".to_string()
    };

    Json(SystemStatus {
        application: "Arch Tools".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        os_name: os.name,
        os_version: os.version,
        arch_linux: os.arch_linux,
        session_type,
        display,
        xrandr_available,
        can_modify,
        connected_displays,
        pending_confirmation,
        message,
    })
}

pub async fn displays(State(state): State<AppState>) -> Result<Json<DisplaysResponse>, AppError> {
    let displays = query_displays(&state).await?;
    let pending = state
        .pending
        .lock()
        .await
        .as_ref()
        .map(|change| PendingInfo {
            token: change.token.clone(),
            expires_at: change.expires_at,
        });
    Ok(Json(DisplaysResponse { displays, pending }))
}

pub async fn validate(
    State(state): State<AppState>,
    Json(request): Json<DisplayConfigurationRequest>,
) -> Result<Json<ValidationResponse>, AppError> {
    let current = query_displays(&state).await?;
    let (errors, warnings) = validate_configuration(&request.displays, &current);
    Ok(Json(ValidationResponse {
        valid: errors.is_empty(),
        errors,
        warnings,
        displays: current,
    }))
}

pub async fn apply(
    State(state): State<AppState>,
    Json(request): Json<DisplayConfigurationRequest>,
) -> Result<Json<ApplyResponse>, AppError> {
    let _operation = state.operation.lock().await;
    if state.pending.lock().await.is_some() {
        return Err(AppError::Conflict(
            "已有配置等待确认，请先保留或恢复该配置".to_string(),
        ));
    }
    state.xrandr.ensure_usable().await.map_err(map_xrandr)?;
    let before = query_displays(&state).await?;
    let (errors, _) = validate_configuration(&request.displays, &before);
    if !errors.is_empty() {
        return Err(AppError::BadRequest(errors.join("；")));
    }
    let previous = snapshot_configuration(&before);

    if let Err(apply_error) = state.xrandr.apply(&request.displays).await {
        return match state.xrandr.apply(&previous).await {
            Ok(()) => Err(AppError::Conflict(format!(
                "配置应用失败，已恢复原设置：{apply_error}"
            ))),
            Err(rollback_error) => {
                error!(%apply_error, %rollback_error, "configuration apply and rollback both failed");
                Err(AppError::Internal(format!(
                    "配置应用失败，且自动恢复也失败：{apply_error}；{rollback_error}"
                )))
            }
        };
    }
    let after = match state.xrandr.query().await {
        Ok(displays) => match verify_applied_configuration(&request.displays, &displays) {
            Ok(()) => displays,
            Err(reason) => {
                rollback_after_verification_failure(&state, &previous, &reason).await?;
                return Err(AppError::Conflict(format!(
                    "配置验证失败，已恢复原设置：{reason}"
                )));
            }
        },
        Err(error) => {
            let reason = error.to_string();
            rollback_after_verification_failure(&state, &previous, &reason).await?;
            return Err(AppError::Conflict(format!(
                "配置验证失败，已恢复原设置：{reason}"
            )));
        }
    };

    let token = generate_token();
    let expires_at = unix_seconds().saturating_add(CONFIRM_SECONDS);
    *state.pending.lock().await = Some(PendingChange {
        token: token.clone(),
        expires_at,
        previous,
    });
    info!(%token, expires_at, "display configuration applied, awaiting confirmation");
    spawn_timeout_revert(state.clone(), token.clone());

    Ok(Json(ApplyResponse {
        success: true,
        confirmation_required: true,
        token: Some(token),
        expires_at: Some(expires_at),
        message: format!("配置已应用，请在 {CONFIRM_SECONDS} 秒内确认"),
        displays: after,
    }))
}

pub async fn confirm(
    State(state): State<AppState>,
    Json(request): Json<PendingInfo>,
) -> Result<Json<OperationResponse>, AppError> {
    let _operation = state.operation.lock().await;
    let mut pending = state.pending.lock().await;
    let Some(change) = pending.as_ref() else {
        return Err(AppError::NotFound("没有等待确认的配置".to_string()));
    };
    if change.token != request.token {
        return Err(AppError::BadRequest("确认令牌无效".to_string()));
    }
    *pending = None;
    drop(pending);
    info!(token = %request.token, "display configuration confirmed");
    let displays = query_displays(&state).await?;
    Ok(Json(OperationResponse {
        success: true,
        message: "已保留当前显示设置".to_string(),
        displays,
    }))
}

pub async fn revert(
    State(state): State<AppState>,
    Json(request): Json<PendingInfo>,
) -> Result<Json<OperationResponse>, AppError> {
    let _operation = state.operation.lock().await;
    let change = take_pending(&state, &request.token).await?;
    state
        .xrandr
        .apply(&change.previous)
        .await
        .map_err(map_xrandr)?;
    let displays = query_displays(&state).await?;
    info!(token = %request.token, "display configuration reverted by user");
    Ok(Json(OperationResponse {
        success: true,
        message: "已恢复之前的显示设置".to_string(),
        displays,
    }))
}

async fn take_pending(state: &AppState, token: &str) -> Result<PendingChange, AppError> {
    let mut pending = state.pending.lock().await;
    if pending.as_ref().is_some_and(|change| change.token != token) {
        return Err(AppError::BadRequest("恢复令牌无效".to_string()));
    }
    pending
        .take()
        .ok_or_else(|| AppError::NotFound("没有可恢复的显示配置".to_string()))
}

fn spawn_timeout_revert(state: AppState, token: String) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(CONFIRM_SECONDS)).await;
        let _operation = state.operation.lock().await;
        let previous = {
            let mut pending = state.pending.lock().await;
            match pending.as_ref() {
                Some(change) if change.token == token => pending.take().map(|item| item.previous),
                _ => None,
            }
        };
        if let Some(previous) = previous {
            match state.xrandr.apply(&previous).await {
                Ok(()) => warn!(%token, "display configuration confirmation timed out; reverted"),
                Err(error) => error!(%token, %error, "automatic display rollback failed"),
            }
        }
    });
}

async fn query_displays(state: &AppState) -> Result<Vec<Display>, AppError> {
    state.xrandr.ensure_usable().await.map_err(map_xrandr)?;
    state.xrandr.query().await.map_err(map_xrandr)
}

async fn rollback_after_verification_failure(
    state: &AppState,
    previous: &[DisplayConfiguration],
    reason: &str,
) -> Result<(), AppError> {
    if let Err(rollback_error) = state.xrandr.apply(previous).await {
        error!(%rollback_error, %reason, "configuration verification and rollback both failed");
        return Err(AppError::Internal(format!(
            "配置验证失败，且自动恢复也失败：{reason}；{rollback_error}"
        )));
    }
    Ok(())
}

fn verify_applied_configuration(
    requested: &[DisplayConfiguration],
    actual: &[Display],
) -> Result<(), String> {
    for expected in requested {
        let Some(display) = actual
            .iter()
            .find(|display| display.name == expected.output)
        else {
            return Err(format!("应用后找不到输出 {}", expected.output));
        };
        if display.enabled != expected.enabled {
            return Err(format!("{} 的启用状态与请求不一致", expected.output));
        }
        if !expected.enabled {
            continue;
        }
        if display.primary != expected.primary {
            return Err(format!("{} 的主显示器状态与请求不一致", expected.output));
        }
        let Some(position) = &display.position else {
            return Err(format!("无法读取 {} 的坐标", expected.output));
        };
        if position.x != expected.x || position.y != expected.y {
            return Err(format!("{} 的实际坐标与请求不一致", expected.output));
        }
        if display.rotation != expected.rotation {
            return Err(format!("{} 的旋转方向与请求不一致", expected.output));
        }
        if let Some(expected_mode) = &expected.mode {
            let Some(actual_mode) = &display.current_mode else {
                return Err(format!("无法读取 {} 的当前模式", expected.output));
            };
            if actual_mode.width != expected_mode.width
                || actual_mode.height != expected_mode.height
            {
                return Err(format!("{} 的实际分辨率与请求不一致", expected.output));
            }
            if let (Some(expected_rate), Some(actual_rate)) =
                (expected_mode.refresh_rate, actual_mode.refresh_rate)
                && (expected_rate - actual_rate).abs() > RATE_TOLERANCE
            {
                return Err(format!("{} 的实际刷新率与请求不一致", expected.output));
            }
        }
    }
    if !actual.iter().any(|display| display.enabled) {
        return Err("应用后没有启用的显示器".to_string());
    }
    Ok(())
}

fn map_xrandr(error: XrandrError) -> AppError {
    match error {
        XrandrError::NotInstalled
        | XrandrError::UnsupportedSession
        | XrandrError::DisplayUnavailable(_) => AppError::Unavailable(error.to_string()),
        XrandrError::ApplyFailed(_) => AppError::Conflict(error.to_string()),
        XrandrError::QueryFailed(_) => AppError::Internal(error.to_string()),
    }
}

pub fn validate_configuration(
    configurations: &[DisplayConfiguration],
    current: &[Display],
) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let connected: HashMap<&str, &Display> = current
        .iter()
        .filter(|display| display.connected)
        .map(|display| (display.name.as_str(), display))
        .collect();
    let mut seen = HashSet::new();

    if configurations.is_empty() {
        errors.push("配置不能为空".to_string());
    }
    for configuration in configurations {
        if !seen.insert(configuration.output.as_str()) {
            errors.push(format!("输出 {} 重复出现", configuration.output));
            continue;
        }
        let Some(display) = connected.get(configuration.output.as_str()) else {
            errors.push(format!("显示器 {} 未连接或不存在", configuration.output));
            continue;
        };
        if configuration.x < 0 || configuration.y < 0 {
            errors.push(format!("{} 的坐标不能为负数", configuration.output));
        }
        if configuration.x > 32767 || configuration.y > 32767 {
            errors.push(format!("{} 的坐标超出允许范围", configuration.output));
        }
        if !configuration.enabled {
            if configuration.primary {
                errors.push(format!(
                    "已关闭的 {} 不能设为主显示器",
                    configuration.output
                ));
            }
            continue;
        }
        if let Some(selection) = &configuration.mode {
            let Some(mode) = display
                .modes
                .iter()
                .find(|mode| mode.width == selection.width && mode.height == selection.height)
            else {
                errors.push(format!(
                    "{} 不支持分辨率 {}",
                    configuration.output,
                    selection.resolution()
                ));
                continue;
            };
            if let Some(rate) = selection.refresh_rate
                && (!rate.is_finite()
                    || rate <= 0.0
                    || !mode
                        .refresh_rates
                        .iter()
                        .any(|item| (item.value - rate).abs() <= RATE_TOLERANCE))
            {
                errors.push(format!(
                    "{} 不支持 {} @ {} Hz",
                    configuration.output,
                    selection.resolution(),
                    rate
                ));
            }
        } else if display.modes.is_empty() {
            errors.push(format!("{} 没有可用显示模式", configuration.output));
        }
    }

    for name in connected.keys() {
        if !seen.contains(name) {
            errors.push(format!("请求缺少已连接输出 {name}"));
        }
    }
    let enabled = configurations.iter().filter(|item| item.enabled).count();
    let primary = configurations
        .iter()
        .filter(|item| item.enabled && item.primary)
        .count();
    if enabled == 0 {
        errors.push("至少必须保留一台启用的显示器".to_string());
    }
    if primary != 1 {
        errors.push("必须且只能指定一台已启用的主显示器".to_string());
    }

    let enabled_items: Vec<_> = configurations.iter().filter(|item| item.enabled).collect();
    for (index, left) in enabled_items.iter().enumerate() {
        for right in enabled_items.iter().skip(index + 1) {
            if left.x == right.x && left.y == right.y {
                warnings.push(format!(
                    "{} 与 {} 坐标相同，将形成镜像或重叠布局",
                    left.output, right.output
                ));
            }
        }
    }
    (errors, warnings)
}

pub fn snapshot_configuration(displays: &[Display]) -> Vec<DisplayConfiguration> {
    displays
        .iter()
        .filter(|display| display.connected)
        .map(|display| DisplayConfiguration {
            output: display.name.clone(),
            enabled: display.enabled,
            mode: display.current_mode.as_ref().map(|mode| ModeSelection {
                width: mode.width,
                height: mode.height,
                refresh_rate: mode.refresh_rate,
            }),
            x: display.position.as_ref().map_or(0, |position| position.x),
            y: display.position.as_ref().map_or(0, |position| position.y),
            primary: display.primary,
            rotation: display.rotation.clone(),
        })
        .collect()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn generate_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:032x}{count:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CurrentDisplayMode, DisplayMode, Position, RefreshRate, Rotation};

    fn display(name: &str, primary: bool) -> Display {
        Display {
            name: name.to_string(),
            connected: true,
            enabled: true,
            primary,
            current_mode: Some(CurrentDisplayMode {
                width: 1920,
                height: 1080,
                refresh_rate: Some(60.0),
            }),
            position: Some(Position { x: 0, y: 0 }),
            rotation: Rotation::Normal,
            physical_size: None,
            modes: vec![DisplayMode {
                width: 1920,
                height: 1080,
                refresh_rates: vec![RefreshRate {
                    value: 60.0,
                    current: true,
                    preferred: true,
                }],
                current: true,
                preferred: true,
            }],
        }
    }

    fn config(name: &str, primary: bool) -> DisplayConfiguration {
        DisplayConfiguration {
            output: name.to_string(),
            enabled: true,
            mode: Some(ModeSelection {
                width: 1920,
                height: 1080,
                refresh_rate: Some(60.0),
            }),
            x: 0,
            y: 0,
            primary,
            rotation: Rotation::Normal,
        }
    }

    #[test]
    fn accepts_valid_configuration() {
        let (errors, _) =
            validate_configuration(&[config("HDMI-1", true)], &[display("HDMI-1", true)]);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn rejects_turning_everything_off() {
        let mut item = config("HDMI-1", false);
        item.enabled = false;
        let (errors, _) = validate_configuration(&[item], &[display("HDMI-1", true)]);
        assert!(errors.iter().any(|error| error.contains("至少")));
    }

    #[test]
    fn rejects_unknown_mode_and_output() {
        let mut item = config("HDMI-1", true);
        item.mode.as_mut().unwrap().width = 9999;
        let (errors, _) = validate_configuration(
            &[item, config("EVIL;rm", false)],
            &[display("HDMI-1", true)],
        );
        assert!(errors.iter().any(|error| error.contains("不支持分辨率")));
        assert!(errors.iter().any(|error| error.contains("不存在")));
    }
}
