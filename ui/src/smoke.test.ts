// Bootstrap harness check: proves the vitest + tsc gate runs in CI before any
// real UI code exists. Replaced by real component tests starting with T-1001.
import { expect, test } from "vitest";

test("test harness runs", () => {
  expect(1 + 1).toBe(2);
});
