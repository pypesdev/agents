# Changelog

All notable changes to the Pypes agent platform are documented in this file.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it leaves 0.x.

## [Unreleased]

### Added
- `src/executors/` module with the first concrete action executor: `webhook` (HTTP POST). Each `Agent.actions` entry is now a JSON spec like `{"type":"webhook","url":...,"headers":{...},"payload":{...}}` — typed, dispatched, and returns a structured outcome.
- `pypes agent <NAME> run` CLI subcommand that runs every stored action through the executor pipeline.
- `examples/webhook_executor.rs` — end-to-end example with an in-process mock receiver. `cargo run --example webhook_executor`.
- `examples/webhook.json` — minimal agent definition with one webhook action.
- README section: **Action Executors → Webhook**.
- `wiremock` dev-dependency for mock-server-backed executor tests.
- `src/lib.rs` so the binary's modules are also reachable from `examples/` and downstream crates.
- Second action executor: `cron` (`src/executors/cron.rs`). A `cron` action wraps another `Action` with a 5-, 6-, or 7-field cron expression. Includes a pure `parse` / `next_fire_after` API and an in-process `Scheduler` with a `tick(now)` shape so unit tests can drive it with a mock clock and the real loop can drive it with `Utc::now()`.
- `examples/cron_executor.rs` — end-to-end example: schedules a webhook to fire on the next per-second tick and exits within ~1 second.
- README section: **Action Executors → Cron**.
- `cron` and `chrono` dependencies (chrono with `clock` + `serde` features).
- `src/scheduler_loop.rs` — long-running tokio task spawned by `pypes start` that loads every cron action across all agents on boot, fires due entries through the existing webhook executor, advances the scheduler, and logs each fire to the daemon's stderr (`~/.agents/tmp/daemon.err`). `POST /agents` now triggers an in-process reload so newly stored cron actions go live without restarting the daemon.

### Changed
- `Agent.actions` storage is unchanged on disk (`Vec<String>`), but each entry is now interpreted as a typed `Action` enum at execution time. Strings that don't parse fall through as `Unrecognized` rather than blocking the pipeline.
- `ExecutionOutcome` gains a `Cron(Result<CronScheduled, CronError>)` variant. `process_actions` reports the next computed fire time for cron entries without firing them; firing is handled by the scheduler.
- Integrations table: webhook + cron executors are now shipped; only LLM remains in the planned column.

## [v0.0.6] - 2026-05-03

### Added
- HTTP server migrated to **axum 0.7** with `GET /agents` and `POST /agents` routes (`src/server/`).
- PR-check GitHub Actions workflow that runs `cargo test` on every pull request.
- `Dockerfile` for containerized builds.
- `install.sh` one-liner installer pointed at `pypesdev/agents` releases.
- `examples/` directory with two ready-to-POST agent definitions: `echo.json` and `web-summarize.json`.
- README integration table now reflects the actual May 2026 status of each integration with target releases for what is planned.
- README Quickstart, CLI reference, and HTTP API tables.
- CI badge for the `pr_check` workflow at the top of the README.

### Changed
- Reorganized server code under `src/server/` (`server.rs`, `handler.rs`, `requests.rs`, `responses.rs`).
- README: updated install URL from prior owner to `pypesdev/agents`, added HiringFunnel status header, added Pypes LLC attribution footer.
- README: replaced unverifiable "in progress" claims with concrete shipped/experimental/planned status.

### Notes
- No new integration code shipped in this release. Gmail, SMS, and vision/image inputs remain unimplemented and are now labeled as planned with target releases.
- `Agent.actions` is persisted as `Vec<String>` but no executor dispatches them yet — labeled experimental.

### Commits since v0.0.5
- install.sh (06c6eb8)
- install instructions in readme (544d942)
- refactor server mod (93ccf84)
- all the clones (73aff67)
- wip ui (be10cce)
- add Dockerfile (88571ef)
- reorganize file structure (970a084)
- additional routes (0037b97)
- Merge pull request #5 from jaredzwick/additional-routes (1e936fb)
- axum working (7ef9ffe)
- cleanup handlers (3449949)
- add pr check for tests (f40eacd)
- Merge pull request #6 from jaredzwick/use-axum (df80190)
- fix workflow trigger (a9896f8)
- README: point install.sh URL to pypesdev/agents (603e99a)
- README: add HiringFunnel status header (49d672a)
- README: add Pypes LLC / HiringFunnel attribution footer (5e5ffd0)

## [v0.0.5] - 2023-11-09

Initial public release. See [the v0.0.5 GitHub release](https://github.com/pypesdev/agents/releases/tag/v0.0.5) for details.
