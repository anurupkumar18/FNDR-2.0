# Reviewer report: technology due diligence

Independent fresh-context agent, 2026-08-20. Brief: verify or refute the plan's eight load-bearing technology bets against primary sources (crate docs, changelogs, issues), assuming prior research may contain errors. Synthesis: [REVIEW-2026-08-20.md](REVIEW-2026-08-20.md).

## Bet 1: LanceDB Rust crate (native BM25 FTS, hybrid + RRF, pluggable reranking): CONFIRMED

- Crate current and active: lancedb 0.37.1 (2026-08-10); 0.29 to 0.37 shipped May to Aug 2026 (crates.io/crates/lancedb).
- Native BM25 from Rust: `Index::FTS` documented with Rust examples; full vector index menu (IvfPq, IvfHnswSq) and scalar indexes are Rust-native (docs.rs/lancedb).
- Hybrid from Rust: `Query::full_text_search` plus `nearest_to`; `VectorQuery::execute_hybrid`, `rerank`, `norm` exist.
- Pluggable reranking: `lancedb::rerankers::Reranker` trait; built-in `rrf::RRFReranker`.
- "Incremental index updates with flat-scan tail" confirmed (unindexed rows covered until `optimize`; `fast_search()` skips the tail).
- Gaps vs Python: hybrid docs show only Python/TS examples; built-in rerankers in Rust are RRF only. Not a problem: ADR-006 runs the reranker through llama.cpp, so only RRF plus the trait are needed. Expect to work from docs.rs and source; pin the crate exactly (fast cadence, no 1.0).

## Bet 2: llama-cpp-2 for Qwen3 embeddings, reranking, VL multimodal: (a) CONFIRMED, (b) CONFIRMED, (c) PARTLY

- Binding health: llama-cpp-2 0.1.154 (2026-08-05), weekly cadence, 1.08M downloads; pinned llama.cpp submodule (b10200, Aug 2026) is far past upstream Qwen3-VL support.
- (a) Embeddings: in-repo `embeddings` example; `LlamaPoolingType` exposes None/Mean/Cls/Last/Rank; PR #1087 (Aug 2026) fixed embedding reads by live pooling type. Instruction prefixes are plain prompt text.
- (b) Reranking: in-repo `reranker` example using rank pooling; upstream Qwen3-Reranker support merged in llama.cpp PR #15824 (Sept 2025). The app formats the Qwen3 rerank prompt itself; see Bet 8 for GGUF sourcing.
- (c) mtmd: real `mtmd` module behind a feature flag, marked "experimental and subject to breaking changes"; crates.io packaging failure (issue #828) fixed Feb 2026; example demonstrated with Gemma 3; no direct evidence of Qwen3-VL through this binding specifically. Official Qwen3-VL GGUFs exist; mtmd chunk evaluation is model-agnostic, so it should work, but it is unproven.
- Watch: open issue #1096, `apply_chat_template` silently produces wrong prompts for many models; build VLM prompts via mtmd markers, not the chat-template helper.
- Plan change: add a week-1 spike "Qwen3-VL-2B through llama-cpp-2 mtmd (published crate): image + prompt to grounded JSON" before committing the VLM synthesis design. VLM is optional by design, so failure degrades rather than blocks.

## Bet 3: tauri-specta / specta for Tauri 2 bindings: PARTLY

- Active: specta and tauri-specta 2.0.0-rc.25 (May 2026), recent human commits; no first-party alternative (tauri-bindgen dormant since Jan 2024).
- But: RC since ~2023 with a ~14-month release gap; "is rc production safe" issue (#247, Jul 2026) unanswered; docs.rs build for rc.25 failed; breaking changes land between RCs, so exact `=` pins are required.
- Known limitations: i64/u64 need explicit BigInt export configuration (churned; #481 open), `tauri::ipc::Request/Response` unsupported (#170), `emitTo` events unimplemented (#187).
- Plan change: keep the bet; pin exact `=rc` versions in month 1; define the i64/id convention inside `fndr-types` before the first IPC command; binding-generator upgrades are scheduled maintenance PRs, never drive-by bumps.

## Bet 4: screencapturekit crate for an always-on 0.5 FPS loop: PARTLY

- SCK-from-Rust is the right API bet (CGDisplayStream/CGWindowList deprecated since WWDC22). The specific crate is the risk: screencapturekit 8.0.1 (2026-07-18), one primary maintainer, three major versions in seven weeks (Jun 2026), and a closed-issue history of exactly the always-on failure class: #52 "tons of memory leaks" (closed not-planned), #73 callbacks stop after a while, #43 audio-buffer crashes, #127 macOS 26 weak-linking crash on macOS 15 (fixed Feb 2026).
- Production comparables avoided it: screenpipe uses its own pinned fork (sck-rs) plus cidre; Cap's scap is stalled.
- macOS 26.1 reportedly requires an app bundle for the Screen Recording privacy pane (field reports), fine for the bundled app but a `cargo run` dev-mode trap.
- Plan changes: (1) pin exactly and add a multi-day soak test (RSS trend assertion) in month 1; (2) for 0.5 FPS prefer periodic `SCScreenshotManager` captures over a persistent SCStream, sidestepping the callback-stall and buffer-lifetime failure classes; (3) name the fallback in ADR-001: objc2-screen-capture-kit (maintained inside the objc2 project) or vendoring, as both comparables did; (4) document the dev-mode TCC quirk in the platform runbook.

## Bet 5: real-model benchmarks on GitHub Actions macOS runners: PARTLY

- Runner reality: hosted arm64 macOS runners are a 3 vCPU M1 slice, 7 GB RAM, 14 GB disk; free on public repos, $0.062/min private, ~10x Linux.
- No Metal: GPU passthrough open since Feb 2023 (actions/runner-images#7085); llama.cpp's own CI sets `GGML_METAL=OFF` on GitHub runners.
- Cache: 10 GB per repo, 7-day eviction; caching a 639 MB GGUF is practical.
- Comparables: llama.cpp runs real-model suites on self-hosted runners and only a 270M smoke model on hosted macOS; fastembed runs small real ONNX models per PR with aggressive trimming.
- Estimate: embedding a few-thousand-chunk corpus with Q8 0.6B on 3 vCPUs is roughly 10 to 45 minutes.
- Plan change: restructure the lane. (1) Per-PR quality gate on free Linux runners (4 vCPU/16 GB public) with the cached GGUF and a small fixed corpus, Recall@5/MRR only; (2) nightly or label-triggered macOS parity lane; (3) all latency and RAM numbers come exclusively from the reproducible `make bench` on the reference M1 8 GB machine, never CI, because hosted runners have no Metal. Extend the PRD's reference-machine sentence to cover latency targets so nobody wires a latency assertion into CI.

## Bet 6: FluidAudio scope and licenses: CONFIRMED

- Inference-only, exactly as planned: the API takes files/buffers; the README points at external tooling for system audio, so the sidecar's own ScreenCaptureKit audio tap is required and correctly scoped (SCK audio needs macOS 13+; plan requires 14+).
- Licenses: SDK unmodified Apache 2.0; nvidia/parakeet-tdt-0.6b-v3 CC-BY-4.0 ungated; pyannote community-1 is CC-BY-4.0 with a gated official repo, but FluidAudio pulls the ungated FluidInference CoreML conversions, so no HF token at runtime. ADR-003's license row is accurate.
- Active: v0.15.6 (2026-08-19), macOS 14+/iOS 17+. ANE execution is a vendor claim, not independently verified.
- Additions: ship CC-BY-4.0 attribution notices for Parakeet and pyannote in the app's acknowledgements (a redistribution requirement); pin the FluidAudio version tag.

## Bet 7: SQLite + LanceDB dual-store in a desktop app: CONFIRMED

- The failure modes are real and documented: ~800 MB after 5,000 single-record inserts (lancedb#3086, closed Aug 2026); "Too many concurrent writers" from over-aggressive cleanup; compaction temporarily grows disk (budget ~2x live-index headroom). ADR-002's batched flush plus scheduled optimize/prune is the maintainer-recommended counter-pattern (optimize after ~100k rows or ~20 ops).
- Field precedents: Continue.dev ships embedded LanceDB (one 30 GB data-dir report); AnythingLLM ships it as desktop default (commit-conflict crash reports); screenpipe chose SQLite-only.
- Concurrency: in-process single-writer many-readers is the supported shape. The single Lance writer plus Tauri single-instance covers it; also make the single-instance lock guard the DB directory so a stray CLI (`fndr doctor`, rebuild) cannot open a second writer while the app runs.
- No sandbox/notarization blockers found (static link, no JIT); absence of evidence, not a guarantee.
- Churn caveat: underlying Lance moved orgs and is at v11.0.0-beta.x; file format 2.1 declared stable with compatibility commitments. Pin exactly; the rebuild-from-SQLite property is the insurance and must be CI-tested as ADR-002 already specifies.

## Bet 8: Qwen3-Embedding-0.6B (matryoshka, instructions, GGUF, llama.cpp): PARTLY

- MRL confirmed: user-defined output dims 32 to 1024; 768 valid; Apache 2.0.
- Instruction format confirmed: `Instruct: {task}\nQuery:{query}` for queries, documents bare; matches the plan's asymmetric contract. EOS is appended by GGUF tokenizer metadata; do not append manually.
- Official GGUF confirmed; llama.cpp support merged (PR #15023, Aug 2025) with last-token pooling from GGUF metadata.
- Wrong detail 1: ADR-003 lists "~400 MB Q4 (mmap)". The official GGUF ships only Q8_0 (639 MB) and f16 (1.2 GB). No official Q4 exists; given the GGUF embedding-quality issue (#14234, resolution unverified), community sub-8-bit quants are exactly what to avoid for the required always-on model.
- Wrong detail 2 (reranker row): "official conversion only" is half right. Community GGUFs missing `cls.output.weight` do produce near-zero scores, but there is NO official Qwen GGUF for Qwen3-Reranker-0.6B; the trustworthy artifact is ggml-org/Qwen3-Reranker-0.6B-Q8_0-GGUF (the llama.cpp org's conversion).
- Unstated burden: llama.cpp has no MRL dimensions parameter. The 768d contract means FNDR takes 1024d output, truncates to 768, and L2-renormalizes in `fndr-inference`. Also do not serve embedding and reranking from one context/instance (llama.cpp#20085: all-zero outputs).
- Plan changes: update the ADR-003 table (Q8_0 639 MB; ggml-org reranker source, SHA-256 pinned); write truncate+renormalize into the contract struct and tests; add a 768-vs-1024 ablation to the month-2 bench.

## Better options shipped recently that the plan missed

Nothing material on models (EmbeddingGemma worse on license and score; Granite R2 only if the 8 GB budget forces it). rmcp positively confirmed on the 2026-07-28 spec (4.7M+ downloads). sqlite-vec ANN still alpha, fallback calculus unchanged. The one genuinely better pattern missed is in capture: the production-proven approach is a vendored/pinned SCK binding or periodic SCScreenshotManager screenshots rather than a persistent SCStream from the crates.io crate.

## Three riskiest bets, ranked

1. **Bet 5 (real-model evals in CI).** P0.7 as written implies a lane hosted runners cannot honestly provide (no Metal, 3 vCPUs, 10x cost). If the CI lane is slow, flaky, or measures meaningless CPU latency, the team routes around it and the plan's central discipline dies quietly. Restructure now.
2. **Bet 4 (the screencapturekit crate).** The whole product sits on an always-on loop, and the chosen crate has the exact failure history that kills always-on apps, while both shipped comparables vendored their own bindings. Mitigation is cheap but must be scheduled in month 1.
3. **Bet 8 (embedding contract details).** The required model's ADR row cites a nonexistent Q4 artifact, 768d silently depends on app-side truncate+renormalize, an unresolved GGUF-quality issue argues Q8_0-only, and the reranker's "official" GGUF is a community conversion. None fatal, but ADR-006 forbids a dual-contract transition, so getting the contract wrong in month 1 is the most expensive small mistake available.
