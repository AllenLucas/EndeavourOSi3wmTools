//! Request guards for the loopback-only HTTP servers.

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Mutating `/api/` requests must be JSON (forcing a CORS preflight) and, when the
/// browser sends an `Origin`, it must match the `Host` the request was sent to.
pub async fn origin_guard(request: Request<Body>, next: Next) -> Response {
    if request.uri().path().starts_with("/api/") && request.method() != axum::http::Method::GET {
        let valid_content_type = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"));
        if !valid_content_type {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "仅接受 application/json",
            )
                .into_response();
        }
        if let Some(origin) = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
        {
            let host = request
                .headers()
                .get(header::HOST)
                .and_then(|value| value.to_str().ok());
            let origin_authority = origin.strip_prefix("http://");
            if host.is_none() || origin_authority != host {
                return (StatusCode::FORBIDDEN, "拒绝来自外部网站的请求").into_response();
            }
        }
    }
    next.run(request).await
}

/// Reject requests whose `Host` is not a loopback name, defeating DNS rebinding.
pub async fn loopback_host_guard(request: Request<Body>, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !is_loopback_host(host) {
        return (StatusCode::FORBIDDEN, "仅允许通过本机回环地址访问").into_response();
    }
    next.run(request).await
}

pub fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        host.rsplit_once(':').map_or(host, |(name, _)| name)
    };
    matches!(name, "127.0.0.1" | "localhost" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_loopback_hosts() {
        assert!(is_loopback_host("127.0.0.1:18080"));
        assert!(is_loopback_host("localhost:18080"));
        assert!(is_loopback_host("[::1]:18080"));
        assert!(is_loopback_host("localhost"));
        assert!(!is_loopback_host("evil.example:18080"));
        assert!(!is_loopback_host("192.168.1.2:18080"));
        assert!(!is_loopback_host(""));
    }
}
