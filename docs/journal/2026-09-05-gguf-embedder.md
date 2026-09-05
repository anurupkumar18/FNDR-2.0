## Handoff: T-402 concrete GGUF embedder (2026-09-05)

Done: `GgufEmbedder` (`crates/fndr-inference/src/gguf_embedder.rs`), a real
`Embedder` implementation backed by llama.cpp via the `llama-cpp-2` crate
(pinned `=0.1.156`, exact version verified against the real published
source in `~/.cargo/registry`, not from memory, per the repo's own lesson
about rc/pre-1.0 crates drifting between versions). It loads the pinned
Qwen3-Embedding-0.6B-Q8_0 GGUF (from `models/`, downloaded earlier this
session via the real `download_verified` path), runs inference with
`LlamaPoolingType::Last` (Qwen3-Embedding's documented pooling method),
and feeds the raw native vector through the `truncate_and_renormalize`
matryoshka function landed in the previous commit.

This is genuinely real, not decorative: I ran the `#[ignore]`d
construction-probe test against the actual downloaded model and it
produced a real 768-dim, unit-norm, non-zero embedding via Metal on this
machine, and confirmed a query and a document embed to different vectors
(index/query asymmetry actually exercised end to end, not just unit-tested
in isolation).

**Critical CI-safety decision:** the construction-probe test is marked
`#[ignore = "needs the real GGUF model downloaded to models/ ...; never
runs in CI"]`. The model file is correctly gitignored and will never exist
on a fresh CI checkout, so an un-ignored version of this test would fail
on every single future PR, forever. Run it explicitly with `cargo test -p
fndr-inference -- --ignored` after `cargo run -p fndr-downloader --example
fetch_model`. This is the single most important thing for a future session
to not accidentally "fix" by removing the `#[ignore]`.

**Metal note:** `llama-cpp-2`'s own (published, normalized) Cargo.toml has
a `target.'cfg(macos + aarch64/arm64)'` override that force-enables its
`metal` feature via `llama-cpp-sys-2` on Apple Silicon macOS regardless of
any feature flags we set — meaning Metal is already compiled in on this
machine and (if `.github/workflows/ci.yml`'s `rust` job's `macos-14`
runner is Apple Silicon) will be compiled in on CI too. I still added an
explicit, off-by-default `metal` feature on `fndr-inference` itself
(forwarding to `llama-cpp-2/metal`) as a documented manual toggle, but it
is not load-bearing for this to work on Apple Silicon. Compilation
succeeding does not guarantee the CI *runner* can actually execute Metal
shaders (ADR-006 notes hosted runners lack real GPU/Metal for
latency/RAM numbers) — but since the real-inference test is `#[ignore]`d,
this never gets exercised in CI regardless, so it's moot for correctness,
only relevant if someone later un-ignores something.

**cargo-deny:** ran `cargo deny check` (installed cargo-deny locally
first, it wasn't present). Result: `bans ok, licenses ok, sources ok` for
the new dependency tree (`llama-cpp-2`, `llama-cpp-sys-2`, `bindgen`,
`clang-sys`, `cmake`, `cexpr`, `enumflags2(+derive)`, `find_cuda_helper`,
`itertools`, `libloading`, `minimal-lexical`, `nom`, `shlex` — all
mainstream MIT/Apache-2.0 crates, no HTTP client leaked in, verified via
`cargo tree -i <banned-crate>` for each of reqwest/ureq/curl/isahc/
attohttpc/surf returning no match under `fndr-inference`). `advisories
FAILED`, but on a pre-existing, unrelated issue: a yanked `chacha20`
pulled in by `rand v0.10.2` via `rmcp` (fndr-mcp) and `twox-hash`
(lance-core) — confirmed via `git diff Cargo.lock` that these crates'
pinned versions are byte-identical to what was already committed before
any of this session's changes, so this is not something my dependency
addition caused. Flagged separately via `spawn_task` (task_2b06824b) for
someone to fix in its own PR rather than folding it into this one.

Ran and confirmed:
- `cargo fmt --all --check` — clean
- `cargo test -p fndr-inference` — 7 passed, 1 ignored (the real-model
  test), as intended
- `cargo test -p fndr-inference -- --ignored` — the real-model
  construction probe passes against the actual downloaded model
- `cargo clippy -p fndr-inference --all-targets -- -D warnings` — clean
- `cargo clippy --workspace --all-targets -- -D warnings` — clean (all
  crates, including the new native dependency, compile and pass lints)
- `cargo deny check` — bans/licenses/sources ok for the new dependency;
  pre-existing unrelated advisories failure flagged separately
- `git diff --check` — clean
- `make test` (full sweep, first uncached run with llama-cpp-2 in the
  tree) — all green: workspace/UI lints, AGENTS.md drift check, fmt,
  workspace clippy, every Rust unit/integration/doc test across the
  workspace, UI `tsc --noEmit`, and Vitest. `git status` confirmed
  `models/` stayed untracked throughout.

In flight / explicitly not done:
- `GgufEmbedder` is not yet wired into anything (not the Lance writer, not
  a CLI, not the model-worker priority queue from T-403). It exists and is
  proven correct in isolation; wiring it into the real capture-to-store
  pipeline is a separate, larger slice (needs T-403's model-worker queue
  per the "no direct session use, no per-call construction" invariant,
  and T-404's batch path for real throughput — this implementation embeds
  one text at a time, which is correct but not the final performance
  shape).
- No throughput/latency numbers recorded (that's T-404's AC, not this
  slice's).
- CI budget impact of this dependency is unverified from here (no GitHub
  Actions access this session) — the local from-scratch compile of the
  full lance+llama.cpp stack under clippy took ~3 minutes on this machine;
  expect the first CI run after this lands to be noticeably slower until
  its cache warms, per the repo's own dependency-budget lesson.

Decisions:
- One text embedded per `LlamaContext`/`LlamaBatch` at a time, not batched
  — simpler and much easier to get correct on a first pass; batching is
  T-404's explicit job, not duplicated here.
- Fresh `LlamaContext` per `embed_documents` call rather than storing one
  on `GgufEmbedder`, because `LlamaContext<'a>` borrows the model and
  isn't obviously safe to share across the `Send + Sync` bound `Embedder`
  requires without real thought about llama.cpp's own thread-safety
  contract — deferred to whoever wires this into the real
  concurrency-bearing model-worker queue (T-403), which is the actual
  place that decision belongs.
- Treated a same-text query/document check (`assert_ne!`) as sufficient
  proof of asymmetry for this slice, rather than any claim about
  retrieval quality — that claim requires FNDR-Bench per ADR-006 and is
  explicitly out of scope here.

Landmines:
- If a future session considers deleting the `#[ignore]` on
  `construction_probe_dimension_and_non_zero`: don't, unless CI itself
  starts provisioning the model file somehow. See the CI-safety note
  above.
- `cargo deny` was not installed on this machine before this session;
  future sessions may need to reinstall it (`cargo install cargo-deny
  --locked`) if it's not already present.
- The pre-existing yanked `chacha20` cargo-deny failure is real and
  unrelated to this work; don't assume `cargo deny check` passing bans/
  licenses/sources but failing advisories means something in *this*
  commit broke — check `git diff Cargo.lock` first, as done here.

Produced by: Anurup + Claude Code
