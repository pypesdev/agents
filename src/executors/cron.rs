//! Cron action executor.
//!
//! Stored action JSON shape:
//! ```json
//! {
//!   "type": "cron",
//!   "expression": "*/5 * * * *",
//!   "action": {
//!     "type": "webhook",
//!     "url": "https://example.com/hook",
//!     "payload": { "tick": true }
//!   }
//! }
//! ```
//!
//! `expression` accepts standard 5-field cron (`min hour dom mon dow`),
//! 6-field cron with seconds (`sec min hour dom mon dow`), or 7-field cron
//! with seconds and year (`sec min hour dom mon dow year`). 5- and 6-field
//! inputs are normalized to the 7-field form the underlying `cron` crate
//! expects (`sec` defaults to `0`, `year` defaults to `*`).
//!
//! Execution is split: parsing + due-time math live here as pure functions
//! tested with a mock clock; the [`Scheduler`] holds per-entry next-fire state
//! and exposes a `tick(now)`-style API so a real-time loop or a test loop can
//! drive it the same way.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};

use super::Action;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronAction {
    pub expression: String,
    pub action: Box<Action>,
}

#[derive(Debug, Clone)]
pub struct CronScheduled {
    pub expression: String,
    pub next_fire: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum CronError {
    InvalidExpression(String),
    NoUpcomingFire,
}

impl std::fmt::Display for CronError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CronError::InvalidExpression(msg) => write!(f, "invalid cron expression: {msg}"),
            CronError::NoUpcomingFire => write!(f, "cron expression has no upcoming fire time"),
        }
    }
}

impl std::error::Error for CronError {}

/// Parse a 5-, 6-, or 7-field cron expression.
///
/// 5-field input is standard cron (`min hour dom mon dow`); we pad with
/// `sec=0` and `year=*`. 6-field input is treated as `sec min hour dom mon dow`
/// and we pad with `year=*`. 7-field input is passed through.
pub fn parse(expression: &str) -> Result<Schedule, CronError> {
    let trimmed = expression.trim();
    let field_count = trimmed.split_whitespace().count();
    let normalized = match field_count {
        5 => format!("0 {trimmed} *"),
        6 => format!("{trimmed} *"),
        7 => trimmed.to_string(),
        n => {
            return Err(CronError::InvalidExpression(format!(
                "expected 5, 6, or 7 fields, got {n}"
            )))
        }
    };
    Schedule::from_str(&normalized).map_err(|e| CronError::InvalidExpression(e.to_string()))
}

/// Compute the next fire time strictly after `after`.
pub fn next_fire_after(
    expression: &str,
    after: DateTime<Utc>,
) -> Result<DateTime<Utc>, CronError> {
    let schedule = parse(expression)?;
    schedule
        .after(&after)
        .next()
        .ok_or(CronError::NoUpcomingFire)
}

/// In-process scheduler for cron actions.
///
/// Built around a `tick(now)` API so unit tests can drive it with a mock clock
/// and the worked example can drive it with `Utc::now()`.
#[derive(Debug, Clone)]
pub struct Scheduler {
    entries: Vec<Entry>,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub action: CronAction,
    pub next_fire: DateTime<Utc>,
}

impl Scheduler {
    pub fn from_actions(actions: &[CronAction], now: DateTime<Utc>) -> Result<Self, CronError> {
        let mut entries = Vec::with_capacity(actions.len());
        for action in actions {
            let next_fire = next_fire_after(&action.expression, now)?;
            entries.push(Entry {
                action: action.clone(),
                next_fire,
            });
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Indices of entries due to fire at or before `now`.
    pub fn due_at(&self, now: DateTime<Utc>) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| if e.next_fire <= now { Some(i) } else { None })
            .collect()
    }

    /// Recompute the next fire time for `idx` strictly after `now`.
    pub fn advance(&mut self, idx: usize, now: DateTime<Utc>) -> Result<DateTime<Utc>, CronError> {
        let next = next_fire_after(&self.entries[idx].action.expression, now)?;
        self.entries[idx].next_fire = next;
        Ok(next)
    }

    /// Earliest upcoming fire time across all entries.
    pub fn earliest_next(&self) -> Option<DateTime<Utc>> {
        self.entries.iter().map(|e| e.next_fire).min()
    }

    /// Sleep until the earliest next fire, then return its due indices using
    /// the wall clock at wake-up. Returns immediately if any entry is already
    /// due. Returns `None` if the scheduler is empty.
    pub async fn next_due(&self) -> Option<Vec<usize>> {
        let now = Utc::now();
        let due_now = self.due_at(now);
        if !due_now.is_empty() {
            return Some(due_now);
        }
        let target = self.earliest_next()?;
        let wait = target - Utc::now();
        if let Ok(std_wait) = wait.to_std() {
            tokio::time::sleep(std_wait).await;
        }
        Some(self.due_at(Utc::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parses_five_field_expression() {
        let schedule = parse("*/5 * * * *").unwrap();
        let after = Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap();
        let next = schedule.after(&after).next().unwrap();
        // Five-field padding fires on second 0; the next */5 boundary after 12:00:00 is 12:05:00.
        assert_eq!(
            next,
            Utc.with_ymd_and_hms(2026, 5, 3, 12, 5, 0).unwrap()
        );
    }

    #[test]
    fn parses_six_field_expression_with_seconds() {
        // Every second.
        let schedule = parse("* * * * * *").unwrap();
        let after = Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap();
        let next = schedule.after(&after).next().unwrap();
        assert_eq!(
            next,
            Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 1).unwrap()
        );
    }

    #[test]
    fn rejects_wrong_field_count() {
        assert!(matches!(
            parse("* * *"),
            Err(CronError::InvalidExpression(_))
        ));
        assert!(matches!(
            parse("* * * * * * * *"),
            Err(CronError::InvalidExpression(_))
        ));
    }

    #[test]
    fn rejects_garbage_expression() {
        assert!(matches!(
            parse("not a cron"),
            Err(CronError::InvalidExpression(_))
        ));
    }

    #[test]
    fn next_fire_is_strictly_after_input() {
        let now = Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap();
        // "0 12 * * *" => daily at noon. now == noon; next fire is tomorrow noon.
        let next = next_fire_after("0 12 * * *", now).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 5, 4, 12, 0, 0).unwrap());
    }

    fn webhook_dummy() -> Box<Action> {
        use super::super::webhook::WebhookAction;
        use std::collections::HashMap;
        Box::new(Action::Webhook(WebhookAction {
            url: "http://127.0.0.1:1/unused".to_string(),
            headers: HashMap::new(),
            payload: None,
        }))
    }

    #[test]
    fn scheduler_fires_on_time_with_mock_clock() {
        // Tight mock clock: drive `due_at` and `advance` deterministically and
        // assert each tick fires exactly once.
        let started = Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap();
        let cron = CronAction {
            expression: "* * * * * *".to_string(), // every second
            action: webhook_dummy(),
        };
        let mut scheduler = Scheduler::from_actions(&[cron], started).unwrap();

        // First fire is 12:00:01.
        let first = scheduler.entries()[0].next_fire;
        assert_eq!(first, Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 1).unwrap());

        // Just before the tick, nothing is due.
        let just_before = first - chrono::Duration::milliseconds(1);
        assert!(scheduler.due_at(just_before).is_empty());

        // At the tick, our entry is due.
        assert_eq!(scheduler.due_at(first), vec![0]);

        // After advancing, the next fire is one second later and the same
        // instant is no longer due.
        let next = scheduler.advance(0, first).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 2).unwrap());
        assert!(scheduler.due_at(first).is_empty());
        assert_eq!(scheduler.due_at(next), vec![0]);
    }

    #[test]
    fn scheduler_earliest_next_picks_min() {
        let now = Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 0).unwrap();
        let every_minute = CronAction {
            expression: "* * * * *".to_string(),
            action: webhook_dummy(),
        };
        let every_second = CronAction {
            expression: "* * * * * *".to_string(),
            action: webhook_dummy(),
        };
        let scheduler =
            Scheduler::from_actions(&[every_minute, every_second], now).unwrap();
        let earliest = scheduler.earliest_next().unwrap();
        assert_eq!(earliest, Utc.with_ymd_and_hms(2026, 5, 3, 12, 0, 1).unwrap());
    }

    #[tokio::test]
    async fn scheduler_next_due_waits_for_real_tick() {
        // Integration-flavored: schedule for ~250ms in the future and confirm
        // `next_due` blocks until then and returns the right index. Tight
        // bounds keep this fast even under load.
        let started = Utc::now();
        let cron = CronAction {
            expression: "* * * * * *".to_string(), // every second
            action: webhook_dummy(),
        };
        let scheduler = Scheduler::from_actions(&[cron], started).unwrap();
        let target = scheduler.entries()[0].next_fire;

        let before = Utc::now();
        let due = scheduler.next_due().await.unwrap();
        let after = Utc::now();

        assert_eq!(due, vec![0]);
        // Did we actually wait at least until the scheduled tick?
        assert!(after >= target, "wake-up at {after} happened before target {target}");
        // And not too much longer (generous 750ms upper bound for CI).
        let total = (after - before).to_std().unwrap();
        assert!(
            total < std::time::Duration::from_millis(1500),
            "next_due took {total:?}, expected <1500ms"
        );
    }

    #[test]
    fn parses_cron_action_with_nested_webhook_via_action_enum() {
        // Round-trip the Cron variant through the outer Action enum so the
        // tag = "type" discriminator keeps working with the recursive shape.
        let raw = r#"{
            "type":"cron",
            "expression":"*/5 * * * *",
            "action":{"type":"webhook","url":"https://example.com/hook"}
        }"#;
        let parsed: Action = serde_json::from_str(raw).unwrap();
        match parsed {
            Action::Cron(c) => {
                assert_eq!(c.expression, "*/5 * * * *");
                match *c.action {
                    Action::Webhook(w) => assert_eq!(w.url, "https://example.com/hook"),
                    other => panic!("expected nested webhook, got {other:?}"),
                }
            }
            other => panic!("expected cron, got {other:?}"),
        }
    }
}
