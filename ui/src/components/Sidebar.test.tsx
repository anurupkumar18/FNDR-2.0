import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { Sidebar } from "./Sidebar";

test("marks Search as the current, enabled page", () => {
  render(<Sidebar active="search" />);
  const search = screen.getByRole("button", { name: "Search" });
  expect(search).toBeEnabled();
  expect(search).toHaveAttribute("aria-current", "page");
});

test("disables the not-yet-built screens with a Soon badge", () => {
  render(<Sidebar active="search" />);
  const vault = screen.getByRole("button", { name: /Memory Vault/ });
  expect(vault).toBeDisabled();
  expect(vault).toHaveTextContent("Soon");
});
