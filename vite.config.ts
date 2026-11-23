import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/** `sp serve`'s default port. */
const SERVER_PORT = 47800;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
    // The browser preview talks to a local `sp serve`. The server only answers its own Host and
    // Origin, so the proxy presents both as the server's.
    proxy: {
      "/api": {
        target: `http://127.0.0.1:${SERVER_PORT}`,
        changeOrigin: true,
        configure: (proxy) =>
          proxy.on("proxyReq", (req) => {
            if (req.getHeader("origin")) req.setHeader("origin", `http://127.0.0.1:${SERVER_PORT}`);
          }),
      },
    },
  },
  build: { target: "safari16", chunkSizeWarningLimit: 1500 },
});
