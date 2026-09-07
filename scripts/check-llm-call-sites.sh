#!/usr/bin/env bash
# T-403 review rule: every llama.cpp call goes through the model-worker
# queue (fndr_inference::ModelWorkerHandle), never a direct GgufEmbedder
# call from application code. Run manually before a PR that adds a new
# GgufEmbedder call site; not wired into CI yet (see the commit that
# added this script for why -- it needs review before becoming a gate
# that blocks every future PR).
set -euo pipefail
cd "$(dirname "$0")/.."

# GgufEmbedder's fields are private, so `GgufEmbedder::load(...)` is the
# only way to obtain a real instance -- that construction site is what
# must live only inside a ModelWorkerHandle loader closure (or
# fndr-inference's own tests/examples proving it works in isolation).
# Calling the Embedder trait's methods on an already-constructed instance
# is not checked here: that is what the worker thread itself must do,
# and what any TestEmbedder-style fake legitimately does in tests.
hits=$(grep -rn "GgufEmbedder::load(" \
  --include="*.rs" crates/ \
  | grep -v "crates/fndr-inference/src/" \
  | grep -v "/tests/" \
  | grep -v "/examples/" \
  || true)

if [ -n "$hits" ]; then
  echo "T-403 violation: GgufEmbedder constructed outside a ModelWorkerHandle loader:" >&2
  echo "$hits" >&2
  echo >&2
  echo "Route model loading through fndr_inference::ModelWorkerHandle::spawn instead." >&2
  exit 1
fi

echo "no-direct-llm-call check: ok"
