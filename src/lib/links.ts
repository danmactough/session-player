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
  showLinkTargets();
}

/**
 * Shows where a link goes while the pointer is on it (or it has keyboard
 * focus), like a browser's status bar: a small label at the bottom left of
 * the pane holding the link, present only while there is a link to show.
 */
function showLinkTargets() {
  const bar = document.createElement("div");
  bar.className = "link-status";
  bar.setAttribute("aria-hidden", "true");
  document.body.append(bar);

  let current: Element | null = null;
  const show = (target: EventTarget | null) => {
    const a = (target as Element | null)?.closest?.("a[href]") ?? null;
    if (a === current) return;
    current = a;
    const href = a?.getAttribute("href") ?? "";
    if (!a || !/^(https?:|mailto:)/i.test(href)) {
      bar.classList.remove("on");
      return;
    }
    let text = href;
    try {
      text = decodeURI(href);
    } catch {
      // A malformed escape; show the link as written.
    }
    bar.textContent = text.replace(/^mailto:/i, "");
    const pane = (a.closest("main, aside, nav, [role=dialog]") ?? document.body).getBoundingClientRect();
    bar.style.left = `${pane.left}px`;
    bar.style.bottom = `${window.innerHeight - pane.bottom}px`;
    bar.style.maxWidth = `${Math.max(pane.width - 24, 120)}px`;
    bar.classList.add("on");
  };
  document.addEventListener("mousemove", (e) => show(e.target));
  document.addEventListener("focusin", (e) => show(e.target));
  document.addEventListener("focusout", () => show(null));
  // Scrolling slides a different link (or none) under the pointer, so drop
  // the label until the pointer moves again.
  document.addEventListener("scroll", () => show(null), true);
  document.documentElement.addEventListener("mouseleave", () => show(null));
}
