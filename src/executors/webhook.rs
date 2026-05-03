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
}

#[derive(Debug)]
pub enum WebhookError {
    InvalidHeaderName(String),
    InvalidHeaderValue(String),
    Transport(reqwest::Error),
    NonSuccessStatus { status: u16, body: String },
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
        }
    }
}

impl std::error::Error for WebhookError {}

/// POST `action.payload` as JSON to `action.url`, returning the response status
/// and body on success or a structured error otherwise.
///
/// 2xx is success; anything else surfaces as `NonSuccessStatus` so a caller can
/// decide whether to retry without re-parsing the response.
pub async fn execute_webhook(
    client: &reqwest::Client,
    action: &WebhookAction,
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
    let body = response.text().await.map_err(WebhookError::Transport)?;

    if (200..300).contains(&status) {
        Ok(WebhookResult { status, body })
    } else {
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
        let result = execute_webhook(&client, &action).await.unwrap();
        assert_eq!(result.status, 200);
        assert_eq!(result.body, r#"{"ok":true}"#);
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
        let result = execute_webhook(&client, &action).await.unwrap();
        assert_eq!(result.status, 204);
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
        let err = execute_webhook(&client, &action).await.unwrap_err();
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
        let err = execute_webhook(&client, &action).await.unwrap_err();
        assert!(matches!(err, WebhookError::InvalidHeaderName(_)));
    }
}
