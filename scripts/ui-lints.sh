#!/usr/bin/env bash
# T-105: hand-written IPC is banned. All invoke() calls live in the generated
# bindings; UI code imports `commands` from ui/src/bindings/ instead of
# reaching for @tauri-apps/api/core directly. This is how v1 accumulated
# drifting hand-mirrored types.
set -euo pipefail
cd "$(dirname "$0")/.."

if rg -n "@tauri-apps/api/core" ui/src --glob '!ui/src/bindings/**' 2>/dev/null; then
  echo "FAIL: raw invoke import outside ui/src/bindings/. Use the generated commands." >&2
  exit 1
fi
echo "ui lints: ok"
