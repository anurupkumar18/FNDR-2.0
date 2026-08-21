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
