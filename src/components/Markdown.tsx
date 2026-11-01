import { memo, useContext, useMemo } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { RemoteImages } from "../lib/remoteImages";

marked.setOptions({ gfm: true, breaks: false });

const OPTIONS = { FORBID_TAGS: ["style", "form", "input"], FORBID_ATTR: ["style"] };

// Attributes that make the browser fetch the address they hold (on <a>,
// href is a link the user follows, which opens in their browser instead).
const FETCHING = ["src", "href", "xlink:href", "srcset", "poster", "background", "ping"];
// A url(...) reference to anything but an element in the same document.
const EXTERNAL_URL = /url\(\s*['"]?\s*(?!#)/i;

// An imported transcript comes from someone else, and anything it loads from
// the web would tell them when it was read. Export downloads linked images
// into the bundle (as data: URLs here), so every other address is dropped.
// Inline SVG stays, with only its references to the outside removed.
function dropRemoteImages(node: Element) {
  for (const { name, value } of [...node.attributes]) {
    const fetches = FETCHING.includes(name) && !(node.tagName.toLowerCase() === "a" && name === "href");
    const local = value.startsWith("#") || value.startsWith("data:");
    if ((fetches && !local) || EXTERNAL_URL.test(value)) node.removeAttribute(name);
  }
  if (node.tagName === "IMG" && !node.hasAttribute("src")) {
    node.setAttribute("alt", "remote image not included");
    node.classList.add("img-not-included");
  }
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

export const Markdown = memo(function Markdown({ text, className }: { text: string; className?: string }) {
  const remoteImages = useContext(RemoteImages);
  const html = useMemo(() => render(text, remoteImages), [text, remoteImages]);
  return <div className={`md ${className ?? ""}`} dangerouslySetInnerHTML={{ __html: html }} />;
});
