import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

// `@testing-library/dom`'s `waitFor` only recognizes fake timers through a
// global `jest` object (it checks `typeof jest !== "undefined"` and then
// `setTimeout._isMockFunction || setTimeout.clock`). Vitest's fake timers
// (backed by @sinonjs/fake-timers) do set `setTimeout.clock`, but Vitest
// never defines a `jest` global -- so without this shim, `waitFor` can't
// tell fake timers are active, falls back to real-timer polling, and hangs
// until the suite's outer timeout under any test using
// `vi.useFakeTimers()` together with `waitFor`. This shim is what
// testing-library's own docs recommend for Vitest.
declare global {
  // eslint-disable-next-line no-var -- required to shape a global shim
  var jest: { advanceTimersByTime: typeof vi.advanceTimersByTime } | undefined;
}
globalThis.jest = { advanceTimersByTime: vi.advanceTimersByTime };

afterEach(() => {
  cleanup();
});
