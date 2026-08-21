#!/usr/bin/env bash
# T-107: generate AGENTS.md from the fndr-v2-engineering skill so Cursor,
# Codex, and every other agent tool read the same conventions source.
# Deterministic concatenation: SKILL.md body (frontmatter stripped) followed
# by each reference file. Never edit AGENTS.md by hand; CI runs `--check`
# and fails on drift.
set -euo pipefail
cd "$(dirname "$0")/.."

SKILL=.claude/skills/fndr-v2-engineering
FEATURE=.claude/skills/fndr-feature-dev
OUT=AGENTS.md

strip_frontmatter() {
  awk 'BEGIN{fm=0} /^---$/{fm++; next} fm!=1' "$1"
}

generate() {
  echo "<!-- GENERATED FILE, DO NOT EDIT. -->"
  echo "<!-- Sources: $SKILL/ and $FEATURE/  Regenerate: scripts/gen-agents-md.sh -->"
  echo "<!-- References below are inlined; a pointer to references/<name>.md resolves to the matching section. -->"
  strip_frontmatter "$SKILL/SKILL.md"
  # lessons.md is inlined so every tool and teammate inherits the learning
  # loop automatically; the drift check is the sync mechanism.
  for ref in invariants lanes workflows ai-collaboration lessons; do
    echo
    echo "<!-- Inlined from $SKILL/references/$ref.md -->"
    echo
    cat "$SKILL/references/$ref.md"
  done
  echo
  echo "<!-- Inlined from $FEATURE/SKILL.md -->"
  echo
  strip_frontmatter "$FEATURE/SKILL.md"
  for ref in right-first-try feature-planning; do
    echo
    echo "<!-- Inlined from $FEATURE/references/$ref.md -->"
    echo
    cat "$FEATURE/references/$ref.md"
  done
}

if [ "${1:-}" = "--check" ]; then
  if ! generate | diff -u "$OUT" - >/dev/null 2>&1; then
    echo "AGENTS.md is out of sync with $SKILL." >&2
    echo "Run scripts/gen-agents-md.sh and commit the result." >&2
    exit 1
  fi
  echo "AGENTS.md is in sync."
else
  generate > "$OUT"
  echo "Wrote $OUT."
fi
