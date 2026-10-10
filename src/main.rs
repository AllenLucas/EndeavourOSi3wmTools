use std::{
    env, io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    process::{Command, Stdio},
    time::Duration,
};

use arch_tools::{
    api::{self, AppState},
    http_guard::{loopback_host_guard, origin_guard},
};
use axum::{
    Router,
    http::{HeaderMap, StatusCode, header},
    middleware,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("arch_tools=info,tower_http=info")),
        )
        .init();

    if env::args().any(|argument| argument == "--help" || argument == "-h") {
        print_help();
        return;
    }
    let open_browser = env::args().any(|argument| argument == "--open")
        || env::var("ARCH_TOOLS_OPEN_BROWSER")
            .or_else(|_| env::var("ADM_OPEN_BROWSER"))
            .is_ok_and(|value| value != "0");
    let port = env::var("ARCH_TOOLS_PORT")
        .or_else(|_| env::var("ADM_PORT"))
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8787);
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let url = format!("http://{address}");
    let state = AppState::new();
    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(javascript))
        .route("/style.css", get(stylesheet))
        .route("/api/system/status", get(api::system_status))
        .route("/api/wallpaper/set", post(api::wallpaper_set))
        .route("/api/wallpaper/select", post(api::wallpaper_select))
        .route("/api/wallpaper/search", post(api::wallpaper_search))
        .route("/api/displays", get(api::displays))
        .route("/api/displays/validate", post(api::validate))
        .route("/api/displays/apply", post(api::apply))
        .route("/api/displays/confirm", post(api::confirm))
        .route("/api/displays/revert", post(api::revert))
        .fallback(not_found)
        .layer(middleware::from_fn(origin_guard))
        .layer(middleware::from_fn(loopback_host_guard))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == io::ErrorKind::AddrInUse && open_browser => {
            println!("Arch Tools 已经运行，正在打开 {url}");
            if let Err(open_error) = open_url(&url) {
                eprintln!("无法自动打开浏览器：{open_error}\n请手动访问 {url}");
            }
            return;
        }
        Err(error) => {
            eprintln!("无法监听 {address}：{error}");
            eprintln!("如果应用已经运行，请访问 {url}");
            std::process::exit(1);
        }
    };
    info!(%address, "Arch Tools started");
    println!("Arch Tools: {url}");
    println!("保持此终端运行；按 Ctrl+C 可停止服务。\n");
    if open_browser {
        let browser_url = url.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if let Err(error) = open_url(&browser_url) {
                eprintln!("无法自动打开浏览器：{error}\n请手动访问 {browser_url}");
            }
        });
    }
    axum::serve(listener, app).await.expect("HTTP 服务意外退出");
}

fn open_url(url: &str) -> io::Result<()> {
    Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

fn print_help() {
    println!(
        "Arch Tools {}\n\n用法：\n  arch-tools [--open]\n\n选项：\n  --open       启动后自动打开默认浏览器\n  -h, --help   显示帮助\n\n环境变量：\n  ARCH_TOOLS_PORT          本地监听端口，默认 8787\n  ARCH_TOOLS_OPEN_BROWSER  非 0 时自动打开浏览器\n  ARCH_TOOLS_XRANDR        自定义 xrandr 程序路径",
        env!("CARGO_PKG_VERSION")
    );
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn javascript() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/javascript; charset=utf-8".parse().unwrap(),
    );
    headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    (headers, APP_JS)
}

async fn stylesheet() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/css; charset=utf-8".parse().unwrap(),
    );
    (headers, STYLE_CSS)
}

async fn not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "Not Found")
}
