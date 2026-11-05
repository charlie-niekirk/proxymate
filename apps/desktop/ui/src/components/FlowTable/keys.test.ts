import { describe, expect, it } from "vitest";

import { resizeKey, tableKey, type TableKey, type TableState } from "./keys";

const key = (k: string, mods: Partial<TableKey> = {}): TableKey => ({
  key: k,
  shiftKey: false,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  ...mods,
});

const state = (s: Partial<TableState> = {}): TableState => ({
  from: 10,
  first: 5,
  last: 40,
  len: 100,
  page: 20,
  multi: false,
  selected: true,
  ...s,
});

describe("tableKey", () => {
  it.each([
    ["ArrowDown", 11],
    ["ArrowUp", 9],
    ["PageDown", 30],
    ["PageUp", 0],
    ["Home", 0],
    ["End", 99],
  ])("%s moves to row %i", (k, to) => {
    expect(tableKey(key(k), state())).toEqual({ kind: "move", to, extend: false });
  });

  it("starts from the rendered rows when nothing is focused", () => {
    expect(tableKey(key("ArrowDown"), state({ from: null }))).toEqual({ kind: "move", to: 5, extend: false });
    expect(tableKey(key("ArrowUp"), state({ from: null }))).toEqual({ kind: "move", to: 40, extend: false });
    expect(tableKey(key("PageDown"), state({ from: null }))).toEqual({ kind: "move", to: 25, extend: false });
  });

  it("clamps to the view", () => {
    expect(tableKey(key("ArrowDown"), state({ from: 99 }))).toEqual({ kind: "move", to: 99, extend: false });
    expect(tableKey(key("ArrowUp"), state({ from: 0 }))).toEqual({ kind: "move", to: 0, extend: false });
    expect(tableKey(key("PageDown"), state({ from: 95 }))).toEqual({ kind: "move", to: 99, extend: false });
  });

  it("extends the selection with ⇧", () => {
    expect(tableKey(key("ArrowDown", { shiftKey: true }), state())).toEqual({ kind: "move", to: 11, extend: true });
    expect(tableKey(key("End", { shiftKey: true }), state())).toEqual({ kind: "move", to: 99, extend: true });
  });

  it("does nothing to an empty view but still takes the key", () => {
    expect(tableKey(key("ArrowDown"), state({ len: 0, from: null }))).toEqual({ kind: "none" });
  });

  it("Enter goes to the inspector only with a flow focused", () => {
    expect(tableKey(key("Enter"), state())).toEqual({ kind: "inspect" });
    expect(tableKey(key("Enter"), state({ selected: false }))).toEqual({ kind: "none" });
  });

  it("Escape collapses a multi-selection and is left alone otherwise", () => {
    expect(tableKey(key("Escape"), state({ multi: true }))).toEqual({ kind: "collapse" });
    expect(tableKey(key("Escape"), state())).toBeNull();
  });

  it("leaves ⌘, Ctrl and ⌥ keys and other keys to the app", () => {
    expect(tableKey(key("ArrowDown", { metaKey: true }), state())).toBeNull();
    expect(tableKey(key("ArrowDown", { ctrlKey: true }), state())).toBeNull();
    expect(tableKey(key("ArrowDown", { altKey: true }), state())).toBeNull();
    expect(tableKey(key("a"), state())).toBeNull();
    expect(tableKey(key("ArrowLeft"), state())).toBeNull();
  });
});

describe("resizeKey", () => {
  const col = { px: 100, minWidth: 40, width: 120 };
  it("steps by 8 px, 32 with ⇧, within the limits", () => {
    expect(resizeKey(key("ArrowRight"), col, 600)).toBe(108);
    expect(resizeKey(key("ArrowLeft"), col, 600)).toBe(92);
    expect(resizeKey(key("ArrowLeft", { shiftKey: true }), col, 600)).toBe(68);
    expect(resizeKey(key("ArrowLeft"), { ...col, px: 44 }, 600)).toBe(40);
    expect(resizeKey(key("ArrowRight"), { ...col, px: 598 }, 600)).toBe(600);
  });

  it("Home, End and Enter go to the minimum, maximum and default", () => {
    expect(resizeKey(key("Home"), col, 600)).toBe(40);
    expect(resizeKey(key("End"), col, 600)).toBe(600);
    expect(resizeKey(key("Enter"), col, 600)).toBe(120);
  });

  it("ignores other keys", () => {
    expect(resizeKey(key("ArrowUp"), col, 600)).toBeNull();
  });
});
