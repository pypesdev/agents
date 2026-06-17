//! HTTP POST webhook executor.
//!
//! Stored action JSON shape:
//! ```json
//! {
//!   "type": "webhook",
//!   "url": "https://example.com/hook",
//!   "headers": {"Authorization": "Bearer ..."},
//!   "payload": {"event": "hello", "data": 1}
//! }
//! ```
//!
//! `headers` and `payload` are optional. The executor POSTs `payload` (or `{}`
//! when absent) as JSON and forwards any caller-supplied headers verbatim. Auth
//! is not modeled beyond passthrough — secret resolution lives one layer up.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Maximum bytes read from a webhook response body (64 KiB).
///
/// Webhook targets are expected to return only a short acknowledgement;
/// reading an unbounded body is a denial-of-service surface. Pass a custom
/// value to [`execute_webhook`] to override per call site.
pub const RESPONSE_BODY_LIMIT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookAction {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub payload: Option<Value>,
}

#[derive(Debug)]
pub struct WebhookResult {
    pub status: u16,
    pub body: String,
    /// `true` when the response body exceeded `body_size_limit` and was
    /// truncated to fit. The captured prefix is in `body`.
    pub body_truncated: bool,
}

#[derive(Debug)]
pub enum WebhookError {
    InvalidHeaderName(String),
    InvalidHeaderValue(String),
    Transport(reqwest::Error),
    NonSuccessStatus { status: u16, body: String },
    /// The non-2xx response body exceeded `body_size_limit`. `captured_bytes`
    /// is how many bytes were buffered before the stream was dropped.
    ResponseTooLarge { status: u16, captured_bytes: usize },
}

impl std::fmt::Display for WebhookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebhookError::InvalidHeaderName(name) => write!(f, "invalid header name: {name}"),
            WebhookError::InvalidHeaderValue(name) => {
                write!(f, "invalid header value for {name}")
            }
            WebhookError::Transport(e) => write!(f, "webhook transport error: {e}"),
            WebhookError::NonSuccessStatus { status, body } => {
                write!(f, "webhook returned non-success status {status}: {body}")
            }
            WebhookError::ResponseTooLarge {
                status,
                captured_bytes,
            } => {
                write!(
                    f,
                    "webhook response body exceeded limit \
                     ({captured_bytes} bytes captured, status {status})"
                )
            }
        }
    }
}

impl std::error::Error for WebhookError {}

/// Buffer at most `limit` bytes from `response` using the chunk API.
///
/// Returns `(buf, truncated)` where `truncated` is `true` when the stream
/// contained more data than `limit` allowed.
async fn read_body_capped(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<(Vec<u8>, bool), reqwest::Error> {
    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    loop {
        match response.chunk().await? {
            None => break,
            Some(chunk) => {
                let space = limit.saturating_sub(buf.len());
                if chunk.len() > space {
                    buf.extend_from_slice(&chunk[..space]);
                    truncated = true;
                    break;
                }
                buf.extend_from_slice(&chunk);
            }
        }
    }
    Ok((buf, truncated))
}

/// POST `action.payload` as JSON to `action.url`, returning the response status
/// and body on success or a structured error otherwise.
///
/// `body_size_limit` caps how many bytes are buffered from the response body;
/// use [`RESPONSE_BODY_LIMIT_BYTES`] for the default 64 KiB cap.
///
/// - On **2xx** the body is capped and returned in [`WebhookResult`]; if
///   truncated, `body_truncated` is `true`.
/// - On **non-2xx** with a body within the cap, returns
///   [`WebhookError::NonSuccessStatus`].
/// - On **non-2xx** with a body that exceeds the cap, returns
///   [`WebhookError::ResponseTooLarge`] so a caller is never handed a giant
///   allocation in the error payload.
pub async fn execute_webhook(
    client: &reqwest::Client,
    action: &WebhookAction,
    body_size_limit: usize,
) -> Result<WebhookResult, WebhookError> {
    let mut request = client
        .post(&action.url)
        .json(action.payload.as_ref().unwrap_or(&json!({})));

    for (name, value) in &action.headers {
        let header_name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| WebhookError::InvalidHeaderName(name.clone()))?;
        let header_value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| WebhookError::InvalidHeaderValue(name.clone()))?;
        request = request.header(header_name, header_value);
    }

    let response = request.send().await.map_err(WebhookError::Transport)?;
    let status = response.status().as_u16();

    let (buf, truncated) = read_body_capped(response, body_size_limit)
        .await
        .map_err(WebhookError::Transport)?;

    if (200..300).contains(&status) {
        let body = String::from_utf8_lossy(&buf).into_owned();
        Ok(WebhookResult {
            status,
            body,
            body_truncated: truncated,
        })
    } else if truncated {
        Err(WebhookError::ResponseTooLarge {
            status,
            captured_bytes: buf.len(),
        })
    } else {
        let body = String::from_utf8_lossy(&buf).into_owned();
        Err(WebhookError::NonSuccessStatus { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn posts_payload_and_forwards_headers() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/hook"))
            .and(header("authorization", "Bearer abc123"))
            .and(body_json(json!({"event": "hello", "n": 1})))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .expect(1)
            .mount(&server)
            .await;

        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer abc123".to_string());

        let action = WebhookAction {
            url: format!("{}/hook", server.uri()),
            headers,
            payload: Some(json!({"event": "hello", "n": 1})),
        };

        let client = reqwest::Client::new();
        let result = execute_webhook(&client, &action, RESPONSE_BODY_LIMIT_BYTES)
            .await
            .unwrap();
        assert_eq!(result.status, 200);
        assert_eq!(result.body, r#"{"ok":true}"#);
        assert!(!result.body_truncated);
    }

    #[tokio::test]
    async fn defaults_to_empty_object_payload() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/empty"))
            .and(body_json(json!({})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let action = WebhookAction {
            url: format!("{}/empty", server.uri()),
            headers: HashMap::new(),
            payload: None,
        };

        let client = reqwest::Client::new();
        let result = execute_webhook(&client, &action, RESPONSE_BODY_LIMIT_BYTES)
            .await
            .unwrap();
        assert_eq!(result.status, 204);
        assert!(!result.body_truncated);
    }

    #[tokio::test]
    async fn surfaces_non_success_status_as_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/boom"))
            .respond_with(ResponseTemplate::new(500).set_body_string("kaboom"))
            .mount(&server)
            .await;

        let action = WebhookAction {
            url: format!("{}/boom", server.uri()),
            headers: HashMap::new(),
            payload: Some(json!({})),
        };

        let client = reqwest::Client::new();
        let err = execute_webhook(&client, &action, RESPONSE_BODY_LIMIT_BYTES)
            .await
            .unwrap_err();
        match err {
            WebhookError::NonSuccessStatus { status, body } => {
                assert_eq!(status, 500);
                assert_eq!(body, "kaboom");
            }
            other => panic!("expected NonSuccessStatus, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_invalid_header_name() {
        let mut headers = HashMap::new();
        headers.insert("bad header\n".to_string(), "value".to_string());

        let action = WebhookAction {
            url: "http://localhost:1/unused".to_string(),
            headers,
            payload: None,
        };

        let client = reqwest::Client::new();
        let err = execute_webhook(&client, &action, RESPONSE_BODY_LIMIT_BYTES)
            .await
            .unwrap_err();
        assert!(matches!(err, WebhookError::InvalidHeaderName(_)));
    }

    #[tokio::test]
    async fn truncates_oversized_success_body() {
        let server = MockServer::start().await;
        // One byte over the tiny cap we use for this test.
        let cap: usize = 16;
        let big_body = "x".repeat(cap + 1);

        Mock::given(method("POST"))
            .and(path("/big-ok"))
            .respond_with(ResponseTemplate::new(200).set_body_string(big_body))
            .expect(1)
            .mount(&server)
            .await;

        let action = WebhookAction {
            url: format!("{}/big-ok", server.uri()),
            headers: HashMap::new(),
            payload: Some(json!({})),
        };

        let client = reqwest::Client::new();
        let result = execute_webhook(&client, &action, cap).await.unwrap();
        assert_eq!(result.status, 200);
        assert!(result.body_truncated);
        assert_eq!(result.body.len(), cap);
        assert_eq!(result.body, "x".repeat(cap));
    }

    #[tokio::test]
    async fn returns_response_too_large_on_oversized_non_2xx_body() {
        let server = MockServer::start().await;
        let cap: usize = 16;
        let big_body = "e".repeat(cap + 1);

        Mock::given(method("POST"))
            .and(path("/big-err"))
            .respond_with(ResponseTemplate::new(500).set_body_string(big_body))
            .expect(1)
            .mount(&server)
            .await;

        let action = WebhookAction {
            url: format!("{}/big-err", server.uri()),
            headers: HashMap::new(),
            payload: Some(json!({})),
        };

        let client = reqwest::Client::new();
        let err = execute_webhook(&client, &action, cap).await.unwrap_err();
        match err {
            WebhookError::ResponseTooLarge {
                status,
                captured_bytes,
            } => {
                assert_eq!(status, 500);
                assert_eq!(captured_bytes, cap);
            }
            other => panic!("expected ResponseTooLarge, got {other:?}"),
        }
    }
}
