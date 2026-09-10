import { render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { App } from "./App";

vi.mock("../src/bindings/bindings", () => ({
  commands: { searchMemories: vi.fn() },
}));

test("renders the sidebar and the search screen", () => {
  render(<App />);
  expect(screen.getByRole("navigation", { name: "FNDR" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Search" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("heading", { name: "Search" })).toBeInTheDocument();
  expect(screen.getByRole("searchbox")).toBeInTheDocument();
});
