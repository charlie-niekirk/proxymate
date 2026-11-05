import { beforeEach, describe, expect, it } from "vitest";

import type { Rule } from "@/lib/rules";
import { isSelected, selectionCount } from "@/lib/selection";
import { usePrefs } from "@/store/prefs";
import { LIVE_STORE, useUi } from "@/store/ui";

const initial = useUi.getState();
const ui = () => useUi.getState();

describe("store/ui", () => {
  beforeEach(() => useUi.setState(initial, true));

  describe("the active store", () => {
    it("starts over on another store: no selection, no rows", () => {
      ui().select(4);
      ui().setStats({ len: 10, total: 12 });
      ui().setStore(2);
      expect(ui().store).toBe(2);
      expect(ui().selectedId).toBeNull();
      expect(selectionCount(ui().selection, 10)).toBe(0);
      expect(ui().stats).toEqual({ len: 0, total: 0 });
    });

    it("keeps everything when the same store is set again", () => {
      ui().select(4);
      ui().setStats({ len: 10, total: 12 });
      ui().setStore(LIVE_STORE);
      expect(ui().selectedId).toBe(4);
      expect(ui().stats).toEqual({ len: 10, total: 12 });
    });
  });

  describe("the filter", () => {
    it("changes one part at a time", () => {
      ui().setText("api");
      ui().setProtocol("https");
      ui().setScope({ host: "example.com" });
      expect(ui().filter).toEqual({ text: "api", protocol: "https", content: [], host: "example.com", client: null });
      // A scope replaces both parts: setting a client clears the host.
      ui().setScope({ client: "Safari" });
      expect(ui().filter.host).toBeNull();
      expect(ui().filter.client).toBe("Safari");
    });

    it("toggles content chips in the order they were picked, and clears them", () => {
      ui().toggleContent("json");
      ui().toggleContent("media");
      expect(ui().filter.content).toEqual(["json", "media"]);
      ui().toggleContent("json");
      expect(ui().filter.content).toEqual(["media"]);
      ui().clearContent();
      expect(ui().filter.content).toEqual([]);
    });
  });

  it("remembers the sort across launches", () => {
    ui().setSort({ key: "status", dir: "desc" });
    expect(ui().sort).toEqual({ key: "status", dir: "desc" });
    expect(usePrefs.getState().sort).toEqual({ key: "status", dir: "desc" });
  });

  describe("the selection", () => {
    it("select focuses one flow alone, and null selects nothing", () => {
      ui().select(3);
      expect(ui().selectedId).toBe(3);
      expect(isSelected(ui().selection, 3)).toBe(true);
      expect(selectionCount(ui().selection, 100)).toBe(1);
      ui().select(null);
      expect(selectionCount(ui().selection, 100)).toBe(0);
    });

    it("⌘-click adds and focuses a flow; removing the focused one moves the focus", () => {
      ui().select(1);
      ui().toggleSelected(5);
      expect(ui().selectedId).toBe(5);
      expect(selectionCount(ui().selection, 100)).toBe(2);
      ui().toggleSelected(5);
      expect(ui().selectedId).toBe(1);
      expect(isSelected(ui().selection, 5)).toBe(false);
      // Removing an unfocused flow leaves the focus alone.
      ui().toggleSelected(7);
      ui().toggleSelected(1);
      expect(ui().selectedId).toBe(7);
    });

    it("a whole-view selection keeps the focus unless one is given", () => {
      ui().select(2);
      ui().setSelection({ all: true, ids: new Set() });
      expect(ui().selectedId).toBe(2);
      expect(selectionCount(ui().selection, 50)).toBe(50);
      ui().setSelection({ all: true, ids: new Set([4]) }, 9);
      expect(ui().selectedId).toBe(9);
      expect(isSelected(ui().selection, 4)).toBe(false);
    });
  });

  describe("dialogs", () => {
    it("Compose, Rewrite, Rules and Scripts bump their seq on every opening, so forms remount", () => {
      ui().openCompose();
      ui().closeCompose();
      ui().openCompose();
      expect(ui().compose).toMatchObject({ open: true, spec: null, seq: 2 });
      ui().openRewrite({ store: 1, flow: 3 });
      expect(ui().rewrite).toMatchObject({ open: true, store: 1, flow: 3, rule: null, seq: 1 });
      ui().closeRewrite();
      expect(ui().rewrite).toMatchObject({ open: false, flow: 3 });
      ui().openRulesEditor();
      ui().openRulesEditor();
      expect(ui().rulesEditor.seq).toBe(2);
    });

    it("the Rules Editor keeps its rule unless given another, and a draft wins over a rule", () => {
      ui().openRulesEditor({ ruleId: "a" });
      ui().closeRulesEditor();
      ui().openRulesEditor();
      expect(ui().rulesEditor.ruleId).toBe("a");
      const draft = { id: "d" } as unknown as Rule;
      ui().openRulesEditor({ ruleId: "b", draft });
      expect(ui().rulesEditor).toMatchObject({ ruleId: null, draft });
      // Closing drops an unsaved draft.
      ui().closeRulesEditor();
      expect(ui().rulesEditor.draft).toBeNull();
    });

    it("Settings reopens on the page it was left on", () => {
      ui().openSettings("storage");
      ui().closeSettings();
      ui().openSettings();
      expect(ui().settings).toEqual({ open: true, page: "storage" });
    });

    it("picking no script view turns script views off", () => {
      ui().pickScriptView(null);
      expect(ui().scriptViewsOff).toBe(true);
      ui().pickScriptView("base64");
      expect(ui()).toMatchObject({ pickedScriptView: "base64", scriptViewsOff: false });
    });
  });

  it("ignores deep-search progress that hasn't moved", () => {
    const progress = { checked: 10, total: 100, matched: 2, running: true };
    ui().setDeep(progress);
    ui().setDeep({ ...progress });
    // The same object: nothing re-renders for it.
    expect(ui().deep).toBe(progress);
    ui().setDeep({ ...progress, checked: 20 });
    expect(ui().deep?.checked).toBe(20);
  });
});
