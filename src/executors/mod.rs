//! Action executors — turn stored `Agent.actions` strings into real side effects.
//!
//! Each entry in `Agent.actions: Vec<String>` is interpreted as a JSON-encoded
//! [`Action`]. The executor module parses the spec, dispatches to the matching
//! backend (currently `webhook`), and returns a structured [`ExecutionOutcome`]
//! per action.
//!
//! New executor types should add a variant to [`Action`] and a sibling module.

pub mod webhook;

use serde::{Deserialize, Serialize};

use crate::agent::agent::Agent;

/// A typed action spec stored as JSON inside `Agent.actions`.
///
/// The discriminator is the `type` field, e.g. `{"type":"webhook", ...}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Webhook(webhook::WebhookAction),
}

/// Result of dispatching one action.
#[derive(Debug)]
pub enum ExecutionOutcome {
    Webhook(Result<webhook::WebhookResult, webhook::WebhookError>),
    /// The stored string did not parse as a known action spec.
    Unrecognized {
        raw: String,
        error: serde_json::Error,
    },
}

impl ExecutionOutcome {
    pub fn is_ok(&self) -> bool {
        match self {
            ExecutionOutcome::Webhook(r) => r.is_ok(),
            ExecutionOutcome::Unrecognized { .. } => false,
        }
    }
}

/// Parse a single stored action string into a typed [`Action`].
pub fn parse_action(raw: &str) -> Result<Action, serde_json::Error> {
    serde_json::from_str(raw)
}

/// Execute every action stored on `agent` in order, returning one outcome per
/// entry. Unrecognized strings produce an `Unrecognized` outcome rather than
/// halting the loop — this preserves the existing free-form `Vec<String>`
/// storage while letting typed executors opt in.
pub async fn process_actions(agent: &Agent) -> Vec<ExecutionOutcome> {
    let client = reqwest::Client::new();
    let mut outcomes = Vec::with_capacity(agent.actions.len());
    for raw in &agent.actions {
        match parse_action(raw) {
            Ok(Action::Webhook(spec)) => {
                outcomes.push(ExecutionOutcome::Webhook(
                    webhook::execute_webhook(&client, &spec).await,
                ));
            }
            Err(error) => outcomes.push(ExecutionOutcome::Unrecognized {
                raw: raw.clone(),
                error,
            }),
        }
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_webhook_action() {
        let raw = r#"{"type":"webhook","url":"https://example.com/hook","payload":{"hello":"world"}}"#;
        match parse_action(raw).unwrap() {
            Action::Webhook(w) => {
                assert_eq!(w.url, "https://example.com/hook");
                assert_eq!(w.payload.unwrap()["hello"], "world");
            }
        }
    }

    #[test]
    fn unknown_type_is_a_parse_error() {
        let raw = r#"{"type":"telegram","chat":"42"}"#;
        assert!(parse_action(raw).is_err());
    }
}
