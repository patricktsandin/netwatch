import { defineConfig } from "vite";

// In development, a dev daemon serves the API; Vite serves the UI and proxies to it.
// Not to the installed service on 8732: the UI can flush its history and rewrite its
// configuration. Start the dev daemon with NETWATCH_LISTEN=127.0.0.1:8733, or name
// another with NETWATCH_DEV_API.
const api = process.env["NETWATCH_DEV_API"] ?? "127.0.0.1:8733";

export default defineConfig({
  server: {
    proxy: {
      "/api/live": { target: `ws://${api}`, ws: true },
      "/api": `http://${api}`,
    },
  },
});
