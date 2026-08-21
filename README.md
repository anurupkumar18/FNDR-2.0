# FNDR

Local-first screen-context memory engine for macOS. FNDR watches your screen (with consent and strict privacy gates), builds a searchable memory of what you worked on, and serves that memory to AI agents over MCP. Everything runs on-device; nothing derived from captured data ever leaves your machine.

Status: pre-alpha groundwork. The approved plan lives in `docs/` (PRD, ADRs under `docs/decisions/`, architecture, roadmap). Nothing user-facing is built yet.

## Layout

- `crates/`: Rust engine workspace. The engine is shell-agnostic; only `fndr-shell` may import Tauri (CI enforces this).
- `ui/`: React + TypeScript frontend. IPC types are generated from `fndr-types`; hand-written mirrors are banned.
- `docs/`: PRD, ADRs, architecture, roadmap, journal.
- `.claude/skills/fndr-v2-engineering/`: engineering conventions, the source of truth. `AGENTS.md` is generated from it; never edit `AGENTS.md` by hand.

## Development

```sh
make test
```

runs the full local gate (fmt, clippy, cargo test, tsc, vitest). See `CONTRIBUTING.md` for conventions.

## v1 reference

The v1 POC history is imported as the read-only `reference/v1` branch. Code moves from it only per the ADR-005 port policy: targeted functions, constants, prompts, or contracts, with tests and a `// Ported from FNDR v1 <path>` provenance note. Never develop on that branch, and never copy anything on the ADR-005 DISCARD list.

## License

Apache 2.0
