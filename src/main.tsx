import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App.tsx";
import { api } from "./api";
import { applyTheme, storedTheme } from "./lib/theme";
import { openLinksInBrowser } from "./lib/links";

applyTheme(storedTheme());
openLinksInBrowser();

if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  await import("./devBridge");
}

window.addEventListener("error", (e) => {
  // The virtualizer's ResizeObserver can trip this benign browser warning.
  if (e.message.startsWith("ResizeObserver loop")) return;
  api.log("error", `${e.message} @ ${e.filename}:${e.lineno}`);
});
window.addEventListener("unhandledrejection", (e) => api.log("error", `unhandled rejection: ${String(e.reason)}`));

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
