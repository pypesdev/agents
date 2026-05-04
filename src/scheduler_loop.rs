//! Background scheduler loop that drives cron actions stored on agents.
//!
//! The HTTP server's tokio runtime spawns [`spawn`] once on boot. The loop
//! reads every `Action::Cron(_)` entry across all agents in [`PickleDb`],
//! builds an in-process [`Scheduler`], and fires each entry's wrapped action
//! when it falls due. After every fire the entry is advanced via
//! `Scheduler::advance(idx, Utc::now())`.
//!
//! Mutations to the agents store should call [`SchedulerHandle::reload`] so
//! the loop rebuilds from the current store state. Reloads are coalesced —
//! multiple notifications between rebuilds collapse to one.
//!
//! Out of scope (deferred): per-tenant isolation, persistent missed-fire
//! catchup across daemon restarts (the loop starts fresh from `Utc::now()`),
//! and distributed scheduling.
//!
//! Logs go to stderr so the daemon's `~/.agents/tmp/daemon.err` captures
//! every fire and rebuild.

use std::sync::{Arc, RwLock};

use chrono::Utc;
use pickledb::PickleDb;
use tokio::sync::Notify;

use crate::agent::agent::Agent;
use crate::executors::cron::{CronAction, Scheduler};
use crate::executors::webhook::execute_webhook;
use crate::executors::{parse_action, Action};

/// Handle to the running scheduler loop. Cheap to clone — keeps the loop
/// reachable from axum state and CLI surfaces that need to signal a reload.
#[derive(Clone)]
pub struct SchedulerHandle {
    reload: Arc<Notify>,
}

impl SchedulerHandle {
    /// Notify the scheduler loop to rebuild its schedule from the agents
    /// store. Non-blocking. Multiple calls between rebuilds coalesce.
    pub fn reload(&self) {
        self.reload.notify_one();
    }

    /// Construct a handle without a backing task. Useful for tests and CLI
    /// paths that build axum state without spawning a real loop.
    pub fn detached() -> Self {
        Self {
            reload: Arc::new(Notify::new()),
        }
    }
}

/// Walk every agent in `db`, parse each stored action string, and collect
/// the `Action::Cron(_)` entries. Unrecognized strings are skipped — they
/// already surface via `executors::process_actions` on `pypes agent <NAME> run`.
pub fn collect_cron_actions(db: &PickleDb) -> Vec<CronAction> {
    let mut actions = Vec::new();
    for key in db.get_all() {
        let agent = match db.get::<Agent>(&key) {
            Some(a) => a,
            None => continue,
        };
        for raw in &agent.actions {
            if let Ok(Action::Cron(c)) = parse_action(raw) {
                actions.push(c);
            }
        }
    }
    actions
}

/// Spawn the scheduler loop on the current tokio runtime. Returns a handle
/// the caller can keep in shared state.
pub fn spawn(db: Arc<RwLock<PickleDb>>) -> SchedulerHandle {
    let reload = Arc::new(Notify::new());
    let handle = SchedulerHandle {
        reload: reload.clone(),
    };
    tokio::spawn(async move {
        run(db, reload).await;
    });
    handle
}

async fn run(db: Arc<RwLock<PickleDb>>, reload: Arc<Notify>) {
    let mut scheduler = build_scheduler(&db);
    let client = reqwest::Client::new();
    eprintln!(
        "[scheduler] started with {} cron action(s)",
        scheduler.entries().len()
    );

    loop {
        tokio::select! {
            _ = reload.notified() => {
                scheduler = build_scheduler(&db);
                eprintln!(
                    "[scheduler] reloaded with {} cron action(s)",
                    scheduler.entries().len()
                );
            }
            due = wait_for_due(&scheduler) => {
                for idx in due {
                    let entry = scheduler.entries()[idx].clone();
                    fire(&client, idx, &entry.action).await;
                    match scheduler.advance(idx, Utc::now()) {
                        Ok(next) => eprintln!(
                            "[scheduler] entry {idx} `{}` next fire {next}",
                            entry.action.expression,
                        ),
                        Err(e) => eprintln!(
                            "[scheduler] entry {idx} `{}` advance failed: {e}",
                            entry.action.expression,
                        ),
                    }
                }
            }
        }
    }
}

fn build_scheduler(db: &Arc<RwLock<PickleDb>>) -> Scheduler {
    let actions = {
        let guard = db.read().expect("agents db lock poisoned");
        collect_cron_actions(&guard)
    };
    if actions.is_empty() {
        return empty_scheduler();
    }
    match Scheduler::from_actions(&actions, Utc::now()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[scheduler] failed to build: {e} — running with empty schedule");
            empty_scheduler()
        }
    }
}

fn empty_scheduler() -> Scheduler {
    Scheduler::from_actions(&[], Utc::now()).expect("empty scheduler always builds")
}

async fn wait_for_due(scheduler: &Scheduler) -> Vec<usize> {
    if scheduler.entries().is_empty() {
        // Park indefinitely; the outer select! reacts to reload signals.
        std::future::pending::<Vec<usize>>().await
    } else {
        scheduler.next_due().await.unwrap_or_default()
    }
}

async fn fire(client: &reqwest::Client, idx: usize, action: &CronAction) {
    match action.action.as_ref() {
        Action::Webhook(spec) => match execute_webhook(client, spec).await {
            Ok(res) => eprintln!(
                "[scheduler] entry {idx} `{}` fired webhook {} → status={} ({} bytes)",
                action.expression,
                spec.url,
                res.status,
                res.body.len(),
            ),
            Err(e) => eprintln!(
                "[scheduler] entry {idx} `{}` webhook {} failed: {e}",
                action.expression, spec.url,
            ),
        },
        Action::Cron(_) => eprintln!(
            "[scheduler] entry {idx} `{}` wraps another cron action — skipping (only Action::Webhook is fired today)",
            action.expression,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::agent::Agent;
    use crate::executors::cron::CronAction;
    use crate::executors::webhook::WebhookAction;
    use pickledb::{PickleDbDumpPolicy, SerializationMethod};
    use std::collections::HashMap;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::timeout;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fresh_db() -> (tempfile::TempDir, PickleDb) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("agents.db");
        let db = PickleDb::new(path, PickleDbDumpPolicy::NeverDump, SerializationMethod::Json);
        (dir, db)
    }

    fn store_cron_agent(db: &mut PickleDb, name: &str, expression: &str, url: &str) {
        let cron = CronAction {
            expression: expression.to_string(),
            action: Box::new(Action::Webhook(WebhookAction {
                url: url.to_string(),
                headers: HashMap::new(),
                payload: Some(serde_json::json!({"event": "cron.tick"})),
            })),
        };
        let stored = serde_json::to_string(&Action::Cron(cron)).expect("serialize cron action");
        let agent = Agent {
            name: name.to_string(),
            inputs: Vec::new(),
            actions: vec![stored],
        };
        db.set(name, &agent).expect("store agent");
    }

    #[test]
    fn collect_cron_actions_returns_only_cron_entries_across_all_agents() {
        let (_dir, mut db) = fresh_db();

        // Agent with a cron action (counts) and a webhook action (skipped — not cron).
        let cron_raw = serde_json::to_string(&Action::Cron(CronAction {
            expression: "*/5 * * * *".to_string(),
            action: Box::new(Action::Webhook(WebhookAction {
                url: "http://127.0.0.1:1/unused".to_string(),
                headers: HashMap::new(),
                payload: None,
            })),
        }))
        .unwrap();
        let webhook_raw = serde_json::to_string(&Action::Webhook(WebhookAction {
            url: "http://127.0.0.1:1/unused".to_string(),
            headers: HashMap::new(),
            payload: None,
        }))
        .unwrap();
        db.set(
            "alpha",
            &Agent {
                name: "alpha".to_string(),
                inputs: Vec::new(),
                actions: vec![cron_raw, webhook_raw, "not json".to_string()],
            },
        )
        .unwrap();

        // Agent with no cron actions (skipped entirely).
        db.set(
            "beta",
            &Agent {
                name: "beta".to_string(),
                inputs: Vec::new(),
                actions: vec![],
            },
        )
        .unwrap();

        let cron_actions = collect_cron_actions(&db);
        assert_eq!(cron_actions.len(), 1);
        assert_eq!(cron_actions[0].expression, "*/5 * * * *");
    }

    /// Integration test mirroring the `examples/cron_executor.rs` pattern:
    /// store a cron agent, spawn the loop, wait for the mock receiver to be hit.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn loop_fires_stored_cron_action_against_mock_receiver() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .expect(1..)
            .mount(&server)
            .await;

        let (_dir, mut db) = fresh_db();
        store_cron_agent(
            &mut db,
            "ticker",
            "* * * * * *", // every second
            &format!("{}/hook", server.uri()),
        );
        let db = Arc::new(RwLock::new(db));

        let _handle = spawn(db);

        // Give the loop up to ~2.5s to fire on the next per-second boundary.
        let received = timeout(Duration::from_millis(2500), async {
            loop {
                if !server.received_requests().await.unwrap_or_default().is_empty() {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or(false);

        assert!(received, "scheduler loop did not fire the cron action within 2.5s");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reload_picks_up_newly_added_agents() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .expect(1..)
            .mount(&server)
            .await;

        // Start with an empty db so the loop has no schedule.
        let (_dir, db) = fresh_db();
        let db = Arc::new(RwLock::new(db));
        let handle = spawn(db.clone());

        // Confirm we really start empty: brief wait shows no requests fired.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty());

        // Mutate the db (mirrors what the POST /agents handler does), then reload.
        store_cron_agent(
            &mut db.write().unwrap(),
            "ticker",
            "* * * * * *",
            &format!("{}/hook", server.uri()),
        );
        handle.reload();

        let received = timeout(Duration::from_millis(2500), async {
            loop {
                if !server.received_requests().await.unwrap_or_default().is_empty() {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or(false);

        assert!(
            received,
            "scheduler did not pick up newly added agent after reload"
        );
    }
}
