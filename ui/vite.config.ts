import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The Tauri "workspace" window's frontendDist root is crates/fndr-shell/ui/
// (shared with the existing trust window); this app's build output lands in
// a subfolder of it, workspace/, referenced by that window's "url" in
// tauri.conf.json. Vite warns about writing outside its project root when
// emptyOutDir is set on such a path; that warning is expected here.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "../crates/fndr-shell/ui/workspace",
    emptyOutDir: true,
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./vitest.setup.ts"],
    globals: false,
  },
});
