import { useEffect, useRef, useState } from "react";

interface Props<K extends string> {
  value: K;
  options: { key: K; label: string }[];
  onChange: (key: K) => void;
  label: string;
  className?: string;
  title?: string;
}

/** A small dropdown that matches the app instead of the native select menu. */
export function Menu<K extends string>({ value, options, onChange, label, className = "", title }: Props<K>) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const current = options.find((o) => o.key === value);

  useEffect(() => {
    if (!open) return;
    setActive(Math.max(0, options.findIndex((o) => o.key === value)));
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const choose = (key: K) => {
    onChange(key);
    setOpen(false);
    buttonRef.current?.focus();
  };

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
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      choose(options[active].key);
    }
  };

  return (
    <div className={`menu ${open ? "open" : ""} ${className}`} ref={rootRef} onKeyDown={onKeyDown}>
      <button
        ref={buttonRef}
        className="menu-button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={`${label}: ${current?.label ?? ""}`}
        title={title}
        onClick={() => setOpen((o) => !o)}
      >
        {current?.label}
        <span className="caret">▾</span>
      </button>
      {open && (
        <ul className="menu-list" role="listbox" aria-label={label}>
          {options.map((o, i) => (
            <li
              key={o.key}
              role="option"
              aria-selected={o.key === value}
              className={`${i === active ? "active" : ""} ${o.key === value ? "chosen" : ""}`}
              onPointerEnter={() => setActive(i)}
              onClick={() => choose(o.key)}
            >
              <span className="check">{o.key === value ? "✓" : ""}</span>
              {o.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
