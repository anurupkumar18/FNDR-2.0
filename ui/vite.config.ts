import type { Plugin } from "vite";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri's custom asset protocol serves this build's script/link tags as a
// same-origin resource, but WKWebView still enforces a CORS-mode fetch for
// any tag carrying a `crossorigin` attribute -- Vite adds one by default to
// every module script and stylesheet link. Tauri's asset handler doesn't
// return Access-Control-Allow-Origin, so that CORS-mode fetch is silently
// discarded: the page loads, CSS applies, but the module script never runs
// and <div id="root"> stays empty forever, with no error visible anywhere
// (not in Rust-side logs, not reproducible via `vite preview` over real
// HTTP, where the same origin makes the same fetch mode succeed). Stripping
// the attribute from the built HTML is the fix; there is no Vite build
// option that suppresses emitting it in the first place.
//
// Scoped to <script>/<link> tag boundaries deliberately: this page has no
// inline <script> content today, but a naive global replace would silently
// mangle the word "crossorigin" inside one if it's ever added later.
function stripCrossorigin(): Plugin {
  return {
    name: "strip-crossorigin-for-tauri-asset-protocol",
    transformIndexHtml(html) {
      return html.replace(
        /<(script|link)\b[^>]*>/g,
        (tag) => tag.replace(/\s+crossorigin(="[^"]*")?/, ""),
      );
    },
  };
}

// The Tauri "workspace" window's frontendDist root is crates/fndr-shell/ui/
// (shared with the existing trust window); this app's build output lands in
// a subfolder of it, workspace/, referenced by that window's "url" in
// tauri.conf.json. Vite warns about writing outside its project root when
// emptyOutDir is set on such a path; that warning is expected here.
export default defineConfig({
  plugins: [react(), stripCrossorigin()],
  // Relative, not absolute: this window's HTML is served from a subfolder
  // (workspace/) of the shared frontendDist root, not its root. An absolute
  // base ("/assets/...") resolves against the webview's origin root and
  // silently 404s -- the page loads with an empty <div id="root">, no error
  // visible anywhere in Rust-side logs, only a blank window.
  base: "./",
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
