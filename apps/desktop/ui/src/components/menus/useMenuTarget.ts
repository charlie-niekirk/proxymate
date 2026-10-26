import { useState, type MouseEvent } from "react";

/** The text selected inside `container`, or "" if the selection is elsewhere (or empty). */
export function selectionIn(container: Element): string {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || !sel.anchorNode || !container.contains(sel.anchorNode)) return "";
  return sel.toString();
}

export interface MenuTarget<T> {
  /** What the right-click landed on, or null for blank space. */
  target: T | null;
  /** Text selected in the trigger at that moment (WebKit selects the word under the cursor first). */
  selection: string;
  onContextMenu: (e: MouseEvent) => void;
}

/**
 * One context menu for a whole list or table: rows carry `data-menu-key`
 * and a right-click resolves the row under the cursor (virtualised rows
 * are positioned, so DOM order means nothing). With nothing under the
 * cursor and nothing selected the menu stays shut, unless `always`, since
 * it would be empty.
 */
export function useMenuTarget<T>(
  /** `el` is the row; `target` what was clicked inside it (a cell). */
  resolve: (key: string, el: HTMLElement, target: Element) => T | null | undefined,
  { always = false }: { always?: boolean } = {},
): MenuTarget<T> {
  const [state, setState] = useState<{ target: T | null; selection: string }>({ target: null, selection: "" });
  const onContextMenu = (e: MouseEvent) => {
    const el = (e.target as Element).closest<HTMLElement>("[data-menu-key]");
    const key = el && e.currentTarget.contains(el) ? el.dataset.menuKey : undefined;
    const target = el && key !== undefined ? (resolve(key, el, e.target as Element) ?? null) : null;
    const selection = selectionIn(e.currentTarget);
    if (target === null && !selection && !always) {
      // Radix opens only if the event isn't prevented; this also keeps the native menu away.
      e.preventDefault();
      return;
    }
    setState({ target, selection });
  };
  return { ...state, onContextMenu };
}
