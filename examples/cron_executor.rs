//! End-to-end demo of the cron action executor.
//!
//! Run with: `cargo run --example cron_executor`
//!
//! Steps:
//! 1. Spin up a local axum mock receiver on an ephemeral port.
//! 2. Build a `CronAction` whose target is a `WebhookAction` pointed at the
//!    mock. The expression `* * * * * *` fires on every second so the example
//!    terminates well under two seconds.
//! 3. Drive the in-process [`Scheduler`] for a single tick: wait until the next
//!    fire, dispatch the wrapped webhook, and print the captured payload.
//!
//! No external services are required.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{extract::State, routing::post, Json, Router};
use chrono::Utc;
use pypes::executors::cron::{CronAction, Scheduler};
use pypes::executors::webhook::{execute_webhook, WebhookAction};
use pypes::executors::Action;
use serde_json::{json, Value};

type Captured = Arc<Mutex<Vec<Value>>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));

    let app = Router::new()
        .route("/hook", post(receive))
        .with_state(captured.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let webhook = WebhookAction {
        url: format!("http://{}/hook", addr),
        headers: HashMap::new(),
        payload: Some(json!({ "event": "cron.tick", "n": 1 })),
    };

    let cron_action = CronAction {
        // 6-field cron: every second. The example fires once and exits.
        expression: "* * * * * *".to_string(),
        action: Box::new(Action::Webhook(webhook.clone())),
    };

    // Round-trip the spec through JSON to mirror how it would be stored on an
    // `Agent.actions` entry.
    let stored = serde_json::to_string(&Action::Cron(cron_action.clone()))?;
    println!("→ stored action: {stored}");

    let started = Utc::now();
    let scheduler = Scheduler::from_actions(&[cron_action], started)?;
    let next = scheduler.entries()[0].next_fire;
    println!("⏲ next fire scheduled at {next} (in {} ms)", (next - started).num_milliseconds());

    let due = scheduler.next_due().await.unwrap_or_default();
    let client = reqwest::Client::new();
    for idx in due {
        let entry = &scheduler.entries()[idx];
        match entry.action.action.as_ref() {
            Action::Webhook(spec) => match execute_webhook(&client, spec).await {
                Ok(res) => println!(
                    "← cron[{idx}] fired webhook → status={} body={}",
                    res.status, res.body
                ),
                Err(e) => println!("← cron[{idx}] webhook error: {e}"),
            },
            Action::Cron(_) => {
                println!("← cron[{idx}] nested cron actions are not fired by this demo");
            }
        }
    }

    let received = captured.lock().unwrap();
    println!("✓ mock receiver got {} request(s):", received.len());
    for body in received.iter() {
        println!("    {body}");
    }

    Ok(())
}

async fn receive(State(captured): State<Captured>, Json(body): Json<Value>) -> &'static str {
    captured.lock().unwrap().push(body);
    "{\"ok\":true}"
}
