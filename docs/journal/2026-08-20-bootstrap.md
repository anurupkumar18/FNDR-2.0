# Handoff: repo bootstrap (2026-08-20)

Done: FNDR-2.0 created at github.com/anurupkumar18/FNDR-2.0 (public, Apache 2.0). Approved plan imported to `docs/` with paths rewritten from the v1 repo's `docs/v2/`. Cargo workspace with 16 empty crates builds, lints, and tests clean; `ui/` tsc plus vitest harness green. Engineering skill installed at `.claude/skills/fndr-v2-engineering/`; AGENTS.md generated from it with a CI drift check. CI green on main (Rust on macOS, UI, guards, cargo-deny). Workspace lints negative-tested: reqwest added to fndr-store and tauri added to fndr-types both fail the lint as designed, then were reverted. v1 history imported as read-only `reference/v1`; v1 release tags deliberately not pushed.

In flight: T-108 covers four machines; this machine went bare-to-green today and `scripts/dev-setup.sh` is the scripted path for the other three. Next moves: T-109 walking skeleton plus the M1 spikes (T-208, T-310, T-408, T-906). The GitLab CSV import of `docs/tickets.csv` is a manual owner step.

Decisions: workspace version `2.0.0-dev`, edition 2024, toolchain pinned to 1.98.0 in rust-toolchain.toml. `make bench` fails loudly (exit 2) until fndr-bench exists so nothing mistakes it for a measured pass. fndr-shell is a plain lib crate until T-901 brings Tauri in.

Landmines: the owner's personal Claude Code guard hook resolves the current branch from the session's working directory, so a session opened in the old FNDR directory misjudges FNDR-2.0 pushes; open implementation sessions inside FNDR-2.0. `ui/` has no React or Vite yet by design (arrives with T-1001); do not "fix" that in passing.

Produced by: Anurup + Claude Code
