#!/usr/bin/env bash
# Runs the workspace dependency lints (T-103 egress, T-104 engine-no-tauri).
set -euo pipefail
cd "$(dirname "$0")/.."

# Prefer the system python: version-manager shims fail without a local
# .tool-versions, and this script must run on a bare checkout.
PY=python3
[ -x /usr/bin/python3 ] && PY=/usr/bin/python3

cargo metadata --format-version 1 | "$PY" scripts/workspace_lints.py
