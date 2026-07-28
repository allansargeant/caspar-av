import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The console is served by `caspar-avd` from `web/`, so that is the build
// output. Relative asset paths keep it working behind a reverse proxy on a
// sub-path, which is how it ends up deployed on a show network.
export default defineConfig({
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
