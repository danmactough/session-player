import { createContext, memo, useContext, useMemo } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";

marked.setOptions({ gfm: true, breaks: false });

const OPTIONS = { FORBID_TAGS: ["style", "form", "input"], FORBID_ATTR: ["style"] };

// An imported transcript comes from someone else, and an image it loads from
// the web would tell them when it was read. Export downloads linked images
// into the bundle (as data: URLs here), so anything still remote is dropped.
function dropRemoteImages(node: Element) {
  for (const attr of ["srcset", "poster", "background"]) node.removeAttribute(attr);
  if (node.tagName !== "IMG" || node.getAttribute("src")?.startsWith("data:")) return;
  node.removeAttribute("src");
  node.setAttribute("alt", "remote image not included");
  node.classList.add("img-not-included");
}

// Transcripts can contain arbitrary HTML (pasted pages, tool output echoed by
// the model), so everything is sanitized before it reaches the DOM.
function render(text: string, remoteImages: boolean): string {
  const html = marked.parse(text, { async: false }) as string;
  if (remoteImages) return DOMPurify.sanitize(html, OPTIONS);
  DOMPurify.addHook("afterSanitizeAttributes", dropRemoteImages);
  try {
    return DOMPurify.sanitize(html, OPTIONS);
  } finally {
    DOMPurify.removeHook("afterSanitizeAttributes");
  }
}

/** False inside an imported session: only images carried in its bundle are shown. */
export const RemoteImages = createContext(true);

export const Markdown = memo(function Markdown({ text, className }: { text: string; className?: string }) {
  const remoteImages = useContext(RemoteImages);
  const html = useMemo(() => render(text, remoteImages), [text, remoteImages]);
  return <div className={`md ${className ?? ""}`} dangerouslySetInnerHTML={{ __html: html }} />;
});
