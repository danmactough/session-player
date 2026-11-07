import type { ReactNode } from "react";
import { Menu } from "./Menu";

interface Props {
  children: ReactNode;
  onClick: () => void;
  /** Opens the prompt editor before running. */
  onCustomize: () => void;
  disabled?: boolean;
  className?: string;
}

/** A primary button that runs with the saved prompt, and a chevron beside it
 * for running once with an edited one. */
export function SplitButton({ children, onClick, onCustomize, disabled, className = "" }: Props) {
  return (
    <div className={`split-button ${disabled ? "disabled" : ""} ${className}`}>
      <button className="primary split-main" onClick={onClick} disabled={disabled}>
        {children}
      </button>
      <Menu<"customize" | "none">
        className="split-more"
        label="More options"
        title="More options"
        value="none"
        options={[{ key: "customize", label: "Customize prompt…" }]}
        onChange={(k) => k === "customize" && !disabled && onCustomize()}
        trigger={
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            <path d="M2 3.5 5 6.5 8 3.5" />
          </svg>
        }
      />
    </div>
  );
}
