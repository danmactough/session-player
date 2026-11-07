import { useEffect, useRef, useState } from "react";

export interface FilterGroup {
  key: string;
  label: string;
  options: { key: string; label: string }[];
  /** The options chosen; empty means the group doesn't filter. */
  value: string[];
  onChange: (value: string[]) => void;
}

interface Props {
  groups: FilterGroup[];
  onClearAll: () => void;
}

/**
 * One "Filter" menu for the session list: each group (Analyzed, Outcome,
 * Annotation type) opens a submenu of options to tick, several at once. The
 * menu stays open while options are ticked. With any filter on, the button
 * shows how many, and an × beside it clears them all.
 */
export function FilterMenu({ groups, onClearAll }: Props) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  /** Index into the open group's options, or null when no submenu is open. */
  const [subActive, setSubActive] = useState<number | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const count = groups.reduce((n, g) => n + g.value.length, 0);
  const group = groups[active];

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  }, [open]);

  const toggle = (g: FilterGroup, key: string) => g.onChange(g.value.includes(key) ? g.value.filter((k) => k !== key) : [...g.value, key]);

  const openMenu = () => {
    setActive(0);
    setSubActive(null);
    setOpen(true);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!open) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        openMenu();
      }
      return;
    }
    // Keep j/k and other app shortcuts from firing while the menu is open.
    e.stopPropagation();
    if (e.key === "Escape") {
      setOpen(false);
      buttonRef.current?.focus();
    } else if (subActive !== null && group) {
      const n = group.options.length;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSubActive((i) => ((i ?? 0) + 1) % n);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSubActive((i) => ((i ?? 0) - 1 + n) % n);
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        setSubActive(null);
      } else if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        toggle(group, group.options[subActive].key);
      }
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => (i + 1) % groups.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => (i - 1 + groups.length) % groups.length);
    } else if (e.key === "ArrowRight" || e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      setSubActive(0);
    }
  };

  return (
    <div className={`menu filter-menu ${open ? "open" : ""} ${count ? "filtering" : ""}`} ref={rootRef} onKeyDown={onKeyDown}>
      <button
        ref={buttonRef}
        className="menu-button filter-button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={count ? `Filter: ${count} on` : "Filter"}
        onClick={() => (open ? setOpen(false) : openMenu())}
      >
        Filter
        {count > 0 && <span className="filter-count">• ({count})</span>}
        <span className="caret">▾</span>
      </button>
      {count > 0 && (
        <button className="filter-clear" onClick={onClearAll} aria-label="Clear all filters" title="Clear all filters">
          ×
        </button>
      )}
      {open && (
        <ul className="menu-list" role="menu" aria-label="Filter">
          {groups.map((g, i) => (
            <li
              key={g.key}
              role="menuitem"
              aria-haspopup="menu"
              aria-expanded={i === active && subActive !== null}
              className={`has-sub ${i === active ? "active" : ""} ${g.value.length ? "chosen" : ""}`}
              onPointerEnter={() => {
                setActive(i);
                setSubActive(0);
              }}
              onClick={() => {
                setActive(i);
                setSubActive(0);
              }}
            >
              <span className="check">{g.value.length ? "✓" : ""}</span>
              {g.label}
              {g.value.length > 0 && <span className="menu-hint">{g.value.length}</span>}
              <span className="menu-more">▸</span>
              {i === active && subActive !== null && (
                <ul className="menu-list menu-sub" role="menu" aria-label={g.label}>
                  {g.options.map((o, j) => {
                    const on = g.value.includes(o.key);
                    return (
                      <li
                        key={o.key}
                        role="menuitemcheckbox"
                        aria-checked={on}
                        className={`${j === subActive ? "active" : ""} ${on ? "chosen" : ""}`}
                        onPointerEnter={() => setSubActive(j)}
                        onClick={(e) => {
                          e.stopPropagation();
                          toggle(g, o.key);
                        }}
                      >
                        <span className="check">{on ? "✓" : ""}</span>
                        {o.label}
                      </li>
                    );
                  })}
                </ul>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
