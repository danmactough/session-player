import { useEffect, useRef, useState } from "react";

/**
 * A dropdown of checkboxes, styled like Menu. Choosing an option toggles it
 * and leaves the menu open; nothing chosen means no filter.
 */
export function MultiMenu<K extends string>({
  label,
  options,
  value,
  onChange,
  className = "",
}: {
  label: string;
  options: { key: K; label: string }[];
  value: K[];
  onChange: (value: K[]) => void;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  }, [open]);

  const toggle = (key: K) => onChange(value.includes(key) ? value.filter((k) => k !== key) : [...value, key]);
  const chosen = options.filter((o) => value.includes(o.key));

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!open) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setOpen(true);
      }
      return;
    }
    // Keep j/k and other app shortcuts from firing while the menu is open.
    e.stopPropagation();
    if (e.key === "Escape") {
      setOpen(false);
      buttonRef.current?.focus();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => (i + 1) % options.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => (i - 1 + options.length) % options.length);
    } else if ((e.key === "Enter" || e.key === " ") && options[active]) {
      e.preventDefault();
      toggle(options[active].key);
    }
  };

  return (
    <div className={`menu ${open ? "open" : ""} ${chosen.length ? "filtering" : ""} ${className}`} ref={rootRef} onKeyDown={onKeyDown}>
      <button
        ref={buttonRef}
        className="menu-button"
        aria-haspopup="listbox"
        aria-expanded={open}
        title={chosen.length ? chosen.map((o) => o.label).join(", ") : undefined}
        onClick={() => setOpen((o) => !o)}
      >
        {label}
        {chosen.length === 1 ? `: ${chosen[0].label}` : chosen.length > 1 ? `: ${chosen.length}` : ""}
        <span className="caret">▾</span>
      </button>
      {open && (
        <ul className="menu-list" role="listbox" aria-multiselectable="true" aria-label={label}>
          {options.map((o, i) => (
            <li
              key={o.key}
              role="option"
              aria-selected={value.includes(o.key)}
              className={`${i === active ? "active" : ""} ${value.includes(o.key) ? "chosen" : ""}`}
              onPointerEnter={() => setActive(i)}
              onClick={() => toggle(o.key)}
            >
              <span className="check">{value.includes(o.key) ? "✓" : ""}</span>
              {o.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
