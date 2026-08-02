import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

import { readFileSync } from 'node:fs';

// The workspace Cargo.toml, not console/package.json: caspar-av is a Rust
// product whose release tag follows the crate version, and the console's own
// package.json has been left behind by past releases (0.1.0 at the v0.1.1 tag).
// A wrong version in the About dialog is the one thing it must not have.
const cargo = readFileSync(new URL('../Cargo.toml', import.meta.url), 'utf8');
const version = /^version\s*=\s*"([^"]+)"/m.exec(cargo)?.[1] ?? '';

// The console is served by `caspar-avd` from `web/`, so that is the build
// output. Relative asset paths keep it working behind a reverse proxy on a
// sub-path, which is how it ends up deployed on a show network.
export default defineConfig({
  // The About dialog shows the version the build actually produced. about-data.js
  // carries one baked at sync time as a fallback, and it goes stale the moment a
  // release is tagged; this is the one that is always right.
  define: { __APP_VERSION__: JSON.stringify(`v${version}`) },
  plugins: [react()],
  base: "./",
  build: { outDir: "../web", emptyOutDir: true },
  server: {
    // `npm run dev` proxies to a locally running daemon so the UI can be
    // iterated on against real state.
    proxy: {
      "/api": "http://127.0.0.1:8080",
      "/ws": { target: "ws://127.0.0.1:8080", ws: true },
    },
  },
});
