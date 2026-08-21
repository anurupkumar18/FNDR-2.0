// T-105 round-trip proof: the generated bindings are importable, typecheck
// under strict tsc, and expose the sample command. Calling commands needs a
// running Tauri shell, so this only exercises the module surface.
import { expect, test } from "vitest";

import { commands, type EngineInfo } from "./bindings/bindings";

test("generated EngineInfo type is consumable", () => {
  const info: EngineInfo = { app_version: "2.0.0-dev" };
  expect(info.app_version).toBe("2.0.0-dev");
});

test("generated commands surface exposes engineInfo", () => {
  expect(typeof commands.engineInfo).toBe("function");
});
