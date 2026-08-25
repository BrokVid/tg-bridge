//! Tg Bridge library: HMAC-authenticated relay to the Telegram Bot API.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

pub mod actions;
pub mod auth;
pub mod config;
pub mod metrics;
pub mod nonce;
pub mod proxy;
pub mod ratelimit;

use metrics::Metrics;
use nonce::NonceCache;
use ratelimit::RateLimiter;

pub struct AppState {
    pub cfg: config::Config,
    pub http: reqwest::Client,
    pub limiter: RateLimiter,
    pub metrics: Metrics,
    /// replay protection over (client, signature) pairs; see nonce.rs
    pub nonces: NonceCache,
}

pub type SharedState = Arc<AppState>;

pub fn tgb_error(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Json(json!({
            "ok": false,
            "error_code": status.as_u16(),
            "description": format!("tgb: {msg}"),
        })),
    )
        .into_response()
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({"ok": true, "version": env!("CARGO_PKG_VERSION")}))
}

pub fn build_router(state: SharedState) -> Router {
    let metrics_enabled = state.cfg.metrics.enabled;
    // uploads need more room than the default extractor limit; JSON routes
    // still enforce their own smaller max_body_bytes manually
    let upload_limit = state.cfg.server.max_upload_bytes;
    let base = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/t/{alias}/{method}", post(passthrough))
        .route("/v1/a/{name}", post(action))
        .layer(axum::extract::DefaultBodyLimit::max(upload_limit));
    if metrics_enabled {
        base.route("/metrics", get(metrics_endpoint)).with_state(state)
    } else {
        // /metrics disabled: not mounted at all
        base.with_state(state)
    }
}
fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Shared authentication part 1: client lookup, IP allowlist, header presence.
/// The timestamp window and HMAC check need the body, see [authenticate_body].
#[allow(clippy::result_large_err)]
fn authenticate_basic(
    st: &AppState,
    headers: &HeaderMap,
    ip: std::net::IpAddr,
) -> Result<(String, config::Client), Response> {
    let Some(client_name) = header_str(headers, "x-tgb-client") else {
        return Err(tgb_error(StatusCode::UNAUTHORIZED, "missing X-TgB-Client"));
    };
    let Some(_ts_raw) = header_str(headers, "x-tgb-timestamp") else {
        return Err(tgb_error(
            StatusCode::UNAUTHORIZED,
            "missing X-TgB-Timestamp",
        ));
    };
    let Some(_sig) = header_str(headers, "x-tgb-signature") else {
        return Err(tgb_error(
            StatusCode::UNAUTHORIZED,
            "missing X-TgB-Signature",
        ));
    };
    let Some(client) = st.cfg.clients.get(&client_name) else {
        tracing::warn!(client = %client_name, "unknown client");
        return Err(tgb_error(StatusCode::UNAUTHORIZED, "unknown client"));
    };
    if !client.allowed_ips.is_empty() && !client.allowed_ips.iter().any(|n| n.contains(&ip)) {
        tracing::warn!(client = %client_name, ip = %ip, "ip not allowed");
        return Err(tgb_error(StatusCode::UNAUTHORIZED, "ip not allowed"));
    }
    Ok((client_name, client.clone()))
}

/// Full auth: basic checks + timestamp window + constant-time HMAC over
/// `{timestamp}\n{body}`. Returns the client name, client config and the
/// signature (the caller needs it for the replay check, see [check_replay]).
#[allow(clippy::result_large_err)]
fn authenticate_body(
    st: &AppState,
    headers: &HeaderMap,
    ip: std::net::IpAddr,
    body: &[u8],
) -> Result<(String, config::Client, String), Response> {
    let (client_name, client) = authenticate_basic(st, headers, ip)?;
    let ts = header_str(headers, "x-tgb-timestamp")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    // abs_diff: no overflow on extreme timestamps
    if now_secs().abs_diff(ts) > st.cfg.server.timestamp_window_secs as u64 {
        return Err(tgb_error(StatusCode::UNAUTHORIZED, "timestamp out of window"));
    }
    let signature = header_str(headers, "x-tgb-signature").unwrap_or_default();
    // cap before hex::decode to avoid decoding attacker-controlled blobs
    if signature.is_empty() || signature.len() > 128 {
        return Err(tgb_error(StatusCode::UNAUTHORIZED, "bad signature"));
    }
    if !auth::verify(client.secret.as_bytes(), ts, body, &signature) {
        tracing::warn!(client = %client_name, "bad signature");
        return Err(tgb_error(StatusCode::UNAUTHORIZED, "bad signature"));
    }
    Ok((client_name, client, signature))
}

/// Rejects a (client, signature) pair already served within the TTL. Must run
/// after the rate limiter so a 429 never burns the request's slot in the
/// cache. `None` = allowed (or protection disabled).
fn check_replay(st: &AppState, client_name: &str, signature: &str) -> Option<Response> {
    if !st.cfg.server.replay_protection {
        return None;
    }
    let key = format!("{client_name}:{signature}");
    if st.nonces.insert_if_absent(&key, now_secs()) {
        None
    } else {
        tracing::warn!(client = %client_name, "replay detected");
        Some(tgb_error(StatusCode::UNAUTHORIZED, "replay detected"))
    }
}

/// Path segments must be conservative identifiers: they are interpolated into
/// the upstream URL. Bot API method names are case-insensitive alphanumerics
/// with underscores; bot aliases follow the same charset (config-side).
fn valid_method_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn valid_alias_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Telegram message length limits (Bot API 10.2): sendMessage/editMessageText
/// text is 1..=4096 chars; media captions are 0..=1024 chars.
fn validate_telegram_lengths(method: &str, params: &serde_json::Value) -> Result<(), &'static str> {
    let get_len = |field: &str| -> Option<usize> {
        params.get(field).and_then(|v| v.as_str()).map(|s| s.chars().count())
    };
    let m = method.to_ascii_lowercase();
    match m.as_str() {
        "sendmessage" | "editmessagetext" => {
            if get_len("text").is_some_and(|n| n > 4096) {
                return Err("text exceeds Telegram limit of 4096 characters");
            }
        }
        "sendphoto" | "sendvideo" | "senddocument" | "sendaudio" | "sendanimation"
        | "sendvoice" | "sendmedianote" | "sendlivephoto" | "editmessagecaption" => {
            if get_len("caption").is_some_and(|n| n > 1024) {
                return Err("caption exceeds Telegram limit of 1024 characters");
            }
        }
        "answercallbackquery" if get_len("text").is_some_and(|n| n > 200) => {
            return Err("text exceeds Telegram limit of 200 characters");
        }
        _ => {}
    }
    Ok(())
}

async fn passthrough(
    State(st): State<SharedState>,
    Path((alias, method)): Path<(String, String)>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: axum::body::Bytes,
) -> Response {
    let content_type = header_str(&headers, "content-type");
    passthrough_impl(&st, alias, method, &headers, addr.ip(), content_type, body).await
}

async fn passthrough_impl(
    st: &SharedState,
    alias: String,
    method: String,
    headers: &HeaderMap,
    addr: std::net::IpAddr,
    content_type: Option<String>,
    body: axum::body::Bytes,
) -> Response {
    let kind = "passthrough";
    // multipart/form-data is a file upload (Bot API ~50 MB); JSON is a normal call
    let is_upload = content_type
        .as_deref()
        .is_some_and(|c| c.to_ascii_lowercase().starts_with("multipart/form-data"));
    let limit = if is_upload {
        st.cfg.server.max_upload_bytes
    } else {
        st.cfg.server.max_body_bytes
    };
    if body.len() > limit {
        return tgb_error(StatusCode::PAYLOAD_TOO_LARGE, "body too large");
    }
    if !valid_alias_segment(&alias) || !valid_method_segment(&method) {
        return tgb_error(StatusCode::BAD_REQUEST, "invalid alias or method");
    }
    let (client_name, client, signature) =
        match authenticate_body(st, headers, addr, &body) {
            Ok(v) => v,
            Err(r) => return r,
        };
    if !client.allow_passthrough {
        tracing::warn!(client = %client_name, "passthrough disabled for client");
        return tgb_error(StatusCode::FORBIDDEN, "passthrough not allowed for client");
    }
    if !st.limiter.allow(&client_name) {
        st.metrics.record_request(&client_name, kind, 429, 0);
        return tgb_error(StatusCode::TOO_MANY_REQUESTS, "rate limited");
    }
    if let Some(r) = check_replay(st, &client_name, &signature) {
        return r;
    }

    let result = resolve_bot_and_check_method(st, &client, &alias, &method);
    let token = match result {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    let started = std::time::Instant::now();
    let resp = proxy::passthrough_response(
        &st.http,
        &st.cfg.telegram.api_base,
        &token,
        &alias,
        &method,
        content_type,
        body,
    )
    .await;
    let ms = started.elapsed().as_millis() as u64;
    match resp {
        Ok(r) => {
            st.metrics
                .record_request(&client_name, kind, r.status().as_u16(), ms);
            tracing::info!(
                client = %client_name,
                bot = %alias,
                method = %method,
                status = r.status().as_u16(),
                ms,
                "proxied"
            );
            r
        }
        Err(e) => {
            st.metrics.record_upstream_error(kind);
            tracing::error!(client = %client_name, method = %method, error = %e, "upstream error");
            tgb_error(StatusCode::BAD_GATEWAY, "telegram unreachable")
        }
    }
}

#[allow(clippy::result_large_err)]
fn resolve_bot_and_check_method(
    st: &AppState,
    client: &config::Client,
    alias: &str,
    method: &str,
) -> Result<String, Response> {
    let Some(token) = st.cfg.bots.get(alias) else {
        return Err(tgb_error(StatusCode::NOT_FOUND, "unknown bot alias"));
    };
    if !client.bots.is_empty() && !client.bots.iter().any(|b| b == alias) {
        return Err(tgb_error(StatusCode::FORBIDDEN, "bot not allowed for client"));
    }
    if let Some(list) = &client.methods_allowlist {
        if !list.iter().any(|m| m == method) {
            return Err(tgb_error(StatusCode::FORBIDDEN, "method not allowed for client"));
        }
    }
    Ok(token.clone())
}

async fn action(
    State(st): State<SharedState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: axum::body::Bytes,
) -> Response {
    action_impl(&st, name, &headers, addr, body).await
}

async fn action_impl(
    st: &SharedState,
    name: String,
    headers: &HeaderMap,
    addr: SocketAddr,
    body: axum::body::Bytes,
) -> Response {
    let kind = "action";
    if body.len() > st.cfg.server.max_body_bytes {
        return tgb_error(StatusCode::PAYLOAD_TOO_LARGE, "body too large");
    }
    if !valid_method_segment(&name) {
        return tgb_error(StatusCode::BAD_REQUEST, "invalid action name");
    }
    let (client_name, _client, signature) =
        match authenticate_body(st, headers, addr.ip(), &body) {
            Ok(v) => v,
            Err(r) => return r,
        };
    if !st.limiter.allow(&client_name) {
        st.metrics.record_request(&client_name, kind, 429, 0);
        return tgb_error(StatusCode::TOO_MANY_REQUESTS, "rate limited");
    }
    if let Some(r) = check_replay(st, &client_name, &signature) {
        return r;
    }

    let Some(spec) = st.cfg.actions.get(&name).cloned() else {
        return tgb_error(StatusCode::NOT_FOUND, "unknown action");
    };
    if spec.client != client_name {
        return tgb_error(StatusCode::FORBIDDEN, "action not allowed for client");
    }
    if !st.cfg.bots.contains_key(&spec.bot) {
        return tgb_error(StatusCode::NOT_FOUND, "unknown bot alias");
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return tgb_error(StatusCode::BAD_REQUEST, &format!("bad json: {e}")),
    };
    let payload_map = match payload.as_object() {
        Some(m) => m.clone(),
        None => return tgb_error(StatusCode::BAD_REQUEST, "json object expected"),
    };
    let params = match actions::render_params(&spec.params, &payload_map) {
        Ok(p) => p,
        Err(missing) => {
            return tgb_error(
                StatusCode::BAD_REQUEST,
                &format!("missing fields: {}", missing.join(", ")),
            )
        }
    };
    if let Err(msg) = validate_telegram_lengths(&spec.method, &params) {
        return tgb_error(StatusCode::BAD_REQUEST, msg);
    }
    // security-relevant params (chat_id, etc.) must not be overridable via
    // template placeholders pointing at client-controlled fields; warn loudly
    if spec
        .params
        .get("chat_id")
        .map(|v| v.as_str().is_some_and(|s| s.contains("{{")))
        .unwrap_or(false)
    {
        tracing::warn!(action = %name, "action chat_id is templated from client input");
    }
    let token = &st.cfg.bots[&spec.bot];
    let params_bytes = serde_json::to_vec(&params).expect("serializable");

    let started = std::time::Instant::now();
    match proxy::send_json(
        &st.http,
        &st.cfg.telegram.api_base,
        token,
        &spec.bot,
        &spec.method,
        params_bytes,
    )
    .await
    {
        Ok((status, tg_json)) => {
            let ms = started.elapsed().as_millis() as u64;
            st.metrics.record_request(&client_name, kind, status, ms);
            tracing::info!(
                client = %client_name,
                action = %name,
                method = %spec.method,
                status,
                ms,
                "action executed"
            );
            let telegram_ok = tg_json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            (
                StatusCode::OK,
                Json(json!({"ok": true, "telegram_ok": telegram_ok, "result": tg_json})),
            )
                .into_response()
        }
        Err(e) => {
            st.metrics.record_upstream_error(kind);
            tracing::error!(client = %client_name, action = %name, error = %e, "upstream error");
            tgb_error(StatusCode::BAD_GATEWAY, "telegram unreachable")
        }
    }
}

async fn metrics_endpoint(
    State(st): State<SharedState>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    // protected like every other endpoint; returns global aggregates only
    if let Err(r) = authenticate_body(&st, &headers, addr.ip(), b"") {
        return r;
    }
    if !st.limiter.allow("metrics") {
        return tgb_error(StatusCode::TOO_MANY_REQUESTS, "rate limited");
    }
    let signature = header_str(&headers, "x-tgb-signature").unwrap_or_default();
    if let Some(r) = check_replay(&st, "metrics", &signature) {
        return r;
    }
    let text = st.metrics.render();
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        text,
    )
        .into_response()
}
