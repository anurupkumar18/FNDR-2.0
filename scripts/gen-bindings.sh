#!/usr/bin/env bash
# T-105: regenerate ui/src/bindings/bindings.ts from the Rust command surface.
# The fndr-shell bindings_in_sync test fails CI when this was not run.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo run -p fndr-shell --bin gen_bindings
