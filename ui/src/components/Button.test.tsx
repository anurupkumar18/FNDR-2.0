import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { Button } from "./Button";

test("renders a primary button with its btn classes", () => {
  render(<Button variant="primary">Start capture</Button>);
  const button = screen.getByRole("button", { name: "Start capture" });
  expect(button).toHaveClass("btn", "btn-primary");
});

test("defaults to the secondary variant", () => {
  render(<Button>Cancel</Button>);
  expect(screen.getByRole("button", { name: "Cancel" })).toHaveClass("btn-secondary");
});

test("forwards the disabled prop", () => {
  render(<Button disabled>Cancel</Button>);
  expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
});
