# Handoff: T-109 walking skeleton (2026-08-20)

Done: the deliberately ugly end-to-end slice exists and is tested headless. A PNG frame (file source or a one-shot `screencapture` shell-out) goes through the ported v1 Vision OCR wrapper, into a minimal SQLite store with an FTS5 index, and out through `fndr.search` served over rmcp streamable HTTP with auth enforced from the first commit (bearer token, constant-time compare, Host and Origin allowlists, a global rate limit). `cargo run -p fndr-mcp --example skeleton` runs it and prints the `claude mcp add` line. The named regression tests `mcp_rejects_unauthenticated_loopback` and `mcp_rejects_web_origin_with_valid_token` exercise the real network boundary with raw HTTP; `capture_ocr_store_search_round_trip` proves the whole pipe on the rendered fixture image.

Findings (T-109 AC):

1. MCP tool names may contain dots per the current tool-name SEP (verified in rmcp's validator), so ADR-007's `fndr.` namespace holds as specified. No amendment needed.
2. rmcp 3.1 ships its own Host and Origin allowlists in `StreamableHttpServerConfig` (defaults: loopback hosts, no origins). Our auth layer sits in front; keep both (defense in depth), and revisit when T-701 builds the real surface.
3. specta-typescript 0.0.12 forbids i64/u64/i128/u128 by default; the ADR-001 IPC integer convention is the exporter default, and `dangerously_cast_bigints_to_number` is the named escape we ban.
4. rusqlite `bundled` includes FTS5; external-content tables with sync triggers work as expected, including delete sync.
5. Vision OCR runs headless on image bytes with no TCC involvement, so real-OCR tests run in CI. fndr-ocr must link `framework=Vision` explicitly; v1 only worked because the app shell happened to load it.
6. The v1 OCR wrapper mixes in text cleanup that ARCHITECTURE assigns to fndr-textsignal; split it when T-305 finalizes (noted at the top of the ported file).
7. tauri pulls reqwest only for iOS/Android targets; deny.toml scopes the graph to Apple desktop targets so the egress bans stay meaningful.
8. objc2 stays on the 0.5 line (what the ported wrapper was written against); the 0.6 upgrade is a scheduled maintenance PR together with the T-305 cleanup.

In flight: nothing broken. Next moves per E01/E02: T-201 real schema, T-208 Lance spike, T-301 textsignal port; live `screencapture` capture needs a human to grant Screen Recording once (the typed error path is tested).

Decisions: skeleton pieces live in their real crates (no throwaway crate); the runner is an example under fndr-mcp so the crate map stays clean. Store is in-memory in the example; persistence lands with T-201.

Landmines: `#[tool_router(server_handler)]` emits the ServerHandler; adding prompts or resources later means switching to explicit `#[tool_handler]`. The rate limit is global per process, deliberately crude. The edition 2024 build needed explicit unsafe blocks in three spots of the ported wrapper (logic unchanged).

Produced by: Anurup + Claude Code
