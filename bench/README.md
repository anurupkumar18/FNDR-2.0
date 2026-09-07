# Bench corpora and baselines

`corpus-sample/` is a tiny format fixture: it defines the corpus layout
(`records.jsonl` + `queries.jsonl`) and keeps `make bench` honest and fast in
CI. It is NOT an evaluation instrument; real corpora, the frozen held-out
split, and the donation protocol are E05 (T-501+). Never tune ranking against
this sample.

`baselines/` holds committed `BenchReport` JSON per corpus and route.
`make bench` compares quality metrics (Recall@5, MRR@10) against the matching
baseline and fails on regression; latency is recorded but never compared
(machine-dependent; published latency comes from the reference machine only,
PRD P0.7). When a change legitimately improves the numbers, rerun with
`--out` and commit the refreshed baseline in the same PR.

Corpus format, one JSON object per line:

```
records.jsonl  {"id": 1, "source": "screen", "captured_at_ms": 0, "text": "..."}
queries.jsonl  {"query": "...", "relevant": [1]}
```

## Real-model vector baseline

The vector route is opt-in so the normal fast FTS gate never loads a model.
Use a new empty directory; it receives a disposable SQLite database and Lance
derivative, not a user vault:

```sh
vector_work=$(mktemp -d /tmp/fndr-vector-bench.XXXXXX)
cargo run -p fndr-bench -- --route vector --corpus bench/corpus-sample \
  --model models/Qwen3-Embedding-0.6B-Q8_0.gguf --work-dir "$vector_work"
find "$vector_work" -type f -delete
find "$vector_work" -type d -empty -delete
```

On the local M1 reference machine on 2026-09-06, this synthetic fixture scored
Recall@5 1.0000, MRR@10 1.0000, p50 90.43 ms, and p95 90.62 ms. These are a
route smoke measurement, not a committed quality baseline or a ranking claim;
the sample corpus is too small for tuning.
