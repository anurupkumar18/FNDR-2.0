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

export function ResultCard({ hit }: ResultCardProps) {
  return (
    <article className="result-card" data-route={hit.route}>
      <div className="result-card-header">
        <span className="result-card-app">{hit.app_name ?? "Unknown app"}</span>
        <time className="result-card-time">{formatCapturedAt(hit.captured_at_ms)}</time>
      </div>
      <p className="result-card-snippet">{hit.snippet}</p>
    </article>
  );
}
