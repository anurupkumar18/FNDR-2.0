## Handoff: T-402 embedding contract asymmetry + matryoshka rule (2026-09-05)

Done: Implemented the app-side, model-independent half of T-402's contract
in `crates/fndr-inference/src/embedding.rs`:

- `query_embedding_text(query)` / `QUERY_INSTRUCTION`: the Qwen3-Embedding
  instruction-prefix convention for query-side text ("Instruct: {task}\n
  Query: {query}"), so documents and queries are never composed the same
  way (ADR-006's index/query asymmetry requirement).
- `Embedder::embed_query` (new default trait method): wraps the query text
  with the instruction prefix and calls through to `embed_documents`, so
  every future concrete `Embedder` gets the asymmetry correctly for free
  without having to remember to apply it. `embed_documents` itself is
  untouched — it still receives raw, unprefixed text.
- `truncate_and_renormalize(vector, target_dim)`: the matryoshka rule
  (truncate the native embedding to its leading `target_dim` dims, then
  L2-renormalize), a pure function independent of the model backing it.

Also, as groundwork: downloaded the real pinned
`Qwen3-Embedding-0.6B-Q8_0.gguf` (639,150,592 bytes) into `models/`
(repo-root, gitignored) using the actual production
`fndr_downloader::download_verified` function — the first time that
function has run against a real Hugging Face artifact rather than a test
fixture; its SHA-256 matched the registry's pinned value both via the
function's own internal check and my independent `shasum -a 256`. Kept the
fetch utility as a small, generalized, reusable dev-bootstrap example
(`crates/fndr-downloader/examples/fetch_model.rs`, `cargo run -p
fndr-downloader --example fetch_model -- [model-id]`) rather than deleting
it as scratch, since any future session needs this exact step to do real
GGUF work — it also idempotently no-ops when the file is already present
and verified.

Ran and confirmed:
- `cargo fmt --all --check` — clean
- `cargo test -p fndr-inference -p fndr-store -p fndr-downloader` — 7 + 15
  (10+3+2) + existing fndr-downloader tests all pass, including the 4 new
  tests (truncate/renormalize round-trip, zero-vector edge case, prefix
  asymmetry, `embed_query` default-impl behavior via a test-only recording
  `Embedder`)
- `cargo clippy -p fndr-inference -p fndr-store -p fndr-downloader
  --all-targets -- -D warnings` — clean
- `git diff --check` — clean
- `make test` (full sweep) — see result inline in this session; if this
  entry predates that result being pasted in, re-run it before trusting
  this line
- Manually exercised `fetch_model` twice: first run genuinely downloaded
  and verified 639MB from huggingface.co; second run correctly detected
  the file already present and verified without re-downloading

In flight / explicitly not done: the actual concrete GGUF-backed
`Embedder` implementation (loading the model via a llama.cpp binding,
running real inference, returning real 1024d vectors before truncation).
No llama.cpp crate is pinned anywhere in the workspace yet — this is
listed in `docs/journal` as the next step, not attempted this session,
because it means adding a new heavy native-FFI dependency (build-time
compiler/toolchain requirements, likely Metal linkage on macOS) that
needs its own `cargo deny` check, CI-budget note, and careful verification
against the pinned crate's actual source per the repo's own lesson about
rc-line crates drifting between versions — not something to rush blind.
`docs/ROADMAP-TICKETS.md`'s T-402 row was left as "Partial" (not
promoted to "Done") and updated to name specifically what's done now.

Decisions:
- Made `embed_query` a default trait method rather than requiring every
  future `Embedder` impl to remember to apply the prefix itself — this
  makes the asymmetry structurally hard to get wrong, matching the "no
  silent degradation" spirit of catching classes of mistakes at the type
  level rather than by convention.
- Left `truncate_and_renormalize`'s zero-vector case as a silent no-op
  (returns the zero vector unchanged) rather than an error, because the
  existing writer-side non-zero probe (already implemented, per this
  crate's own doc comments and `fndr-store`'s wrong-dimension refusal) is
  the intended place to reject it — avoiding a second, possibly
  inconsistent check.
- Kept the model fetch as a real, generalized, kept-in-repo example
  instead of one-off throwaway scratch, since it is genuinely reusable
  dev-environment tooling in the same spirit as `fndr-mcp/examples/
  skeleton.rs`.

Landmines:
- `models/` at the repo root is already gitignored (confirmed in
  `.gitignore` before downloading anything) — the 639MB GGUF file must
  never be committed; double-check `git status` doesn't show it before
  any future commit in this area.
- Compiling anything that pulls in `fndr-store` (and therefore
  `lancedb`/`lance`/`datafusion`) took noticeably longer this run than
  earlier in the session (several minutes with the parent `cargo test`
  process showing near-zero CPU for a while before child rustc processes
  became visible) — not clearly a hang, just slow parallel codegen; don't
  assume a stuck process too early if this recurs.

Produced by: Anurup + Claude Code
