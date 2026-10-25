//! HTTP plumbing shared by every route: JSON bodies, security headers, the cross-origin check,
//! request logging, sign-in rate limiting and serving the built web app.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::{Body as AxumBody, Bytes};
use axum::extract::{ConnectInfo, FromRequest, Request, State};
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Map, Value};

use crate::auth::UserSlot;
use crate::error::AppError;
use crate::AppState;

/// A JSON request body. Like the original API, a missing or non-JSON body reads as `{}`.
pub struct Body(pub Value);

impl FromRequest<AppState> for Body {
    type Rejection = AppError;

    async fn from_request(req: Request, state: &AppState) -> Result<Body, AppError> {
        let is_json = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| {
                let ct = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
                ct == "application/json" || ct.ends_with("+json")
            })
            .unwrap_or(false);
        let bytes = Bytes::from_request(req, state).await.map_err(|e| {
            if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
                AppError::new(413, "Request is too large")
            } else {
                AppError::new(400, "Malformed JSON body")
            }
        })?;
        if !is_json || bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Body(Value::Object(Map::new())));
        }
        // Only objects and arrays are accepted at the top level.
        let first = bytes.iter().find(|b| !b.is_ascii_whitespace()).copied();
        if !matches!(first, Some(b'{') | Some(b'[')) {
            return Err(AppError::new(400, "Malformed JSON body"));
        }
        serde_json::from_slice(&bytes).map(Body).map_err(|_| AppError::new(400, "Malformed JSON body"))
    }
}

fn csp(production: bool) -> String {
    let mut directives = vec![
        "default-src 'self'",
        "script-src 'self'",
        "style-src 'self' 'unsafe-inline' https://fonts.googleapis.com",
        "img-src 'self' data: blob:",
        "media-src 'self' blob:",
        "connect-src 'self'",
        "font-src 'self' data: https://fonts.gstatic.com",
        "object-src 'none'",
        "frame-ancestors 'none'",
        "base-uri 'self'",
        "form-action 'self'",
        "script-src-attr 'none'",
    ];
    if production {
        directives.push("upgrade-insecure-requests");
    }
    directives.join(";")
}

/// The same security headers the API sent before (Helmet's defaults plus a strict CSP), and a
/// Permissions-Policy that lets only this site use the camera, for the scan pages.
pub async fn security_headers(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    let mut set = |name: &'static str, value: &str| {
        h.insert(HeaderName::from_static(name), HeaderValue::from_str(value).unwrap());
    };
    set("content-security-policy", &csp(state.config.is_production));
    set("cross-origin-opener-policy", "same-origin");
    set("cross-origin-resource-policy", "same-origin");
    set("origin-agent-cluster", "?1");
    set("referrer-policy", "no-referrer");
    set("strict-transport-security", "max-age=31536000; includeSubDomains");
    set("x-content-type-options", "nosniff");
    set("x-dns-prefetch-control", "off");
    set("x-download-options", "noopen");
    set("x-frame-options", "SAMEORIGIN");
    set("x-permitted-cross-domain-policies", "none");
    set("x-xss-protection", "0");
    set("permissions-policy", "camera=(self), microphone=(), geolocation=()");
    res
}

fn client_ip(req: &Request, trust_proxy: bool) -> String {
    if trust_proxy {
        let forwarded = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok());
        if let Some(ip) = forwarded.and_then(|f| f.rsplit(',').next()).map(str::trim).filter(|s| !s.is_empty()) {
            return ip.to_string();
        }
    }
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_string()).unwrap_or_default()
}

fn request_protocol(req: &Request, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(proto) = req.headers().get("x-forwarded-proto").and_then(|v| v.to_str().ok()) {
            return proto.split(',').next().unwrap_or("http").trim().to_string();
        }
    }
    "http".into()
}

/// Cookie sessions are CSRF-protected by SameSite=Lax, JSON-only bodies, and this origin check.
pub async fn check_origin(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if [Method::GET, Method::HEAD, Method::OPTIONS].contains(req.method()) {
        return next.run(req).await;
    }
    let Some(origin) = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()).map(str::to_string) else {
        return next.run(req).await;
    };
    let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let own = format!("{}://{}", request_protocol(&req, state.config.trust_proxy), host);
    if origin == own || state.config.app_origins.contains(&origin) {
        return next.run(req).await;
    }
    AppError::new(403, "Cross-origin request blocked").into_response()
}

/// One JSON line per request on stdout.
pub async fn request_log(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    if !state.config.log_requests {
        return next.run(req).await;
    }
    let start = Instant::now();
    let slot = UserSlot::default();
    req.extensions_mut().insert(slot.clone());
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let res = next.run(req).await;
    if path != "/api/health" {
        let user = *slot.0.lock().unwrap();
        println!(
            "{}",
            json!({
                "t": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "method": method, "path": path, "status": res.status().as_u16(),
                "ms": start.elapsed().as_millis() as u64, "user": user,
            })
        );
    }
    res
}

/// A fixed-window limit on sign-in, sign-up and password changes, per client address.
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    hits: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(limit: u32, window: Duration) -> Self {
        RateLimiter { limit, window, hits: Mutex::new(HashMap::new()) }
    }

    /// Counts a hit; returns the hits left and seconds until the window resets, or None when over the limit.
    fn hit(&self, key: &str) -> (bool, u32, u64) {
        let now = Instant::now();
        let mut hits = self.hits.lock().unwrap();
        if hits.len() > 10_000 {
            hits.retain(|_, (start, _)| now.duration_since(*start) < self.window);
        }
        let entry = hits.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        let reset = self.window.saturating_sub(now.duration_since(entry.0)).as_secs();
        (entry.1 <= self.limit, self.limit.saturating_sub(entry.1), reset)
    }
}

pub async fn rate_limit(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let limiter = &state.auth_limiter;
    let (allowed, remaining, reset) = limiter.hit(&client_ip(&req, state.config.trust_proxy));
    let mut res = if allowed {
        next.run(req).await
    } else {
        (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "Too many attempts. Try again in a few minutes." })))
            .into_response()
    };
    let h = res.headers_mut();
    let policy = format!("{};w={}", limiter.limit, limiter.window.as_secs());
    h.insert("ratelimit-policy", HeaderValue::from_str(&policy).unwrap());
    let current = format!("limit={}, remaining={remaining}, reset={reset}", limiter.limit);
    h.insert("ratelimit", HeaderValue::from_str(&current).unwrap());
    res
}

/// Unknown API paths: 401 for visitors who are not signed in, 404 for those who are.
pub async fn api_not_found(State(state): State<AppState>, req: Request) -> Response {
    let (parts, _) = req.into_parts();
    match crate::auth::authenticate(&parts, &state).await {
        Ok(_) => AppError::new(404, "Not found").into_response(),
        Err(e) => e.into_response(),
    }
}

/// Serves the built web app. Hashed files under /assets are cached for a year; every other path
/// falls back to index.html, which is never cached.
pub fn web_app(dir: &str) -> axum::Router<AppState> {
    use axum::routing::get;
    use tower::ServiceExt;
    use tower_http::services::ServeDir;

    let root = std::path::PathBuf::from(dir);
    let index = root.join("index.html");
    // Request paths keep their /assets prefix, so both serve from the app root.
    let assets = ServeDir::new(&root).append_index_html_on_directories(false);
    let files = ServeDir::new(&root).append_index_html_on_directories(false);

    let serve_assets = move |req: Request| {
        let assets = assets.clone();
        async move {
            let mut res = assets.oneshot(req).await.unwrap().map(AxumBody::new);
            if res.status().is_success() {
                res.headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
            } else if res.status() == StatusCode::NOT_FOUND {
                return AppError::new(404, "Not found").into_response();
            }
            res
        }
    };
    let serve_app = move |req: Request| {
        let files = files.clone();
        let index = index.clone();
        async move {
            let is_file = req.uri().path() != "/";
            if is_file {
                let res = files.oneshot(req).await.unwrap();
                if res.status().is_success() || res.status() == StatusCode::NOT_MODIFIED {
                    let mut res = res.map(AxumBody::new);
                    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));
                    return res;
                }
            }
            match tokio::fs::read(&index).await {
                Ok(html) => {
                    ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], html)
                        .into_response()
                }
                Err(_) => AppError::new(404, "Not found").into_response(),
            }
        }
    };
    axum::Router::new().route("/assets/{*path}", get(serve_assets)).fallback(get(serve_app))
}
