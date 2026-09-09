import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { expect, test, vi } from "vitest";
import { SearchField } from "./SearchField";

function ControlledSearchField({ onChange }: { onChange: (value: string) => void }) {
  const [value, setValue] = useState("");
  function handleChange(newValue: string) {
    setValue(newValue);
    onChange(newValue);
  }
  return <SearchField value={value} onChange={handleChange} />;
}

test("renders the current value and reports changes", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<ControlledSearchField onChange={onChange} />);

  const input = screen.getByRole("searchbox", { name: "Search your memory" });
  await user.type(input, "rust");

  expect(onChange).toHaveBeenCalledTimes(4);
  expect(onChange).toHaveBeenLastCalledWith("rust");
});

test("reflects the value prop", () => {
  render(<SearchField value="preset query" onChange={() => {}} />);
  expect(screen.getByRole("searchbox")).toHaveValue("preset query");
});
