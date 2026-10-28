import { describe, expect, it } from "vitest";

import { EMPTY_SELECTION, type TableSelection, isMulti, isSelected, listedIds, range, selectionCount, single, toFlowSelection, toggle } from "./selection";

describe("table selection", () => {
  it("toggles ids in and out", () => {
    let sel = single(3);
    sel = toggle(sel, 7);
    expect([isSelected(sel, 3), isSelected(sel, 7), isSelected(sel, 5)]).toEqual([true, true, false]);
    sel = toggle(sel, 3);
    expect(isSelected(sel, 3)).toBe(false);
    expect(selectionCount(sel, 100)).toBe(1);
    expect(isMulti(sel, 100)).toBe(false);
  });

  it("keeps a whole view as exceptions, never as a list", () => {
    let sel: TableSelection = { all: true, ids: new Set<number>() };
    sel = toggle(sel, 4);
    expect([isSelected(sel, 4), isSelected(sel, 99_999)]).toEqual([false, true]);
    expect(selectionCount(sel, 100_000)).toBe(99_999);
    expect(listedIds(sel)).toBeNull();
    expect(toFlowSelection(sel, 12)).toEqual({ kind: "view", view: 12, except: [4] });
  });

  it("lists ids in arrival order for the engine", () => {
    expect(toFlowSelection(range([9, 2, 5]), 1)).toEqual({ kind: "ids", ids: [2, 5, 9] });
    expect(toFlowSelection(EMPTY_SELECTION, 1)).toEqual({ kind: "ids", ids: [] });
    expect(single(null).ids.size).toBe(0);
  });
});
