import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../api";

/**
 * Opens every link in the default browser. Clicks, ⌘-clicks and middle
 * clicks on any <a> in the page are caught here; the backend also refuses to
 * let the window navigate to an outside page, as a backstop.
 */
export function openLinksInBrowser() {
  const onClick = (e: MouseEvent) => {
    if (e.button > 1) return;
    const a = (e.target as Element | null)?.closest?.("a[href]");
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute("href") ?? "";
    // Links in transcripts point outside the app; anything else (an in-page
    // anchor, a relative path from a README) has nowhere useful to go.
    if (!/^(https?:|mailto:)/i.test(href)) return;
    if ("__TAURI_INTERNALS__" in window) void openUrl(href).catch((err) => api.log("error", `open ${href}: ${err}`));
    else window.open(href, "_blank", "noopener");
  };
  document.addEventListener("click", onClick, true);
  document.addEventListener("auxclick", onClick, true);
}
