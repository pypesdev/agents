//! Action executors — turn stored `Agent.actions` strings into real side effects.
//!
//! Each entry in `Agent.actions: Vec<String>` is interpreted as a JSON-encoded
//! [`Action`]. The executor module parses the spec, dispatches to the matching
//! backend (`webhook` for immediate fire, `cron` for scheduled fire), and
//! returns a structured [`ExecutionOutcome`] per action.
//!
//! New executor types should add a variant to [`Action`] and a sibling module.

pub mod cron;
pub mod webhook;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::agent::agent::Agent;

/// A typed action spec stored as JSON inside `Agent.actions`.
///
/// The discriminator is the `type` field, e.g. `{"type":"webhook", ...}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Webhook(webhook::WebhookAction),
    Cron(cron::CronAction),
}

/// Result of dispatching one action.
#[derive(Debug)]
pub enum ExecutionOutcome {
    Webhook(Result<webhook::WebhookResult, webhook::WebhookError>),
    /// Cron actions don't fire inline — they're handed to the [`cron::Scheduler`].
    /// `process_actions` reports the next computed fire time so the caller can
    /// see scheduling worked without blocking the run.
    Cron(Result<cron::CronScheduled, cron::CronError>),
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
            ExecutionOutcome::Cron(r) => r.is_ok(),
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
///
/// Cron actions are not fired here; instead we compute and report the next
/// fire time. Real firing happens in [`cron::Scheduler`].
pub async fn process_actions(agent: &Agent) -> Vec<ExecutionOutcome> {
    let client = reqwest::Client::new();
    let now = Utc::now();
    let mut outcomes = Vec::with_capacity(agent.actions.len());
    for raw in &agent.actions {
        match parse_action(raw) {
            Ok(Action::Webhook(spec)) => {
                outcomes.push(ExecutionOutcome::Webhook(
                    webhook::execute_webhook(&client, &spec).await,
                ));
            }
            Ok(Action::Cron(spec)) => {
                let scheduled = cron::next_fire_after(&spec.expression, now).map(|next_fire| {
                    cron::CronScheduled {
                        expression: spec.expression.clone(),
                        next_fire,
                    }
                });
                outcomes.push(ExecutionOutcome::Cron(scheduled));
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
            other => panic!("expected webhook, got {other:?}"),
        }
    }

    #[test]
    fn parses_cron_action_wrapping_webhook() {
        let raw = r#"{
            "type":"cron",
            "expression":"*/5 * * * *",
            "action":{"type":"webhook","url":"https://example.com/hook"}
        }"#;
        match parse_action(raw).unwrap() {
            Action::Cron(c) => {
                assert_eq!(c.expression, "*/5 * * * *");
                assert!(matches!(*c.action, Action::Webhook(_)));
            }
            other => panic!("expected cron, got {other:?}"),
        }
    }

    #[test]
    fn unknown_type_is_a_parse_error() {
        let raw = r#"{"type":"telegram","chat":"42"}"#;
        assert!(parse_action(raw).is_err());
    }
}
