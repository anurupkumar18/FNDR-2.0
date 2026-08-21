#!/usr/bin/env bash
# T-107: generate AGENTS.md from the fndr-v2-engineering skill so Cursor,
# Codex, and every other agent tool read the same conventions source.
# Deterministic concatenation: SKILL.md body (frontmatter stripped) followed
# by each reference file. Never edit AGENTS.md by hand; CI runs `--check`
# and fails on drift.
set -euo pipefail
cd "$(dirname "$0")/.."

SKILL=.claude/skills/fndr-v2-engineering
OUT=AGENTS.md

generate() {
  echo "<!-- GENERATED FILE, DO NOT EDIT. -->"
  echo "<!-- Source: $SKILL/  Regenerate: scripts/gen-agents-md.sh -->"
  echo "<!-- References below are inlined; a pointer to references/<name>.md resolves to the matching section. -->"
  awk 'BEGIN{fm=0} /^---$/{fm++; next} fm!=1' "$SKILL/SKILL.md"
  for ref in invariants lanes workflows ai-collaboration; do
    echo
    echo "---"
    echo
    echo "<!-- Inlined from $SKILL/references/$ref.md -->"
    echo
    cat "$SKILL/references/$ref.md"
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
