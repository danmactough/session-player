import { useEffect, useRef, useState, type ReactNode } from "react";

export interface MenuOption<K extends string> {
  key: K;
  label: string;
  /** Shown where the checkmark would go. */
  icon?: ReactNode;
  /** Sits at the right edge (a keyboard shortcut, say). */
  hint?: ReactNode;
  /** Makes this a flyout: hovering it, or →, opens these options beside it. */
  submenu?: MenuOption<K>[];
}

interface Props<K extends string> {
  value: K;
  options: MenuOption<K>[];
  onChange: (key: K) => void;
  label: string;
  className?: string;
  title?: string;
  /** Replaces the button's "current choice ▾" content, e.g. with just a chevron. */
  trigger?: ReactNode;
}

/** A small dropdown that matches the app instead of the native select menu. */
export function Menu<K extends string>({ value, options, onChange, label, className = "", title, trigger }: Props<K>) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  /** Index into the open flyout's options, or null when none is open. */
  const [subActive, setSubActive] = useState<number | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const holds = (o: MenuOption<K>) => o.key === value || !!o.submenu?.some((c) => c.key === value);
  const current = options.flatMap((o) => [o, ...(o.submenu ?? [])]).find((o) => o.key === value);
  const sub = options[active]?.submenu;

  useEffect(() => {
    if (!open) return;
    setActive(Math.max(0, options.findIndex(holds)));
    setSubActive(null);
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
    } else if (subActive !== null && sub) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSubActive((i) => ((i ?? 0) + 1) % sub.length);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSubActive((i) => ((i ?? 0) - 1 + sub.length) % sub.length);
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        setSubActive(null);
      } else if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        choose(sub[subActive].key);
      }
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => (i + 1) % options.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => (i - 1 + options.length) % options.length);
    } else if (sub && (e.key === "ArrowRight" || e.key === "Enter" || e.key === " ")) {
      e.preventDefault();
      setSubActive(Math.max(0, sub.findIndex((c) => c.key === value)));
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
        {trigger ?? (
          <>
            {current?.label}
            <span className="caret">▾</span>
          </>
        )}
      </button>
      {open && (
        <ul className={`menu-list ${options.some((o) => o.icon) ? "with-icons" : ""}`} role="listbox" aria-label={label}>
          {options.map((o, i) => (
            <li
              key={o.key}
              role="option"
              aria-selected={holds(o)}
              aria-haspopup={o.submenu ? "listbox" : undefined}
              aria-expanded={o.submenu ? i === active && subActive !== null : undefined}
              className={`${i === active ? "active" : ""} ${holds(o) ? "chosen" : ""} ${o.submenu ? "has-sub" : ""}`}
              onPointerEnter={() => {
                setActive(i);
                setSubActive(o.submenu ? Math.max(0, o.submenu.findIndex((c) => c.key === value)) : null);
              }}
              onClick={() => (o.submenu ? setSubActive(Math.max(0, o.submenu.findIndex((c) => c.key === value))) : choose(o.key))}
            >
              {o.icon ? <span className="menu-icon">{o.icon}</span> : <span className="check">{holds(o) ? "✓" : ""}</span>}
              {o.label}
              {o.hint && <span className="menu-hint">{o.hint}</span>}
              {o.submenu && <span className="menu-more">▸</span>}
              {o.submenu && i === active && subActive !== null && (
                <ul className="menu-list menu-sub with-icons" role="listbox" aria-label={o.label}>
                  {o.submenu.map((c, j) => (
                    <li
                      key={c.key}
                      role="option"
                      aria-selected={c.key === value}
                      className={`${j === subActive ? "active" : ""} ${c.key === value ? "chosen" : ""}`}
                      onPointerEnter={() => setSubActive(j)}
                      onClick={(e) => {
                        e.stopPropagation();
                        choose(c.key);
                      }}
                    >
                      {c.icon ? <span className="menu-icon">{c.icon}</span> : <span className="check">{c.key === value ? "✓" : ""}</span>}
                      {c.label}
                      {c.hint && <span className="menu-hint">{c.hint}</span>}
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
