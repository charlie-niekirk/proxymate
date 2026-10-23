import { useCallback, useMemo, useReducer } from "react";

/** A value with an undo history (the JSON editor's document, say). */
export interface Undoable<T> {
  value: T;
  /** A new value; it becomes one undo step. */
  set(next: T | ((current: T) => T)): void;
  undo(): void;
  redo(): void;
  canUndo: boolean;
  canRedo: boolean;
  /** Starts over from `value`, forgetting the history. */
  reset(value: T): void;
}

interface History<T> {
  past: T[];
  present: T;
  future: T[];
}

type Step<T> = { kind: "set"; next: T | ((current: T) => T) } | { kind: "undo" } | { kind: "redo" } | { kind: "reset"; value: T };

function reducer<T>(limit: number) {
  return (h: History<T>, step: Step<T>): History<T> => {
    switch (step.kind) {
      case "set": {
        const next = typeof step.next === "function" ? (step.next as (c: T) => T)(h.present) : step.next;
        if (Object.is(next, h.present)) return h;
        return { past: [...h.past, h.present].slice(-limit), present: next, future: [] };
      }
      case "undo": {
        const prev = h.past[h.past.length - 1];
        if (h.past.length === 0) return h;
        return { past: h.past.slice(0, -1), present: prev as T, future: [h.present, ...h.future] };
      }
      case "redo": {
        if (h.future.length === 0) return h;
        return { past: [...h.past, h.present], present: h.future[0] as T, future: h.future.slice(1) };
      }
      case "reset":
        return { past: [], present: step.value, future: [] };
    }
  };
}

/** Keeps `limit` undo steps (the plan's 200 for the JSON editor). */
export function useUndoable<T>(initial: () => T, limit = 200): Undoable<T> {
  const [h, dispatch] = useReducer(reducer<T>(limit), undefined, () => ({ past: [], present: initial(), future: [] }));
  const set = useCallback((next: T | ((current: T) => T)) => dispatch({ kind: "set", next }), []);
  const undo = useCallback(() => dispatch({ kind: "undo" }), []);
  const redo = useCallback(() => dispatch({ kind: "redo" }), []);
  const reset = useCallback((value: T) => dispatch({ kind: "reset", value }), []);
  return useMemo(
    () => ({ value: h.present, set, undo, redo, reset, canUndo: h.past.length > 0, canRedo: h.future.length > 0 }),
    [h, set, undo, redo, reset],
  );
}
