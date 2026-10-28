import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App.tsx";
import { api } from "./api";

if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  await import("./devBridge");
}

window.addEventListener("error", (e) => api.log("error", `${e.message} @ ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => api.log("error", `unhandled rejection: ${String(e.reason)}`));

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
