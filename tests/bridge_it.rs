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
    Client, Config, Metrics as MetricsCfg, RateLimit, Server, Telegram,
};
use tg_bridge::{AppState, SharedState};

const SECRET: &str = "test-secret-0123456789abcdef";
const CLIENT: &str = "testclient";

fn test_config(api_base: String) -> Config {
    Config {
        server: Server {
            listen: "127.0.0.1:0".into(),
            max_body_bytes: 65536,
            request_timeout: Duration::from_secs(5),
            timestamp_window_secs: 60,
        },
        telegram: Telegram { api_base },
        rate_limit: RateLimit {
            requests_per_minute: 1000,
        },
        metrics: MetricsCfg { enabled: true },
        bots: HashMap::from([("salut".to_string(), "123456:FAKE-TOKEN".to_string())]),
        clients: HashMap::from([(
            CLIENT.to_string(),
            Client {
                secret: SECRET.into(),
                allowed_ips: vec![],
                bots: vec!["salut".into()],
                methods_allowlist: Some(vec!["getMe".into(), "sendMessage".into()]),
            },
        )]),
        actions: HashMap::from([(
            "notify".to_string(),
            serde_json::from_value(json!({
                "client": CLIENT,
                "bot": "salut",
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

/// Spawns a fake api.telegram.org that records the last request body.
async fn fake_telegram() -> (String, Arc<std::sync::Mutex<Option<serde_json::Value>>>) {
    let last = Arc::new(std::sync::Mutex::new(None));
    let last_clone = last.clone();
    let app = axum::Router::new().route(
        "/bot123456:FAKE-TOKEN/{method}",
        axum::routing::post(
            |axum::extract::Path(method): axum::extract::Path<String>,
             body: axum::body::Bytes| async move {
                *last_clone.lock().unwrap() =
                    Some(serde_json::from_slice(&body).unwrap());
                axum::Json(json!({"ok": true, "result": {"method": method}}))
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), last)
}

fn test_state(api_base: String) -> SharedState {
    Arc::new(AppState {
        cfg: test_config(api_base),
        http: reqwest::Client::new(),
        limiter: tg_bridge::ratelimit::RateLimiter::new(3),
        metrics: tg_bridge::metrics::Metrics::default(),
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
    let req = Request::builder()
        .method("POST")
        .uri(format!("http://bridge.test{method_path}"))
        .header("content-type", "application/json")
        .header("x-tgb-client", CLIENT)
        .header(
            "x-tgb-timestamp",
            (tg_bridge::now_secs() + ts_offset).to_string(),
        )
        .header(
            "x-tgb-signature",
            sig_override.unwrap_or_else(|| sign_hex(secret, tg_bridge::now_secs() + ts_offset, body.as_bytes())),
        )
        .extension(ConnectInfo("127.0.0.1:55555".parse::<SocketAddr>().unwrap()))
        .body(Body::from(body.to_owned()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
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
        .uri("http://bridge.test/v1/t/salut/getMe")
        .extension(ConnectInfo("127.0.0.1:55555".parse::<SocketAddr>().unwrap()))
        .body(Body::from("{}"))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bad_signature_rejected() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let (status, body) = send(router, "/v1/t/salut/getMe", "{}", 0, b"wrong", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["description"], json!("tgb: bad signature"));
}

#[tokio::test]
async fn stale_timestamp_rejected() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    let (status, _) = send(router, "/v1/t/salut/getMe", "{}", -120, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn passthrough_forwards_to_telegram() {
    let (api_base, _last) = fake_telegram().await;
    let router = tg_bridge::build_router(test_state(api_base));
    let (status, body) = send(router, "/v1/t/salut/getMe", "{\"a\":1}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["result"]["method"], json!("getMe"));
}

#[tokio::test]
async fn disallowed_method_rejected() {
    let router = tg_bridge::build_router(test_state("http://127.0.0.1:1".into()));
    // deleteWebhook is not in methods_allowlist
    let (status, body) =
        send(router, "/v1/t/salut/deleteWebhook", "{}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["description"], json!("tgb: method not allowed for client"));
}

#[tokio::test]
async fn action_renders_template_and_calls_telegram() {
    let (api_base, last) = fake_telegram().await;
    let router = tg_bridge::build_router(test_state(api_base));
    let payload = json!({"level": "warn", "text": "disk 91%"}).to_string();
    let (status, body) =
        send(router, "/v1/a/notify", &payload, 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["telegram_ok"], json!(true));
    let sent = last.lock().unwrap().clone().unwrap();
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
    let (api_base, _last) = fake_telegram().await;
    let state = test_state(api_base);
    let router = tg_bridge::build_router(state.clone());
    for _ in 0..3 {
        let r = send(
            router.clone(),
            "/v1/t/salut/getMe",
            "{\"a\":1}",
            0,
            SECRET.as_bytes(),
            None,
        )
        .await;
        assert_eq!(r.0, StatusCode::OK);
    }
    let (status, body) =
        send(router, "/v1/t/salut/getMe", "{\"a\":1}", 0, SECRET.as_bytes(), None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["description"], json!("tgb: rate limited"));
}

#[tokio::test]
async fn metrics_exposes_counters() {
    let (api_base, _last) = fake_telegram().await;
    let state = test_state(api_base);
    let router = tg_bridge::build_router(state.clone());
    let _ = send(router.clone(), "/v1/t/salut/getMe", "{\"a\":1}", 0, SECRET.as_bytes(), None).await;

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
