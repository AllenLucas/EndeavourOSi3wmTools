use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use arch_tools::{
    api::{self, AppState},
    model::{
        ApplyResponse, DisplayConfigurationRequest, DisplaysResponse, OperationResponse,
        PendingInfo, SystemStatus, ValidationResponse,
    },
};
use axum::{Json, extract::State as AxumState};
use tauri::{Manager, RunEvent, WindowEvent};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tauri::command]
async fn wallpaper_set(
    request: arch_tools::wallpaper::WallpaperRequest,
) -> Result<arch_tools::wallpaper::WallpaperResponse, String> {
    arch_tools::wallpaper::set(request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn wallpaper_select() -> Result<arch_tools::wallpaper::WallpaperResponse, String> {
    arch_tools::wallpaper::select()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn wallpaper_search() -> Result<arch_tools::wallpaper::WallpaperResponse, String> {
    arch_tools::wallpaper::open_search()
        .await
        .map_err(|error| error.to_string())
        .map(|_| arch_tools::wallpaper::WallpaperResponse {
            success: true,
            message: "已在默认浏览器中打开 Pexels 壁纸搜索".to_string(),
            path: None,
            preview: None,
        })
}

#[tauri::command]
async fn system_status(state: tauri::State<'_, AppState>) -> Result<SystemStatus, String> {
    let Json(response) = api::system_status(AxumState(state.inner().clone())).await;
    Ok(response)
}

#[tauri::command]
async fn displays(state: tauri::State<'_, AppState>) -> Result<DisplaysResponse, String> {
    api::displays(AxumState(state.inner().clone()))
        .await
        .map(|Json(response)| response)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn validate_displays(
    state: tauri::State<'_, AppState>,
    request: DisplayConfigurationRequest,
) -> Result<ValidationResponse, String> {
    api::validate(AxumState(state.inner().clone()), Json(request))
        .await
        .map(|Json(response)| response)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn apply_displays(
    state: tauri::State<'_, AppState>,
    request: DisplayConfigurationRequest,
) -> Result<ApplyResponse, String> {
    api::apply(AxumState(state.inner().clone()), Json(request))
        .await
        .map(|Json(response)| response)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn confirm_displays(
    state: tauri::State<'_, AppState>,
    request: PendingInfo,
) -> Result<OperationResponse, String> {
    api::confirm(AxumState(state.inner().clone()), Json(request))
        .await
        .map(|Json(response)| response)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn revert_displays(
    state: tauri::State<'_, AppState>,
    request: PendingInfo,
) -> Result<OperationResponse, String> {
    api::revert(AxumState(state.inner().clone()), Json(request))
        .await
        .map(|Json(response)| response)
        .map_err(|error| error.to_string())
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("arch_tools=info")),
        )
        .init();

    let closing = Arc::new(AtomicBool::new(false));
    let application = tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            system_status,
            wallpaper_set,
            wallpaper_select,
            wallpaper_search,
            displays,
            validate_displays,
            apply_displays,
            confirm_displays,
            revert_displays
        ])
        .build(tauri::generate_context!())
        .expect("无法启动 Arch Tools 桌面应用");

    info!("Arch Tools desktop started");
    application.run(move |app, event| {
        if let RunEvent::WindowEvent {
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } = event
        {
            api.prevent_close();
            if closing.swap(true, Ordering::SeqCst) {
                return;
            }
            let app = app.clone();
            let state = app.state::<AppState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = state.revert_pending_on_shutdown().await {
                    error!(%error, "failed to revert pending display configuration while closing");
                }
                app.exit(0);
            });
        }
    });
}
