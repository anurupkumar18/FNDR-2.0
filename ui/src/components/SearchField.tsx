import type { ChangeEvent } from "react";

export interface SearchFieldProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

export function SearchField({ value, onChange, placeholder = "Search your memory" }: SearchFieldProps) {
  function handleChange(event: ChangeEvent<HTMLInputElement>) {
    onChange(event.target.value);
  }

  return (
    <input
      type="search"
      className="search-field"
      value={value}
      onChange={handleChange}
      placeholder={placeholder}
      aria-label="Search your memory"
      autoFocus
    />
  );
}
