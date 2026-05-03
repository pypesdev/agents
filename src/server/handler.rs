pub mod agents {
    use crate::agent::agent::Agent;
    use crate::server::requests;
    use crate::server::responses;
    use crate::server::server::AppState;
    use axum::{
        extract::{Query, State},
        response::IntoResponse,
        Json,
    };
    use serde::Deserialize;

    pub async fn agents_index(
        _pagination: Option<Query<requests::Pagination>>,
        State(state): State<AppState>,
    ) -> impl IntoResponse {
        let db = state.db.read().unwrap();
        let mut agents: Vec<Agent> = Vec::new();
        for agent_iter in db.get_all() {
            if let Some(curr_agent) = db.get::<Agent>(&agent_iter) {
                agents.push(curr_agent);
            } else {
                println!("Attempted to access invalid agent {}", agent_iter);
            }
        }
        Json(agents)
    }

    #[derive(Debug, Deserialize)]
    pub struct CreateAgent {
        name: String,
        inputs: Vec<serde_json::Value>,
        actions: Vec<String>,
    }

    pub async fn agents_create(
        State(state): State<AppState>,
        Json(input): Json<CreateAgent>,
    ) -> impl IntoResponse {
        let agent = Agent {
            name: input.name,
            inputs: input.inputs,
            actions: input.actions,
        };
        state.db.write().unwrap().set(&agent.name, &agent).unwrap();
        // The scheduler runs from a snapshot of the agents store; signal it
        // to rebuild so any cron actions on the new agent become live.
        state.scheduler.reload();
        let response = responses::CreateAgentResponse { records_created: 1 };
        Json(response)
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::agent::Agent;
    use crate::scheduler_loop::SchedulerHandle;
    use crate::server::server::AppState;

    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt; // for `collect`
    use pickledb::{PickleDb, PickleDbDumpPolicy, SerializationMethod};
    use serde_json::{json, Value};
    use std::sync::{Arc, RwLock};
    use tower::ServiceExt;
    #[tokio::test]
    async fn test_agents_index() {
        let mut db = PickleDb::new(
            "~/.agents/db/test.db",
            PickleDbDumpPolicy::NeverDump,
            SerializationMethod::Json,
        );

        let bob = &Agent {
            name: "Bob".to_string(),
            inputs: vec![],
            actions: vec![],
        };
        db.set("agent_name", bob).unwrap();

        let request = Request::builder()
            .uri("/agents")
            .body(Body::empty())
            .unwrap();
        let state = AppState {
            db: Arc::new(RwLock::new(db)),
            scheduler: SchedulerHandle::detached(),
        };

        let app = crate::server::server::app().with_state(state);
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(body, json!([{"name": "Bob", "actions": [], "inputs": []}]))
    }
}
