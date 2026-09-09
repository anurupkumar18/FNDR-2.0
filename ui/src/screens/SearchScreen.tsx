import { useEffect, useRef, useState } from "react";
import { commands } from "../bindings/bindings";
import type { MemorySearchHit, VectorRouteState } from "../bindings/bindings";
import { Panel } from "../components/Panel";
import { SearchField } from "../components/SearchField";
import { ResultCard } from "../components/ResultCard";

const DEBOUNCE_MS = 300;
const RESULT_LIMIT = 20;

type SearchState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "no_vault" }
  | { status: "empty"; vectorRoute: VectorRouteState }
  | { status: "results"; hits: MemorySearchHit[]; vectorRoute: VectorRouteState }
  | { status: "error"; message: string };

/// Honest, human copy for why the semantic route didn't contribute this time.
/// `null` means it did (or the note would be redundant clutter) -- invariant
/// 4 asks for the reason to be visible, not for a badge to appear on every
/// successful search.
function vectorRouteNote(vectorRoute: VectorRouteState): string | null {
  switch (vectorRoute) {
    case "available":
      return null;
    case "model_missing":
      return "Keyword search only — no local model installed.";
    case "index_missing":
      return "Keyword search only — nothing indexed for semantic search yet.";
    case "failed":
      return "Semantic search failed this time — showing keyword results only.";
  }
}

export function SearchScreen() {
  const [query, setQuery] = useState("");
  const [state, setState] = useState<SearchState>({ status: "idle" });
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const requestIdRef = useRef(0);

  useEffect(() => {
    if (timeoutRef.current !== undefined) {
      clearTimeout(timeoutRef.current);
    }

    // Bump the request id on every effect run, not just when a new request
    // is about to dispatch. This invalidates any still-in-flight request
    // from a previous (now-abandoned) query even when this run ends up
    // going idle instead of dispatching -- otherwise a slow response for a
    // cleared query can arrive after the id was last bumped and silently
    // overwrite the idle view.
    const requestId = ++requestIdRef.current;

    const trimmed = query.trim();
    if (trimmed.length === 0) {
      setState({ status: "idle" });
      return;
    }

    timeoutRef.current = setTimeout(() => {
      if (requestId !== requestIdRef.current) {
        return;
      }
      setState({ status: "loading" });
      commands
        .searchMemories(trimmed, RESULT_LIMIT)
        .then((result) => {
          if (requestId !== requestIdRef.current) {
            return;
          }
          if (result.status === "error") {
            setState({ status: "error", message: result.error });
            return;
          }
          const { hits, vault, vector_route } = result.data;
          if (vault === "not_created") {
            setState({ status: "no_vault" });
            return;
          }
          setState(
            hits.length === 0
              ? { status: "empty", vectorRoute: vector_route }
              : { status: "results", hits, vectorRoute: vector_route },
          );
        })
        .catch(() => {
          if (requestId === requestIdRef.current) {
            setState({ status: "error", message: "search_unavailable" });
          }
        });
    }, DEBOUNCE_MS);

    return () => {
      if (timeoutRef.current !== undefined) {
        clearTimeout(timeoutRef.current);
      }
    };
  }, [query]);

  return (
    <Panel eyebrow="MEMORY" title="Search">
      <SearchField value={query} onChange={setQuery} />
      {state.status === "idle" && (
        <p className="search-hint">Type to search everything FNDR has remembered.</p>
      )}
      {state.status === "loading" && <p className="search-hint">Searching…</p>}
      {state.status === "no_vault" && (
        <p className="search-hint">Nothing captured yet — there&apos;s no local memory to search.</p>
      )}
      {state.status === "empty" && (
        <>
          <p className="search-hint">No results for &quot;{query.trim()}&quot;.</p>
          {vectorRouteNote(state.vectorRoute) && (
            <p className="search-hint search-hint-muted">{vectorRouteNote(state.vectorRoute)}</p>
          )}
        </>
      )}
      {state.status === "error" && (
        <p className="search-hint search-hint-error">
          Search is unavailable right now ({state.message}).
        </p>
      )}
      {state.status === "results" && (
        <>
          {vectorRouteNote(state.vectorRoute) && (
            <p className="search-hint search-hint-muted">{vectorRouteNote(state.vectorRoute)}</p>
          )}
          <ul className="result-list">
            {state.hits.map((hit) => (
              <li key={hit.chunk_id}>
                <ResultCard hit={hit} />
              </li>
            ))}
          </ul>
        </>
      )}
    </Panel>
  );
}
