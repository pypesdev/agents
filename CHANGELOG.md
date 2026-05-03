# Changelog

All notable changes to the Pypes agent platform are documented in this file.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it leaves 0.x.

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
