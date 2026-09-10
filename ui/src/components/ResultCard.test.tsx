import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { ResultCard } from "./ResultCard";
import type { MemorySearchHit } from "../bindings/bindings";

const hit: MemorySearchHit = {
  record_id: "record-1",
  chunk_id: "chunk-1",
  app_name: "Terminal",
  captured_at_ms: 1_700_000_000_000,
  snippet: "error mismatched types in fusion.rs",
  route: "keyword",
};

test("renders the hit's app name, snippet, and route", () => {
  render(<ResultCard hit={hit} />);
  expect(screen.getByText("Terminal")).toBeInTheDocument();
  expect(screen.getByText(hit.snippet)).toBeInTheDocument();
  expect(screen.getByRole("article")).toHaveAttribute("data-route", "keyword");
});

test("renders an honest fallback when the app name or timestamp is unknown", () => {
  render(<ResultCard hit={{ ...hit, app_name: null, captured_at_ms: null }} />);
  expect(screen.getByText("Unknown app")).toBeInTheDocument();
  expect(screen.getByText("Unknown time")).toBeInTheDocument();
});

test("highlights the real backend's [bracket] match markers instead of showing them literally", () => {
  // The store's FTS5 snippet() call wraps each matched term in literal
  // square brackets (fndr-store::store.rs) -- this is the real wire format,
  // not a stylistic choice made up for this test.
  const bracketedHit: MemorySearchHit = {
    ...hit,
    snippet: "the suspension [bridge] design",
  };
  render(<ResultCard hit={bracketedHit} />);

  const mark = screen.getByText("bridge");
  expect(mark.tagName).toBe("MARK");
  expect(screen.queryByText(/\[bridge\]/)).not.toBeInTheDocument();
  expect(screen.queryByText("[", { exact: false })).not.toBeInTheDocument();
});
