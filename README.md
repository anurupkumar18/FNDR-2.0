# FNDR

Local-first screen-context memory engine for macOS. FNDR watches your screen (with consent and strict privacy gates), builds a searchable memory of what you worked on, and serves that memory to AI agents over MCP. Everything runs on-device; nothing derived from captured data ever leaves your machine.

Status: pre-alpha, M1 foundations in progress. The approved plan lives in `docs/` (PRD, ADRs under `docs/decisions/`, architecture, roadmap with a progress ledger). Working today: the walking skeleton (one frame captured, OCRed with Apple Vision, stored, and served to agents over authenticated MCP; see Try it below), schema v1 with migrations, the ported v1 perception heuristics, and a real `make bench` retrieval gate with a committed baseline.

## Layout

- `crates/`: Rust engine workspace. The engine is shell-agnostic; only `fndr-shell` may import Tauri (CI enforces this).
- `ui/`: React + TypeScript frontend. IPC types are generated from `fndr-types`; hand-written mirrors are banned.
- `docs/`: PRD, ADRs, architecture, roadmap, journal.
- `.claude/skills/fndr-v2-engineering/`: engineering conventions, the source of truth. `AGENTS.md` is generated from it; never edit `AGENTS.md` by hand.

## Development

```sh
scripts/dev-setup.sh   # bare checkout to green gate (installs pinned Rust)
make test              # full local gate: lints, fmt, clippy, cargo test, tsc, vitest
make bench             # retrieval metrics vs the committed baseline
```

See `CONTRIBUTING.md` for conventions.

## Try it

The walking skeleton captures one frame, OCRs it, stores it, and serves it to
AI agents over authenticated MCP:

```sh
cargo run -p fndr-mcp --example skeleton
```

It prints the `claude mcp add` line to connect Claude Code. Pass
`--image <png>` to run from a screenshot file without any permissions, or
`--query <text>` for a one-shot search instead of serving.

## v1 reference

The v1 POC history is imported as the read-only `reference/v1` branch. Code moves from it only per the ADR-005 port policy: targeted functions, constants, prompts, or contracts, with tests and a `// Ported from FNDR v1 <path>` provenance note. Never develop on that branch, and never copy anything on the ADR-005 DISCARD list.

## License

Apache 2.0
