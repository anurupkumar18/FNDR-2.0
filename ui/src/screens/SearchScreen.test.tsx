import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SearchScreen } from "./SearchScreen";
import { commands } from "../bindings/bindings";
import type { MemorySearchHit } from "../bindings/bindings";

vi.mock("../bindings/bindings", () => ({
  commands: {
    searchMemories: vi.fn(),
  },
}));

const searchMemories = vi.mocked(commands.searchMemories);

beforeEach(() => {
  vi.useFakeTimers();
  searchMemories.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

function typeQuery(user: ReturnType<typeof userEvent.setup>, text: string) {
  return user.type(screen.getByRole("searchbox"), text);
}

test("shows an idle hint before any query is typed", () => {
  render(<SearchScreen />);
  expect(screen.getByText(/Type to search/)).toBeInTheDocument();
});

test("debounces input, then shows real results from a matching query", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  const hit: MemorySearchHit = {
    record_id: "record-1",
    chunk_id: "chunk-1",
    app_name: "Terminal",
    captured_at_ms: 1_700_000_000_000,
    snippet: "error mismatched types in fusion.rs",
    route: "keyword",
  };
  searchMemories.mockResolvedValue({
    status: "ok",
    data: { query: "mismatched", hits: [hit], vault: "ready", vector_route: "available" },
  });

  render(<SearchScreen />);
  await typeQuery(user, "mismatched");

  expect(searchMemories).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(300);

  await waitFor(() => expect(screen.getByText(hit.snippet)).toBeInTheDocument());
  expect(searchMemories).toHaveBeenCalledWith("mismatched", 20);
});

test("tells the person plainly when nothing has ever been captured", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  searchMemories.mockResolvedValue({
    status: "ok",
    data: { query: "anything", hits: [], vault: "not_created", vector_route: "model_missing" },
  });

  render(<SearchScreen />);
  await typeQuery(user, "anything");
  await vi.advanceTimersByTimeAsync(300);

  await waitFor(() => expect(screen.getByText(/Nothing captured yet/)).toBeInTheDocument());
});

test("shows an honest empty state, and why semantic search didn't run, for a real query against an existing vault", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  searchMemories.mockResolvedValue({
    status: "ok",
    data: {
      query: "nothing matches this",
      hits: [],
      vault: "ready",
      vector_route: "model_missing",
    },
  });

  render(<SearchScreen />);
  await typeQuery(user, "nothing matches this");
  await vi.advanceTimersByTimeAsync(300);

  await waitFor(() =>
    expect(screen.getByText(/No results for "nothing matches this"/)).toBeInTheDocument(),
  );
  expect(screen.getByText(/no local model installed/)).toBeInTheDocument();
});

test("shows a visible error rather than silently failing", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  searchMemories.mockResolvedValue({ status: "error", error: "search_unavailable" });

  render(<SearchScreen />);
  await typeQuery(user, "anything");
  await vi.advanceTimersByTimeAsync(300);

  await waitFor(() =>
    expect(screen.getByText(/Search is unavailable/)).toBeInTheDocument(),
  );
});

test("clearing the search box invalidates an in-flight request instead of letting a late response overwrite the idle view", async () => {
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  let resolveSearch: ((value: Awaited<ReturnType<typeof commands.searchMemories>>) => void) | undefined;
  searchMemories.mockImplementation(
    () =>
      new Promise((resolve) => {
        resolveSearch = resolve;
      }),
  );

  render(<SearchScreen />);

  // Dispatch a request: type a query and let the debounce fire.
  await typeQuery(user, "abandoned");
  await vi.advanceTimersByTimeAsync(300);
  await waitFor(() => expect(searchMemories).toHaveBeenCalledWith("abandoned", 20));

  // Abandon it: clear the field before the response arrives.
  await user.clear(screen.getByRole("searchbox"));
  await vi.advanceTimersByTimeAsync(300);
  expect(screen.getByText(/Type to search/)).toBeInTheDocument();

  // The slow response for the abandoned query finally resolves.
  const staleHit: MemorySearchHit = {
    record_id: "record-stale",
    chunk_id: "chunk-stale",
    app_name: "Terminal",
    captured_at_ms: 1_700_000_000_000,
    snippet: "STALE RESULT FOR ABANDONED QUERY",
    route: "keyword",
  };
  resolveSearch?.({
    status: "ok",
    data: { query: "abandoned", hits: [staleHit], vault: "ready", vector_route: "available" },
  });

  // Give any pending microtasks/state updates a chance to run.
  await vi.advanceTimersByTimeAsync(0);

  expect(screen.getByText(/Type to search/)).toBeInTheDocument();
  expect(screen.queryByText(/STALE RESULT FOR ABANDONED QUERY/)).not.toBeInTheDocument();
});
