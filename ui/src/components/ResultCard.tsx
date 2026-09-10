import type { ReactNode } from "react";
import type { MemorySearchHit } from "../bindings/bindings";

export interface ResultCardProps {
  hit: MemorySearchHit;
}

function formatCapturedAt(capturedAtMs: number | null): string {
  if (capturedAtMs === null) {
    return "Unknown time";
  }
  return new Date(capturedAtMs).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

// The store's keyword-route snippets wrap each matched term in literal
// `[...]` (SQLite FTS5's `snippet()` markers, set in fndr-store::store.rs);
// vector-route snippets carry no marks at all, so a highlight is itself an
// honest route signal, same as the trust window's app.js::renderSnippet.
// Rendering `hit.snippet` raw would show the literal brackets to the person
// reading it, never highlight anything.
function renderSnippetParts(snippet: string): ReactNode[] {
  return snippet
    .split(/(\[[^\]]*\])/g)
    .filter((part) => part.length > 0)
    .map((part, index) =>
      part.startsWith("[") && part.endsWith("]") && part.length >= 2 ? (
        <mark key={index}>{part.slice(1, -1)}</mark>
      ) : (
        part
      ),
    );
}

export function ResultCard({ hit }: ResultCardProps) {
  return (
    <article className="result-card" data-route={hit.route}>
      <div className="result-card-header">
        <span className="result-card-app">{hit.app_name ?? "Unknown app"}</span>
        <time className="result-card-time">{formatCapturedAt(hit.captured_at_ms)}</time>
      </div>
      <p className="result-card-snippet">{renderSnippetParts(hit.snippet)}</p>
    </article>
  );
}
