//! End-to-end demo of the webhook action executor.
//!
//! Run with: `cargo run --example webhook_executor`
//!
//! Steps:
//! 1. Spin up a local axum mock receiver on an ephemeral port.
//! 2. Build an in-memory `Agent` whose `actions` contains a JSON-encoded
//!    webhook spec pointed at the mock.
//! 3. Process the agent's actions and print the executor outcome plus the
//!    payload the receiver actually got.
//!
//! No external services are required.

use std::sync::{Arc, Mutex};

use axum::{extract::State, routing::post, Json, Router};
use pypes::agent::agent::Agent;
use pypes::executors::{process_actions, ExecutionOutcome};
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

    let webhook_spec = json!({
        "type": "webhook",
        "url": format!("http://{}/hook", addr),
        "headers": { "Authorization": "Bearer demo-token" },
        "payload": { "event": "agent.acted", "n": 1 }
    })
    .to_string();

    let agent = Agent {
        name: "demo".to_string(),
        inputs: vec![],
        actions: vec![webhook_spec],
    };

    println!("→ stored action: {}", agent.actions[0]);

    let outcomes = process_actions(&agent).await;
    for (idx, outcome) in outcomes.iter().enumerate() {
        match outcome {
            ExecutionOutcome::Webhook(Ok(res)) => {
                println!("← webhook[{idx}] status={} body={}", res.status, res.body);
            }
            ExecutionOutcome::Webhook(Err(e)) => {
                println!("← webhook[{idx}] error: {e}");
            }
            ExecutionOutcome::Cron(Ok(s)) => {
                println!(
                    "← cron[{idx}] scheduled `{}` next at {}",
                    s.expression, s.next_fire
                );
            }
            ExecutionOutcome::Cron(Err(e)) => {
                println!("← cron[{idx}] error: {e}");
            }
            ExecutionOutcome::Unrecognized { raw, error } => {
                println!("← unrecognized[{idx}] {error}: {raw}");
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
