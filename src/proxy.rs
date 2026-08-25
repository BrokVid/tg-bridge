use axum::response::{IntoResponse, Response};

/// Sends raw bytes to `{api_base}/bot{token}/{method}` and returns the
/// upstream reqwest::Response. `content_type` is forwarded verbatim
/// (JSON for normal calls, multipart/form-data for file uploads).
async fn send(
    http: &reqwest::Client,
    api_base: &str,
    token: &str,
    method: &str,
    content_type: Option<String>,
    body: axum::body::Bytes,
) -> anyhow::Result<reqwest::Response> {
    let url = format!("{api_base}/bot{token}/{method}");
    Ok(http
        .post(&url)
        .header(
            reqwest::header::CONTENT_TYPE,
            content_type.unwrap_or_else(|| "application/json".to_owned()),
        )
        .body(body)
        .send()
        .await?)
}

/// Passthrough: returns the Telegram response verbatim (status + body).
#[allow(clippy::too_many_arguments)]
pub async fn passthrough_response(
    http: &reqwest::Client,
    api_base: &str,
    token: &str,
    _alias: &str,
    method: &str,
    content_type: Option<String>,
    body: axum::body::Bytes,
) -> anyhow::Result<Response> {
    let upstream = send(http, api_base, token, method, content_type, body).await?;
    let status = axum::http::StatusCode::from_u16(upstream.status().as_u16())
        .unwrap_or(axum::http::StatusCode::BAD_GATEWAY);
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = upstream.bytes().await?;

    let mut resp = (status, bytes).into_response();
    if let Some(ct) = content_type {
        if let Ok(v) = axum::http::HeaderValue::from_str(&ct) {
            resp.headers_mut()
                .insert(reqwest::header::CONTENT_TYPE, v);
        }
    }
    Ok(resp)
}

/// For actions: parses the Telegram JSON response.
pub async fn send_json(
    http: &reqwest::Client,
    api_base: &str,
    token: &str,
    bot_alias: &str,
    method: &str,
    body: Vec<u8>,
) -> anyhow::Result<(u16, serde_json::Value)> {
    let upstream = send(
        http,
        api_base,
        token,
        method,
        Some("application/json".to_owned()),
        body.into(),
    )
    .await?;
    let status = upstream.status().as_u16();
    let value = serde_json::from_slice(&upstream.bytes().await?)?;
    tracing::trace!(bot = %bot_alias, status, "upstream done");
    Ok((status, value))
}
