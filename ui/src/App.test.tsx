import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { App } from "./App";

test("renders the app shell with the search screen visible", () => {
  render(<App />);
  expect(screen.getByRole("navigation", { name: "FNDR" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Search" })).toBeInTheDocument();
});
