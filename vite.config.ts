import { readFileSync } from "node:fs";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const { version } = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // The browser preview's stand-in backend reports this as the app's version.
  define: { __APP_VERSION__: JSON.stringify(version) },
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: { target: "safari16", chunkSizeWarningLimit: 1500 },
});
