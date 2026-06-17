[![pr_check](https://github.com/pypesdev/agents/actions/workflows/pr-check.yaml/badge.svg)](https://github.com/pypesdev/agents/actions/workflows/pr-check.yaml)

> **Status (May 2026):** Active. Part of the [HiringFunnel](https://app.gethiringfunnel.com) open-source stack. See also [coldflow](https://github.com/pypesdev/coldflow) and [foxyapply](https://github.com/pypesdev/foxyapply).

<br>

### ![The Pypes Agent Project](documentation/readme-assets/banner.png)

<br>

<br>

<img src="documentation/readme-assets/ProductivityTools.png" alt="The ai agent platform for everyone" />

<br>

The Pypes Project is a fast and lightweight tool for defining autonomous AI agents that perceive (JSON inputs streamed in over an HTTP API) and act (named actions you wire up).

It ships as a single Rust binary — a CLI plus a daemonized HTTP server backed by an embedded JSON store.

<br>

<img src="documentation/readme-assets/Features.png" alt="Just let the features speak for themselves." height=35px>

- Stream perception into agents — every input is just JSON.
- Persist agents and their inputs to disk via embedded [pickledb](https://crates.io/crates/pickledb) — no external database to run.
- HTTP API (`axum`) for creating and listing agents from any language.
- Optional Qdrant vector-db resource pulled via Docker for embeddings work.
- Single static binary — `cargo build --release` produces one artifact.

<br>

<p align="center">
    <img src="documentation/readme-assets/GetStartedBelow.png" alt="Ready to turbocharge your productivity? Then let's get started!" height="100px" />
</p>

<br>

<br>

### ![Quick Installation](documentation/readme-assets/QuickInstallation.png)

### <a href="https://github.com/pypesdev/agents/blob/main/install.sh"><img src="documentation/readme-assets/AllExceptWindows.png" alt="All Except Windows" height=25px /></a>
```bash
curl -sSL https://github.com/pypesdev/agents/raw/main/install.sh | sh
```

Or build from source:

```bash
git clone https://github.com/pypesdev/agents.git
cd agents
cargo build --release
./target/release/pypes --help
```

<br>

## Quickstart

The smallest end-to-end loop — create an agent, feed it a JSON input, list it back. State is persisted under `~/.agents/db/`.

```bash
# 1. Create a named agent
pypes add agent echo

# 2. Stream a JSON input into it
pypes agent echo add '{"event":"hello","payload":"world"}'

# 3. List agents
pypes ls
# => Agent echo
```

Prefer HTTP? Start the server attached to your terminal and POST agent definitions directly:

```bash
pypes start --attatch -p 7979

# In another shell:
curl -X POST http://localhost:7979/agents \
  -H 'Content-Type: application/json' \
  -d @examples/echo.json

curl http://localhost:7979/agents
# => [{"name":"echo","inputs":[{"event":"hello","payload":"world"}],"actions":[]}]
```

See [`examples/`](./examples) for ready-to-POST agent definitions.

<br>

## Examples

| File | What it does |
|---|---|
| [`examples/echo.json`](./examples/echo.json) | Minimal hello-world agent — one JSON input, no actions. |
| [`examples/web-summarize.json`](./examples/web-summarize.json) | Illustrative URL-fetch + summarize agent shape. Action wiring is intentionally a no-op until those executors land — see the integration table below. |
| [`examples/webhook.json`](./examples/webhook.json) | Agent with a single `webhook` action; pair with `pypes agent webhook-demo run` to fire it. |
| [`examples/webhook_executor.rs`](./examples/webhook_executor.rs) | End-to-end Rust example: spins up a local mock receiver, dispatches a webhook action, and prints the captured request. Run with `cargo run --example webhook_executor`. |
| [`examples/cron_executor.rs`](./examples/cron_executor.rs) | End-to-end Rust example: schedules a webhook to fire on the next per-second tick via the in-process scheduler, then exits. Run with `cargo run --example cron_executor`. |

<br>

## Integrations

Honest status as of **May 2026**. Anything marked planned has a target release; anything else is fiction we are not shipping yet.

| Integration | Status | Notes |
|---|---|---|
| HTTP / JSON inputs | :heavy_check_mark: Shipped | `POST /agents`, `GET /agents`, CLI `agent <name> add`. Backed by pickledb on disk. |
| Qdrant vector-db | :heavy_check_mark: Shipped | `pypes add vector-db` pulls `qdrant/qdrant` via the local Docker socket. |
| Daemonized server | :heavy_check_mark: Shipped | `pypes start` forks; `pypes start --attatch` runs in the foreground. |
| Action executors → webhook (HTTP POST) | :heavy_check_mark: Shipped | First concrete executor. Each `Agent.actions` entry is a JSON spec like `{"type":"webhook", ...}`; see [Action Executors → Webhook](#action-executors--webhook). |
| Action executors → cron (scheduled) | :heavy_check_mark: Shipped | Wraps another action with a 5-, 6-, or 7-field cron expression and fires it on schedule via an in-process scheduler. See [Action Executors → Cron](#action-executors--cron). |
| Action executors → LLM | :calendar: Planned | Same `{"type": "..."}` discriminator as webhook/cron. |
| Gmail | :calendar: Planned for v0.1.0 | Not implemented. Earlier README claimed "in progress" — that was stale; no module exists in `src/`. |
| SMS (Twilio) | :calendar: Planned for v0.1.0 | Not implemented. |
| Vision / image inputs | :calendar: Planned for v0.2.0 | Not implemented. JSON-only inputs today. |
| Web UI | :hammer_and_wrench: Experimental | `src/server/templates/` ships layout/index stubs; no routes mount them yet. |

Legend: :heavy_check_mark: shipped • :hammer_and_wrench: experimental • :calendar: planned.

<br>

## Action Executors → Webhook

Agents can now act, not just store. Each entry in `Agent.actions` is a
JSON-encoded spec; the executor dispatches by the `type` discriminator.

The first shipped executor is **`webhook`** — a single HTTP POST to a target
URL with a JSON payload and optional headers.

```jsonc
// One entry inside Agent.actions
{
  "type": "webhook",
  "url": "https://example.com/hook",
  "headers": { "Authorization": "Bearer demo-token" },
  "payload": { "event": "agent.acted", "n": 1 }
}
```

Run every action stored on an agent through the executor pipeline:

```bash
pypes agent <NAME> run
# [0] webhook → 200 (11 bytes)
```

A 2xx response counts as success; anything else surfaces as a `NonSuccessStatus`
error so the caller can decide whether to retry. `headers` and `payload` are
optional (payload defaults to `{}`).

The shared HTTP client used by `process_actions` carries a **30-second
timeout** (connection + read combined). If the target does not respond within
that window the executor returns `WebhookError::Timeout`, distinct from generic
transport errors so callers can apply different retry semantics.

### Worked example

```bash
cargo run --example webhook_executor
```

The example boots a tiny in-process axum receiver on an ephemeral port,
constructs an in-memory `Agent` with one webhook action pointed at it, and
prints both the executor outcome and the body the receiver got:

```
→ stored action: {"headers":{"Authorization":"Bearer demo-token"},"payload":{"event":"agent.acted","n":1},"type":"webhook","url":"http://127.0.0.1:49207/hook"}
← webhook[0] status=200 body={"ok":true}
✓ mock receiver got 1 request(s):
    {"event":"agent.acted","n":1}
```

No external services needed. Tests use [`wiremock`](https://crates.io/crates/wiremock)
to assert on the exact request the executor sends — see
[`src/executors/webhook.rs`](./src/executors/webhook.rs).

<br>

## Action Executors → Cron

Cron lets agents act on a schedule — required for any "check inbox every 10
minutes" or "run nightly summary" pattern. A `cron` action wraps another action
with a cron expression; an in-process scheduler fires the wrapped action on
each tick.

```jsonc
// One entry inside Agent.actions
{
  "type": "cron",
  "expression": "*/5 * * * *",
  "action": {
    "type": "webhook",
    "url": "https://example.com/tick",
    "payload": { "tick": true }
  }
}
```

The `expression` field accepts:

- **5-field standard cron** — `min hour day-of-month month day-of-week` (e.g.
  `*/5 * * * *` for every five minutes). `sec` is padded to `0` and `year` to `*`.
- **6-field cron with seconds** — `sec min hour dom mon dow` (e.g.
  `* * * * * *` for every second).
- **7-field cron with year** — passed through to the underlying parser.

`pypes agent <NAME> run` reports the next computed fire time for each cron
action without firing it; the actual firing happens automatically once the
server is running:

```bash
pypes agent scheduled-pinger run
# [0] cron `*/5 * * * *` → next fire 2026-05-03 12:05:00 UTC
```

`pypes start` (daemonized or `--attatch`) launches a background scheduler
loop that loads every cron action across all agents on boot, sleeps until
the next due tick, fires the wrapped action (currently `webhook`), and
advances. Each fire is logged to the daemon stderr at
`~/.agents/tmp/daemon.err`. Creating a new agent via `POST /agents`
triggers an in-process reload so newly stored cron actions become live
without restarting the daemon.

Out of scope for v1: distributed scheduling, persistent missed-fire catchup
across daemon restarts (the loop starts fresh from `Utc::now()` on boot),
and per-tenant isolation.

### Worked example

```bash
cargo run --example cron_executor
```

The example boots a tiny in-process axum receiver on an ephemeral port, builds
a `CronAction` whose target is a webhook pointed at the receiver, advances the
[`Scheduler`](./src/executors/cron.rs) by one real tick, and prints both the
fired outcome and the body the receiver got:

```
→ stored action: {"type":"cron","expression":"* * * * * *","action":{"type":"webhook","url":"http://127.0.0.1:51636/hook","headers":{},"payload":{"event":"cron.tick","n":1}}}
⏲ next fire scheduled at 2026-05-03 09:10:56 UTC (in 253 ms)
← cron[0] fired webhook → status=200 body={"ok":true}
✓ mock receiver got 1 request(s):
    {"event":"cron.tick","n":1}
```

The example terminates within ~1 second because the cron expression fires on
the next per-second boundary. Tests drive the scheduler with a fixed mock
clock and a tight real-time clock — see [`src/executors/cron.rs`](./src/executors/cron.rs).

<br>

## CLI reference

```
pypes start [--attatch] [-p PORT]   # start the HTTP server (port 7979 by default)
pypes stop                          # kill a daemonized server
pypes status                        # check whether the server is reachable
pypes add agent <NAME>              # create an empty agent
pypes add vector-db                 # pull qdrant/qdrant via Docker
pypes rm agent <NAME>               # remove a single agent
pypes rm db                         # wipe the agents database
pypes ls                            # list known agents
pypes agent <NAME> add '<JSON>'     # append a JSON input (or action spec) to an agent
pypes agent <NAME> run              # execute every stored action through the executor pipeline
```

<br>

## HTTP API

| Method | Path | Body | Response |
|---|---|---|---|
| `GET` | `/agents` | — | `Agent[]` |
| `POST` | `/agents` | `{name, inputs: any[], actions: string[]}` | `{records_created: number}` |

<br>

## Development

```bash
cargo build           # debug build
cargo test            # run the test suite (matches the PR-check workflow)
cargo build --release # production binary at target/release/pypes
```

PRs run `cargo test` via [`.github/workflows/pr-check.yaml`](.github/workflows/pr-check.yaml). Releases are cut by pushing a `vX.Y.Z` tag — see [`.github/workflows/release.yaml`](.github/workflows/release.yaml).

<br>

---
Maintained by [Pypes LLC](https://app.gethiringfunnel.com) under the HiringFunnel brand.
