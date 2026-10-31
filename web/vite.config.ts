import { defineConfig } from "vite";

// In development, the daemon serves the API; Vite serves the UI and proxies to it.
export default defineConfig({
  server: {
    proxy: {
      "/api/live": { target: "ws://127.0.0.1:8732", ws: true },
      "/api": "http://127.0.0.1:8732",
    },
  },
});
