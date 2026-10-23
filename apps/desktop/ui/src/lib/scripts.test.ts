import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import type { ViewResult_Serialize } from "@/bindings";
import {
  ENTRY_POINTS,
  GLOBAL_COMPLETIONS,
  TEMPLATES,
  chooseViewer,
  scriptErrors,
  scriptViewBytes,
  scriptViewOptions,
} from "@/lib/scripts";

const read = (path: string) => readFileSync(fileURLToPath(new URL(path, import.meta.url)), "utf8");
const DTS = read("../../../../../crates/pm-core/src/scripts/proxymate.d.ts");
const SURFACE = read("../../../../../scripts/api_surface.txt")
  .split("\n")
  .map((l) => l.trim())
  .filter((l) => l && !l.startsWith("#"));

function result(id: string, titles: string[], error: string | null = null): ViewResult_Serialize {
  return {
    script_id: id,
    script_name: `Script ${id}`,
    views: titles.map((title) => ({ title, kind: "json", text: "{}", base64: null, mime: null, truncated: false })),
    error,
    duration_ms: 1,
  };
}

describe("script views in the viewer dropdown", () => {
  const results = [result("a", ["Script a"]), result("b", ["payload", "token"]), result("c", [], "Error: boom")];
  const options = scriptViewOptions(results);

  it("lists every view, named by the script when it has one view of its own name", () => {
    expect(options.map((o) => [o.value, o.label])).toEqual([
      ["script:a:0", "Script a"],
      ["script:b:0", "payload"],
      ["script:b:1", "token"],
    ]);
    expect(scriptErrors(results)).toEqual([{ scriptId: "c", scriptName: "Script c", error: "Error: boom" }]);
  });

  it("auto-selects the first script view", () => {
    expect(chooseViewer("json", options, null, false)).toBe("script:a:0");
    expect(chooseViewer("json", [], null, false)).toBe("json");
  });

  it("keeps the view the user picked, while this body has it", () => {
    expect(chooseViewer("json", options, "script:b:1", false)).toBe("script:b:1");
    expect(chooseViewer("json", options, "script:z:0", false)).toBe("script:a:0");
  });

  it("stays with the built-in viewer once the user chose it", () => {
    expect(chooseViewer("json-tree", options, null, true)).toBe("json-tree");
    expect(chooseViewer("json-tree", options, "script:b:0", true)).toBe("script:b:0");
  });

  it("decodes base64 views and encodes text ones", () => {
    const view = { title: "", kind: "hex" as const, text: null, base64: "AAH/", mime: null, truncated: false };
    expect([...scriptViewBytes(view)]).toEqual([0, 1, 255]);
    expect(new TextDecoder().decode(scriptViewBytes({ ...view, kind: "text", text: "é", base64: null }))).toBe("é");
  });
});

describe("the editor's completions match proxymate.d.ts", () => {
  const globals = [...new Set(SURFACE.map((l) => l.split(".")[0]!))];

  it("cover every global in api_surface.txt, and only those", () => {
    expect(Object.keys(GLOBAL_COMPLETIONS).sort()).toEqual([...globals].sort());
  });

  it("list exactly the documented members", () => {
    for (const name of globals) {
      const members = SURFACE.filter((l) => l.startsWith(`${name}.`)).map((l) => l.slice(name.length + 1));
      expect(GLOBAL_COMPLETIONS[name]!.members.map((m) => m.label).sort(), name).toEqual(members.sort());
    }
  });

  it("are all declared in proxymate.d.ts", () => {
    for (const name of globals) {
      expect(DTS, name).toMatch(new RegExp(`declare (const|class|function) ${name}\\b`));
    }
    for (const line of SURFACE.filter((l) => l.includes("."))) {
      const member = line.split(".")[1]!;
      expect(DTS, line).toMatch(new RegExp(`\\b${member}\\(`));
    }
    for (const entry of Object.values(ENTRY_POINTS).flat()) {
      expect(DTS, entry.label).toContain(`declare function ${entry.label}(`);
    }
  });
});

describe("templates", () => {
  it("export their kind's entry point", () => {
    const entry = { view: /export function onView\(/, mock: /export function onMock\(/, rewrite: /export function on(Request|Response)\(/ };
    for (const t of TEMPLATES) expect(t.source, t.id).toMatch(entry[t.kind]);
  });

  it("include decoding a base64 JSON field and a JWT", () => {
    expect(TEMPLATES.map((t) => t.id)).toEqual(expect.arrayContaining(["decode-base64-json", "decode-jwt"]));
  });
});
