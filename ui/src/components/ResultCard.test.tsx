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
