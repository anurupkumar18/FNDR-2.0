import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { Panel } from "./Panel";

test("renders the eyebrow, title, and children inside a panel", () => {
  render(
    <Panel eyebrow="MEMORY" title="Search">
      <p>content</p>
    </Panel>,
  );
  expect(screen.getByText("MEMORY")).toHaveClass("eyebrow");
  expect(screen.getByRole("heading", { name: "Search" })).toBeInTheDocument();
  expect(screen.getByText("content")).toBeInTheDocument();
});

test("renders without a header when no eyebrow or title is given", () => {
  render(<Panel><p>content only</p></Panel>);
  expect(screen.queryByRole("heading")).not.toBeInTheDocument();
});
