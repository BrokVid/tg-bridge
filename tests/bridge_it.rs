//! Handler-level integration tests: full request pipeline (auth -> rate
//! limit -> forwarding) against a fake Telegram server, no external network.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use tg_bridge::auth::sign_hex;
use tg_bridge::config::{
    Bot, Client, Config, Metrics as MetricsCfg, RateLimit, Server, Telegram, WebhookTarget,
};
use tg_bridge::{AppState, SharedState};

const SECRET: &str = "test-secret-0123456789abcdef";
const CLIENT: &str = "testclient";
const WEBHOOK_SECRET: &str = "telegram-webhook-secret";

fn test_config(api_base: String) -> Config {
    Config {
        server: Server {
            listen: "127.0.0.1:0".into(),
            max_body_bytes: 65536,
            max_upload_bytes: 1024 * 1024,
            request_timeout: Duration::from_secs(5),
            timestamp_window_secs: 60,
            replay_protection: true,
        },
        telegram: Telegram { api_base },
        rate_limit: RateLimit {
            requests_per_minute: 1000,
        },
        metrics: MetricsCfg { enabled: true },
        bots: HashMap::from([
            (
                "mybot".to_string(),
                Bot {
                    token: "123456:FAKE-TOKEN".to_string(),
                    webhook: None,
                },
            ),
            (
                "noweb".to_string(),
                Bot {
                    token: "789:OTHER".to_string(),
                    webhook: None,
                },
            ),
        ]),
        clients: HashMap::from([(
            CLIENT.to_string(),
            Client {
                secret: SECRET.into(),
                allowed_ips: vec![],
                bots: vec!["mybot".into()],
                allow_passthrough: true,
                methods_allowlist: Some(vec![
                    "getMe".into(),
                    "sendMessage".into(),
                    "sendDocument".into(),
                ]),
            },
        )]),
        actions: HashMap::from([(
            "notify".to_string(),
            serde_json::from_value(json!({
                "client": CLIENT,
                "bot": "mybot",
                "method": "sendMessage",
                "params": {
                    "chat_id": -100123,
                    "text": "[{{level|info}}] {{text}}"
                }
            }))
            .unwrap(),
        )]),
    }
}

/// Fake api.telegram.org handle: parsed JSON of the last request plus the
/// raw bytes/content-type (for multipart passthrough checks).
type RawCapture = Arc<std::sync::Mutex<Option<(String, Vec<u8>)>>>;

struct FakeTg {
    api_base: String,
    last: Arc<std::sync::Mutex<Option<serde_json::Value>>>,
    raw: RawCapture,
}

async fn fake_telegram() -> FakeTg {
    let last = Arc::new(std::sync::Mutex::new(None));
    let raw = Arc::new(std::sync::Mutex::new(None));
    let last_clone = last.clone();
    let raw_clone = raw.clone();
    let app = axum::Router::new().route(
        "/bot123456:FAKE-TOKEN/{method}",
        axum::routing::post(
            |axum::extract::Path(method): axum::extract::Path<String>,
             headers: axum::http::HeaderMap,
             body: axum::body::Bytes| async move {
                let ct = headers
                    .get(axum::http::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                *raw_clone.lock().unwrap() = Some((ct, body.to_vec()));
                let value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
                *last_clone.lock().unwrap() = Some(value);
                axum::Json(json!({"ok": true, "result": {"method": method}}))
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    FakeTg {
        api_base: format!("http://{addr}"),
        last,
        raw,
    }
}

fn test_state(api_base: String) -> SharedState {
    Arc::new(AppState {
        cfg: test_config(api_base),
        http: reqwest::Client::new(),
        limiter: tg_bridge::ratelimit::RateLimiter::new(3),
        metrics: tg_bridge::metrics::Metrics::default(),
        nonces: tg_bridge::nonce::NonceCache::new(125),
    })
}

async fn send(
    router: axum::Router,
    method_path: &str,
    body: &str,
    ts_offset: i64,
    secret: &[u8],
    sig_override: Option<String>,
) -> (StatusCode, serde_json::Value) {
    send_bytes(
        router,
        method_path,
        body.as_bytes().to_vec(),
        ts_offset,
        secret,
        sig_override,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn send_bytes(
    router: axum::Router,
    method_path: &str,
    body: Vec<u8>,
    ts_offset: i64,
    secret: &[u8],
    sig_override: Option<String>,
    content_type: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let ts_val = tg_bridge::now_secs() + ts_offset;
    let req = Request::builder()
        .method("POST")
        .uri(format!("http://bridge.test{method_path}"))
        .header("content-type", content_type.unwrap_or("application/json"))
        .header("x-tgb-client", CLIENT)
        .header("x-tgb-timestamp", ts_val.to_string())
        .header(
            "x-tgb-signature",
            sig_override.unwrap_or_else(|| sign_hex(secret, ts_val, &body)),
        )
        .extension(ConnectInfo("127.0.0.1:55555".parse::<SocketAddr>().unwrap()))
        .body(Body::from(body))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    // axum's own body-limit rejection returns an empty/non-JSON body
    let value = serde_json::from_slice(&bytes).unwrap_or(json!(null));
    (status, value)
}

#[tokio::test]
async fn healthz_ok() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let req = Request::builder()
        .uri("http://bridge.test/healthz")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn unsigned_request_rejected() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let req = Request::builder()
        .method("POST")
        .uri("http://bridge.test/v1/t/mybot/getMe")
        .extension(ConnectInfo("127.0.0.1:55555".parse::<SocketAddr>().unwrap()))
        .body(Body::from("{}"))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bad_signature_rejected() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let (status, body) = send(router, "/v1/t/mybot/getMe", "{}", 0, b"wrong", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["description"], json!("tgb: bad signature"));
}

#[tokio::test]
async fn stale_timestamp_rejected() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let (status, _) = send(router, "/v1/t/mybot/getMe", "{}", -120, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn passthrough_forwards_to_telegram() {
    let tg = fake_telegram().await;
    let router = tg_bridge::build_router(test_state(tg.api_base.clone()));
    let (status, body) =
        send(router, "/v1/t/mybot/getMe", "{\"a\":1}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["result"]["method"], json!("getMe"));
}

#[tokio::test]
async fn disallowed_method_rejected() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    // deleteWebhook is not in methods_allowlist
    let (status, body) =
        send(router, "/v1/t/mybot/deleteWebhook", "{}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["description"], json!("tgb: method not allowed for client"));
}

#[tokio::test]
async fn action_renders_template_and_calls_telegram() {
    let tg = fake_telegram().await;
    let router = tg_bridge::build_router(test_state(tg.api_base.clone()));
    let payload = json!({"level": "warn", "text": "disk 91%"}).to_string();
    let (status, body) =
        send(router, "/v1/a/notify", &payload, 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["telegram_ok"], json!(true));
    let sent = tg.last.lock().unwrap().clone().unwrap();
    assert_eq!(sent["chat_id"], json!(-100123));
    assert_eq!(sent["text"], json!("[warn] disk 91%"));
}

#[tokio::test]
async fn action_missing_field_is_400() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let payload = json!({"title": "no text field"}).to_string();
    let (status, body) = send(router, "/v1/a/notify", &payload, 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["description"], json!("tgb: missing fields: text"));
}

#[tokio::test]
async fn unknown_action_is_404() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let (status, _) = send(router, "/v1/a/nope", "{}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rate_limit_blocks_burst() {
    let tg = fake_telegram().await;
    let state = test_state(tg.api_base);
    let router = tg_bridge::build_router(state.clone());
    for i in 0..3 {
        let r = send(
            router.clone(),
            "/v1/t/mybot/getMe",
            &format!("{{\"a\":{i}}}"),
            0,
            SECRET.as_bytes(),
            None,
        )
        .await;
        assert_eq!(r.0, StatusCode::OK);
    }
    let (status, body) =
        send(router, "/v1/t/mybot/getMe", "{\"a\":3}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["description"], json!("tgb: rate limited"));
}

#[tokio::test]
async fn metrics_exposes_counters() {
    let tg = fake_telegram().await;
    let state = test_state(tg.api_base);
    let router = tg_bridge::build_router(state.clone());
    let _ = send(router.clone(), "/v1/t/mybot/getMe", "{\"a\":1}", 0, SECRET.as_bytes(), None).await;

    let req = Request::builder()
        .method("GET")
        .uri("http://bridge.test/metrics")
        .header("x-tgb-client", CLIENT)
        .header("x-tgb-timestamp", tg_bridge::now_secs().to_string())
        .header(
            "x-tgb-signature",
            sign_hex(SECRET.as_bytes(), tg_bridge::now_secs(), b""),
        )
        .extension(ConnectInfo("127.0.0.1:55555".parse::<SocketAddr>().unwrap()))
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("tgb_requests_total{client=\"testclient\",kind=\"passthrough\",status=\"200\"} 1"));
}

// ---------- edge cases / input hardening ----------

#[tokio::test]
async fn extreme_timestamp_rejected_without_overflow() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    // i64::MIN would overflow naive (now - ts).abs(); must be a clean 401
    let req = Request::builder()
        .method("POST")
        .uri("http://bridge.test/v1/t/mybot/getMe")
        .header("content-type", "application/json")
        .header("x-tgb-client", CLIENT)
        .header("x-tgb-timestamp", i64::MIN.to_string())
        .header(
            "x-tgb-signature",
            sign_hex(SECRET.as_bytes(), i64::MIN, b"{}"),
        )
        .extension(ConnectInfo("127.0.0.1:55555".parse::<SocketAddr>().unwrap()))
        .body(Body::from("{}"))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn oversized_signature_header_rejected() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let ts = tg_bridge::now_secs().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("http://bridge.test/v1/t/mybot/getMe")
        .header("content-type", "application/json")
        .header("x-tgb-client", CLIENT)
        .header("x-tgb-timestamp", ts)
        .header("x-tgb-signature", "a".repeat(10_000))
        .extension(ConnectInfo("127.0.0.1:55555".parse::<SocketAddr>().unwrap()))
        .body(Body::from("{}"))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn path_traversal_in_method_rejected() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    for evil in ["..", "%2e%2e", "getMe%2Fextra", "get%20me", "%D0%BC"] {
        let (status, _) = send(
            router.clone(),
            &format!("/v1/t/mybot/{evil}"),
            "{}",
            0,
            SECRET.as_bytes(),
            None,
        )
        .await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
            "{evil}: unexpected {status}"
        );
    }
}

#[tokio::test]
async fn oversized_body_rejected() {
    let mut cfg_over = test_config("http://127.0.0.1:1".into());
    cfg_over.server.max_body_bytes = 8;
    let router = tg_bridge::build_router(Arc::new(AppState {
        limiter: tg_bridge::ratelimit::RateLimiter::new(100),
        metrics: tg_bridge::metrics::Metrics::default(),
        nonces: tg_bridge::nonce::NonceCache::new(125),
        http: reqwest::Client::new(),
        cfg: cfg_over,
    }));
    let (status, body) =
        send(router, "/v1/t/mybot/getMe", "{\"a\":\"123456789\"}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["description"], json!("tgb: body too large"));
}

#[tokio::test]
async fn action_oversized_text_is_400_not_telegram_error() {
    // action template renders client text; >4096 chars must be rejected by the bridge
    let tg = fake_telegram().await;
    let mut cfg = test_config(tg.api_base);
    if let Some(a) = cfg.actions.get_mut("notify") {
        a.params["text"] = json!("{{text}}");
    }
    let state: SharedState = Arc::new(AppState {
        limiter: tg_bridge::ratelimit::RateLimiter::new(100),
        metrics: tg_bridge::metrics::Metrics::default(),
        nonces: tg_bridge::nonce::NonceCache::new(125),
        http: reqwest::Client::new(),
        cfg,
    });
    let router = tg_bridge::build_router(state);
    let big = "x".repeat(5000);
    let payload = json!({"text": big}).to_string();
    let (status, body) = send(router, "/v1/a/notify", &payload, 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["description"],
        json!("tgb: text exceeds Telegram limit of 4096 characters")
    );
    assert!(tg.last.lock().unwrap().is_none(), "nothing must reach upstream");
}

// ---------- multipart uploads ----------

#[tokio::test]
async fn multipart_upload_forwarded_verbatim() {
    let tg = fake_telegram().await;
    let router = tg_bridge::build_router(test_state(tg.api_base));

    let boundary = "tgbtestboundary123";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"caption\"\r\n\r\nsmoke file\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"document\"; filename=\"a.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]); // binary payload
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let ct = format!("multipart/form-data; boundary={boundary}");

    let (status, resp) = send_bytes(
        router,
        "/v1/t/mybot/sendDocument",
        body.clone(),
        0,
        SECRET.as_bytes(),
        None,
        Some(&ct),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "resp={resp}");

    let raw = tg.raw.lock().unwrap().clone().expect("upstream saw request");
    assert_eq!(raw.0, ct, "content-type with boundary forwarded verbatim");
    assert_eq!(raw.1, body, "multipart bytes forwarded verbatim");
}

#[tokio::test]
async fn oversized_upload_rejected_before_upstream() {
    let tg = fake_telegram().await;
    let mut cfg = test_config(tg.api_base.clone());
    cfg.server.max_upload_bytes = 16;
    let state: SharedState = Arc::new(AppState {
        limiter: tg_bridge::ratelimit::RateLimiter::new(100),
        metrics: tg_bridge::metrics::Metrics::default(),
        nonces: tg_bridge::nonce::NonceCache::new(125),
        http: reqwest::Client::new(),
        cfg,
    });
    let router = tg_bridge::build_router(state);
    let big_body = vec![b'x'; 64];
    let (status, _resp) = send_bytes(
        router,
        "/v1/t/mybot/sendDocument",
        big_body,
        0,
        SECRET.as_bytes(),
        None,
        Some("multipart/form-data; boundary=b"),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    // rejected before the handler (axum body limit) or by the bridge itself;
    // either way nothing reaches upstream
    assert!(tg.raw.lock().unwrap().is_none(), "nothing must reach upstream");
}

#[tokio::test]
async fn json_over_upload_limit_but_under_json_limit_is_ok() {
    // JSON limit is separate from upload limit: a JSON body larger than
    // max_body_bytes stays limited by max_body_bytes even though the route
    // allows bigger multipart bodies.
    let tg = fake_telegram().await;
    let mut cfg = test_config(tg.api_base);
    cfg.server.max_body_bytes = 8;
    let state: SharedState = Arc::new(AppState {
        limiter: tg_bridge::ratelimit::RateLimiter::new(100),
        metrics: tg_bridge::metrics::Metrics::default(),
        nonces: tg_bridge::nonce::NonceCache::new(125),
        http: reqwest::Client::new(),
        cfg,
    });
    let router = tg_bridge::build_router(state);
    let (status, _) = send(router, "/v1/t/mybot/getMe", "{\"a\":\"123456789\"}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn action_with_non_object_payload_is_400() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let (status, _) = send(router, "/v1/a/notify", "[1,2]", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn deep_nested_action_payload_handled_gracefully() {
    // serde_json recursion limit turns pathological nesting into a clean 400
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let mut deep = String::from("{\"a\":");
    for _ in 0..200 {
        deep.push_str("[{\"a\":");
    }
    deep.push('1');
    for _ in 0..200 {
        deep.push_str("}]");
    }
    deep.push('}');
    let (status, _) = send(router, "/v1/a/notify", &deep, 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unknown_bot_alias_is_404() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let (status, _) = send(router, "/v1/t/other/getMe", "{}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bot_not_allowed_for_client_is_403() {
    let mut cfg = test_config("http://127.0.0.1:1".into());
    cfg.bots.insert(
        "other".to_string(),
        Bot {
            token: "token".to_string(),
            webhook: None,
        },
    );
    let state: SharedState = Arc::new(AppState {
        limiter: tg_bridge::ratelimit::RateLimiter::new(100),
        metrics: tg_bridge::metrics::Metrics::default(),
        nonces: tg_bridge::nonce::NonceCache::new(125),
        http: reqwest::Client::new(),
        cfg,
    });
    let router = tg_bridge::build_router(state);
    let (status, _) = send(router, "/v1/t/other/getMe", "{}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ---------- webhook relay (ADR-6) ----------

type WebhookCapture = Arc<std::sync::Mutex<Option<(String, String, Vec<u8>)>>>; // ts, sig, body

struct FakeClient {
    url: String,
    captured: WebhookCapture,
}

/// Client-side endpoint that the bridge delivers webhook updates to.
async fn fake_client_endpoint() -> FakeClient {
    let captured = Arc::new(std::sync::Mutex::new(None));
    let cap = captured.clone();
    let app = axum::Router::new().route(
        "/tg/callback",
        axum::routing::post(
            |headers: axum::http::HeaderMap, body: axum::body::Bytes| async move {
                let get = |k: &str| {
                    headers
                        .get(k)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_owned()
                };
                *cap.lock().unwrap() = Some((
                    get("x-tgb-timestamp"),
                    get("x-tgb-signature"),
                    body.to_vec(),
                ));
                axum::Json(json!({"received": true}))
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    FakeClient {
        url: format!("http://{addr}/tg/callback"),
        captured,
    }
}

async fn send_webhook(
    router: axum::Router,
    alias: &str,
    secret: Option<&str>,
    body: &str,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("http://bridge.test/webhook/{alias}"))
        .header("content-type", "application/json")
        .header("x-telegram-bot-api-secret-token", secret.unwrap_or("wrong"))
        .extension(ConnectInfo("127.0.0.1:55555".parse::<SocketAddr>().unwrap()))
        .body(Body::from(body.to_owned()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(json!(null));
    (status, value)
}

fn state_with_webhook(api_base: String, client_url: String) -> SharedState {
    let mut cfg = test_config(api_base);
    cfg.bots.get_mut("mybot").unwrap().webhook = Some(WebhookTarget {
        secret: WEBHOOK_SECRET.into(),
        url: client_url,
        client: CLIENT.into(),
    });
    Arc::new(AppState {
        limiter: tg_bridge::ratelimit::RateLimiter::new(100),
        metrics: tg_bridge::metrics::Metrics::default(),
        nonces: tg_bridge::nonce::NonceCache::new(125),
        http: reqwest::Client::new(),
        cfg,
    })
}

#[tokio::test]
async fn webhook_relays_signed_update_to_client() {
    let fc = fake_client_endpoint().await;
    let router = tg_bridge::build_router(state_with_webhook(
        "http://127.0.0.1:1".into(),
        fc.url.clone(),
    ));

    let update = json!({"update_id": 42, "message": {"text": "hi"}}).to_string();
    let (status, body) =
        send_webhook(router, "mybot", Some(WEBHOOK_SECRET), &update).await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let cap = fc.captured.lock().unwrap().clone().expect("delivered");
    assert_eq!(cap.2, update.as_bytes());
    // delivery must be signed with the client's secret over {ts}\n{body}
    let ts: i64 = cap.0.parse().expect("numeric ts");
    assert_eq!(cap.1, sign_hex(SECRET.as_bytes(), ts, update.as_bytes()));
}

#[tokio::test]
async fn webhook_bad_secret_rejected_without_delivery() {
    let fc = fake_client_endpoint().await;
    let router = tg_bridge::build_router(state_with_webhook(
        "http://127.0.0.1:1".into(),
        fc.url.clone(),
    ));
    let (status, _) =
        send_webhook(router, "mybot", Some("totally-wrong"), "{}").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(fc.captured.lock().unwrap().is_none());
}

#[tokio::test]
async fn webhook_not_configured_is_404() {
    let fc = fake_client_endpoint().await;
    let router = tg_bridge::build_router(state_with_webhook(
        "http://127.0.0.1:1".into(),
        fc.url.clone(),
    ));
    let (status, _) =
        send_webhook(router.clone(), "noweb", Some(WEBHOOK_SECRET), "{}").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send_webhook(router, "unknown", Some(WEBHOOK_SECRET), "{}").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn webhook_failed_delivery_returns_bad_gateway() {
    // point the webhook at a port with nothing listening
    let router = tg_bridge::build_router(state_with_webhook(
        "http://127.0.0.1:1".into(),
        "http://127.0.0.1:1/tg/callback".into(),
    ));
    let (status, _) =
        send_webhook(router, "mybot", Some(WEBHOOK_SECRET), "{}").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn passthrough_denied_when_disabled_for_client() {
    let mut cfg = test_config("http://127.0.0.1:1".into());
    cfg.clients.get_mut(CLIENT).unwrap().allow_passthrough = false;
    let state: SharedState = Arc::new(AppState {
        limiter: tg_bridge::ratelimit::RateLimiter::new(100),
        metrics: tg_bridge::metrics::Metrics::default(),
        nonces: tg_bridge::nonce::NonceCache::new(125),
        http: reqwest::Client::new(),
        cfg,
    });
    let router = tg_bridge::build_router(state);
    let (status, body) =
        send(router, "/v1/t/mybot/getMe", "{}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["description"], json!("tgb: passthrough not allowed for client"));
}

#[tokio::test]
async fn action_still_allowed_when_passthrough_disabled() {
    let mut cfg = test_config("http://127.0.0.1:1".into());
    cfg.clients.get_mut(CLIENT).unwrap().allow_passthrough = false;
    let state: SharedState = Arc::new(AppState {
        limiter: tg_bridge::ratelimit::RateLimiter::new(100),
        metrics: tg_bridge::metrics::Metrics::default(),
        nonces: tg_bridge::nonce::NonceCache::new(125),
        http: reqwest::Client::new(),
        cfg,
    });
    let router = tg_bridge::build_router(state);
    // unknown action is 404 (routing works); the point is it's not 403-passthrough
    let (status, _) = send(router, "/v1/a/notify", "{\"text\":\"x\"}", 0, SECRET.as_bytes(), None).await;
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "action must not be blocked by allow_passthrough=false"
    );
}

// ---------- replay protection ----------

#[tokio::test]
async fn duplicate_signed_request_rejected_as_replay() {
    let tg = fake_telegram().await;
    let state = test_state(tg.api_base);
    let router = tg_bridge::build_router(state.clone());
    let (status, _) =
        send(router.clone(), "/v1/t/mybot/getMe", "{\"a\":1}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::OK);

    // byte-identical request within the window is a replay
    let (status, body) =
        send(router, "/v1/t/mybot/getMe", "{\"a\":1}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["description"], json!("tgb: replay detected"));
    assert_eq!(tg.last.lock().unwrap().iter().count(), 1, "upstream hit once");
}

#[tokio::test]
async fn same_body_new_timestamp_is_not_replay() {
    let tg = fake_telegram().await;
    let router = tg_bridge::build_router(test_state(tg.api_base));
    for ts_offset in [0, -1] {
        let (status, _) = send(
            router.clone(),
            "/v1/t/mybot/getMe",
            "{\"a\":1}",
            ts_offset,
            SECRET.as_bytes(),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "ts_offset={ts_offset}");
    }
}

#[tokio::test]
async fn replay_check_disabled_when_configured_off() {
    let tg = fake_telegram().await;
    let mut cfg = test_config(tg.api_base);
    cfg.server.replay_protection = false;
    let state: SharedState = Arc::new(AppState {
        limiter: tg_bridge::ratelimit::RateLimiter::new(100),
        metrics: tg_bridge::metrics::Metrics::default(),
        nonces: tg_bridge::nonce::NonceCache::new(125),
        http: reqwest::Client::new(),
        cfg,
    });
    let router = tg_bridge::build_router(state);
    for _ in 0..2 {
        let (status, _) =
            send(router.clone(), "/v1/t/mybot/getMe", "{\"a\":1}", 0, SECRET.as_bytes(), None).await;
        assert_eq!(status, StatusCode::OK);
    }
}
