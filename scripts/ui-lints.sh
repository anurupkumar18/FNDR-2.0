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

# T-1406: crates/fndr-shell/ui/ (the trust window, Tauri's real frontendDist)
# gets the same "no raw hex in components" discipline T-1401 set for the
# other UI -- every color is a semantic role in tokens.css, never a literal
# in app.css/components.css/index.html. A hand-picked hex here is exactly
# how the old status-card/audit-card/truth-card treatments drifted apart.
color_literal_pattern='#[0-9a-fA-F]{3}([0-9a-fA-F]{3}([0-9a-fA-F]{2})?)?\b|rgba?\('
if rg -n -e "$color_literal_pattern" crates/fndr-shell/ui/app.css crates/fndr-shell/ui/components.css crates/fndr-shell/ui/index.html 2>/dev/null; then
  echo "FAIL: raw color literal outside crates/fndr-shell/ui/tokens.css. Add a token instead." >&2
  exit 1
fi

# T-1001: the new React app gets the same token discipline as the trust
# window -- every color a semantic role, never a literal in component CSS.
if rg -n -e "$color_literal_pattern" ui/src --glob '*.css' 2>/dev/null; then
  echo "FAIL: raw color literal in ui/src. Add a token to crates/fndr-shell/ui/tokens.css instead." >&2
  exit 1
fi

# T-1406: every button is one of the defined component variants, not an
# ad-hoc inline-styled control -- that is how three panels end up with three
# different-looking buttons.
if rg -n '<button(?![^>]*class="btn )' crates/fndr-shell/ui/index.html --pcre2 2>/dev/null; then
  echo "FAIL: a <button> in the trust window has no btn component class." >&2
  exit 1
fi

echo "ui lints: ok"
