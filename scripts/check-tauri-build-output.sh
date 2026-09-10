#!/usr/bin/env bash
# T-1001: the workspace window's built HTML has two properties Tauri's
# custom asset protocol requires that no automated test caught until the app
# was actually launched (see ui/vite.config.ts's stripCrossorigin comment
# and lessons.md): no `crossorigin` attribute on any tag, and only relative
# asset paths. Both defects passed 20/20 vitest tests and a clean `vite
# build`/`vite preview` -- this script is the cheap, no-GUI gate that would
# have caught them.
set -euo pipefail
cd "$(dirname "$0")/.."

html="crates/fndr-shell/ui/workspace/index.html"
if [ ! -f "$html" ]; then
  echo "FAIL: $html not found. Run 'npm --prefix ui run build' first." >&2
  exit 1
fi

if grep -q "crossorigin" "$html"; then
  echo "FAIL: $html has a crossorigin attribute. Tauri's custom asset protocol" >&2
  echo "does not return Access-Control-Allow-Origin, so a crossorigin-tagged" >&2
  echo "module script silently fails to execute in the real app (page loads," >&2
  echo "CSS applies, <div id=\"root\"> stays empty forever, no visible error)." >&2
  echo "See ui/vite.config.ts's stripCrossorigin plugin." >&2
  exit 1
fi

if grep -oE '(src|href)="/[^"]*"' "$html"; then
  echo "FAIL: $html references an asset by an absolute path (leading '/')." >&2
  echo "This window's HTML is served from a subfolder (workspace/) of Tauri's" >&2
  echo "shared frontendDist root, not its root -- an absolute path resolves" >&2
  echo "against the wrong location and silently 404s. See ui/vite.config.ts's" >&2
  echo "base: \"./\" setting." >&2
  exit 1
fi

echo "tauri build output check: ok"
