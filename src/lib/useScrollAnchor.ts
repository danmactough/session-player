import { useLayoutEffect, useRef, type DependencyList, type RefObject } from "react";

/**
 * Keeps the top-most visible `[data-anchor]` child of a scroll container at
 * the same position when `deps` change, so items added or removed around it
 * don't move what the user is looking at. Call it during render: the scroll
 * offset is read then, before React updates the DOM (afterwards the browser
 * may already have clamped it). The container must be positioned, since
 * offsetTop is measured against it.
 */
export function useScrollAnchor(ref: RefObject<HTMLElement | null>, deps: DependencyList) {
  const scrollTopBeforeCommit = ref.current?.scrollTop ?? 0;
  const previous = useRef<{ key: string; top: number; bottom: number }[]>([]);

  useLayoutEffect(() => {
    const el = ref.current;
    const prev = previous.current;
    if (!el || !prev.length) return;
    const top = scrollTopBeforeCommit;
    const oldIndex = prev.findIndex((p) => p.bottom > top);
    if (oldIndex < 0) return;
    const find = (k: string) => el.querySelector<HTMLElement>(`[data-anchor="${CSS.escape(k)}"]`);
    let target: HTMLElement | null = null;
    // If the anchor item disappeared, use the next one that survived.
    for (let i = oldIndex; !target && i < prev.length; i++) target = find(prev[i].key);
    if (target) el.scrollTop = Math.max(0, target.offsetTop - (prev[oldIndex].top - top));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  // Runs after the effect above on every commit, so it always holds the
  // positions from the commit before.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    previous.current = [...el.querySelectorAll<HTMLElement>("[data-anchor]")].map((it) => ({
      key: it.dataset.anchor!,
      top: it.offsetTop,
      bottom: it.offsetTop + it.offsetHeight,
    }));
  });
}
