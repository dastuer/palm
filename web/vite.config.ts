import { defineConfig } from "vite";

export default defineConfig({
  server: {
    proxy: {
      "/ws": { target: "ws://127.0.0.1:8787", ws: true },
      "/api": { target: "http://127.0.0.1:8787" },
    },
  },
  build: {
    target: "es2020",
    outDir: "dist",
    assetsInlineLimit: 0,
  },
});
