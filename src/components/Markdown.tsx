import { memo, useMemo } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { openUrl } from "@tauri-apps/plugin-opener";

marked.setOptions({ gfm: true, breaks: false });

// Transcripts can contain arbitrary HTML (pasted pages, tool output echoed by
// the model), so everything is sanitized before it reaches the DOM.
function render(text: string): string {
  const html = marked.parse(text, { async: false }) as string;
  return DOMPurify.sanitize(html, { FORBID_TAGS: ["style", "form", "input"], FORBID_ATTR: ["style"] });
}

function onClick(e: React.MouseEvent) {
  const a = (e.target as HTMLElement).closest("a");
  if (!a) return;
  e.preventDefault();
  const href = a.getAttribute("href");
  if (href && /^https?:\/\//.test(href)) void openUrl(href);
}

export const Markdown = memo(function Markdown({ text, className }: { text: string; className?: string }) {
  const html = useMemo(() => render(text), [text]);
  return <div className={`md ${className ?? ""}`} onClick={onClick} dangerouslySetInnerHTML={{ __html: html }} />;
});
