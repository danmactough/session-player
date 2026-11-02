import { useEffect, useRef, useState } from "react";

/**
 * A multi-select filter shown as a chip box: each chosen value is a chip with
 * its own ×, and clicking the box opens a checklist (styled like Menu) that
 * stays open while values are toggled. Nothing chosen means no filter.
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
  const fieldRef = useRef<HTMLDivElement>(null);

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
      if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        setOpen(true);
      }
      return;
    }
    // Keep j/k and other app shortcuts from firing while the menu is open.
    e.stopPropagation();
    if (e.key === "Escape") {
      setOpen(false);
      fieldRef.current?.focus();
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
    <div className={`menu chip-box ${open ? "open" : ""} ${chosen.length ? "filtering" : ""} ${className}`} ref={rootRef} onKeyDown={onKeyDown}>
      {/* The whole box opens the checklist; the chips' × buttons don't. */}
      <div
        ref={fieldRef}
        className="chip-box-field"
        role="button"
        tabIndex={0}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="chip-box-label">{label}</span>
        {chosen.map((o) => (
          <span key={o.key} className="chip-box-chip">
            {o.label}
            <button
              aria-label={`Remove ${o.label}`}
              onClick={(e) => {
                e.stopPropagation();
                toggle(o.key);
              }}
            >
              ×
            </button>
          </span>
        ))}
        {chosen.length > 1 && (
          <button
            className="chip-box-clear"
            title={`Clear ${label}`}
            onClick={(e) => {
              e.stopPropagation();
              onChange([]);
            }}
          >
            Clear
          </button>
        )}
      </div>
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
