# ADR-003: On-device inference: two-runtime strategy and model lineup

**Status:** Proposed
**Date:** 2026-08-19
**Deciders:** FNDR v2 team (4)

## Context

All inference over captured data must run on-device (ADR-004). The system needs: text embeddings (always-on), a reranker and optional VLM/LLM (on-demand), OCR (continuous), image embeddings (optional), and meeting ASR plus diarization (session-scoped). Target machines include 8 GB Apple Silicon Macs; the always-on budget is a few hundred MB with larger models loaded on demand. The POC mixed ort (ONNX) for embeddings/CLIP, llama-cpp-2 for the VLM, Apple Vision for OCR, and Python sidecars for speech, serialized by a single mutex that different subsystems held inconsistently.

Research findings that shape the decision: no general LLM runtime (llama.cpp, MLX, candle, mistral.rs) can use the Apple Neural Engine; Core ML is the only door to the ANE (roughly 2 W vs roughly 20 W GPU), which matters for always-on and session-scoped encoders. MLX now beats llama.cpp on decode for many models but loses on prefill-heavy short jobs, has Swift-first bindings, and Ollama routes GGUF to llama.cpp; llama.cpp keeps day-1 support for new architectures, mmap load/unload, the broadest quantization menu, and mature Rust bindings. ONNX Runtime's CoreML EP cannot guarantee ANE placement and degrades with graph partitioning.

## Decision

**Two runtimes plus OS frameworks, deliberately:**

1. **Apple-native perception:** Vision framework for OCR (RecognizeTextRequest now; evaluate RecognizeDocumentsRequest structured output on macOS 26 for tables/lists), called from Rust via objc2 as the POC proved. Meeting audio via the **Swift sidecar** running FluidAudio (Parakeet TDT ASR plus pyannote community-1 diarization on the ANE via Core ML). SpeechAnalyzer is an optional transcription fallback where available.
2. **llama.cpp (GGUF) as the single general runtime** behind `llama-cpp-2`, serving text embeddings, reranking, VLM synthesis, and the optional Q&A LLM from one interface, with mmap-based load and on-demand unload.

**One model-worker queue** owns all llama.cpp work with priorities (interactive query > capture-time synthesis > background review > backfill), replacing the POC's `model_pipeline_lock` mutex that capture, review, and daily batches held inconsistently. Core ML/ANE work (audio) runs in the sidecar and does not contend.

**Model lineup** (all pinned by revision and SHA-256, required vs optional gated per POC ADR-012 semantics):

| Role | Model | License | Quantized size | Residency |
|---|---|---|---|---|
| Text embedding (required) | Qwen3-Embedding-0.6B, matryoshka output, **768d chosen contract** (truncate 1024d output to 768 and L2-renormalize app-side; llama.cpp has no dimensions parameter) | Apache 2.0 | ~640 MB Q8_0 (mmap; the official GGUF ships Q8_0 and f16 only, no Q4; avoid community sub-8-bit quants for the required model) | Always-on (evictable) |
| Reranker (P1, eval-gated) | Qwen3-Reranker-0.6B via the ggml-org GGUF conversion, SHA-256 pinned (no official Qwen GGUF exists; community conversions missing cls.output.weight score near zero) | Apache 2.0 | ~640 MB Q8 | On-demand |
| VLM synthesis (optional) | Qwen3-VL-4B Instruct; **2B on 8 GB machines** | Apache 2.0 | ~3.3 GB / ~1.9 GB | On-demand, idle-unload |
| Q&A LLM (optional) | Qwen3-4B Instruct; Apple Foundation Models as optional zero-RAM backend where available | Apache 2.0 / OS | ~2.5 GB Q4 | On-demand |
| Image embedding (P1) | SigLIP 2 base (Core ML fp16 or GGUF path as available) | Apache 2.0 | ~750 MB | On-demand |
| Meeting ASR + diarization | Parakeet TDT 0.6B v3 + pyannote community-1 via FluidAudio | CC-BY-4.0 weights, SDK Apache 2.0 | ~1 to 1.5 GB | Session-scoped |
| OCR | Apple Vision | OS | system | Continuous |

License exclusions recorded: Apple MobileCLIP2/FastVLM (research-only license), jina-clip-v2 (CC-BY-NC), EmbeddingGemma (Gemma terms, not Apache-redistributable), Moondream 3 (BUSL, needs review). MiniLM is retired: mid-50s MTEB vs Qwen3-Embedding's leaderboard-class quality at similar always-on cost.

## Options considered

**A (chosen): Apple frameworks + llama.cpp.** Minimum viable runtime count; every heavyweight model behind one Rust interface; ANE reserved for what only it does well.

**B: Standardize on ONNX Runtime (POC's embedding path).** One runtime for embeddings only; no VLM/LLM/reranker story without adding llama.cpp anyway; CoreML EP unreliable for ANE. Rejected as primary; ort remains acceptable for a specific encoder if a GGUF conversion is missing.

**C: Standardize on MLX.** Best raw Apple Silicon performance trajectory, but Swift-first bindings against a Rust engine, weaker prefill for short jobs, and no ANE either. Held as an upgrade path (an MLX sidecar can slot behind the same model-worker queue) rather than the foundation.

**D: Everything in the Swift sidecar (Core ML).** Maximizes ANE but forces every model through Core ML conversion, loses day-1 GGUF model availability, and moves the inference hot path across a process boundary. Rejected.

## Trade-off analysis

The two-runtime mix accepts one extra moving part (the Swift sidecar) to get the only two things a single runtime cannot provide together: day-1 open-model support with mmap economics (llama.cpp) and low-power ANE execution for continuous and session-scoped perception (Core ML via Apple frameworks and FluidAudio). Standardizing on either one alone forfeits the other's strength.

## Consequences

- Easier: swapping models within a role (contract system per POC ADR-002/010 carries forward); resource budgeting (one queue, one residency policy); meeting quality (diarization the POC never hardened).
- Harder: the sidecar protocol (JSON over stdio, supervised restart) must be built; two toolchains in CI.
- Revisit: MLX adoption when Rust bindings mature or if an M5-class GPU-neural-accelerator advantage becomes decisive; Foundation Models as default Q&A backend as OS coverage grows.

## Action items

1. [ ] Inference crate: model registry (pinned URLs, SHA-256, required/optional), model-worker queue with priorities, load/unload with idle timers (month 1).
2. [ ] Embedding contract v1: Qwen3-Embedding-0.6B Q8_0, 768d via app-side truncate-and-renormalize inside the contract struct with tests, document/query instruction format captured, EOS appended by tokenizer metadata (never manually).
3. [ ] Platform lane: FluidAudio sidecar spike with supervised lifecycle (month 3 to 4, ahead of meetings).
4. [ ] Reranker ablation on FNDR-Bench before promoting it into the default pipeline; embedding and reranking never served from the same llama.cpp context (upstream all-zero-output defect).

## Amendment (2026-08-20, plan review)

Due diligence corrected two table details (no official Q4 embedder GGUF exists, use Q8_0 at 639 MB; the reranker's trustworthy artifact is the ggml-org conversion) and surfaced the app-side truncate-and-renormalize burden for the 768d contract, now in action item 2 and T-402. A 768-vs-1024 quality ablation runs in month 2 (the truncation is app code, so it must be measured). Qwen3-VL through the llama-cpp-2 mtmd feature is plausible but unproven; the month-1 spike (T-408) converts it into a scheduled decision, and VLM prompts are built via mtmd markers, not the chat-template helper (known-broken upstream). CC-BY-4.0 attribution for Parakeet and pyannote ships in the app's acknowledgements (T-1206).
