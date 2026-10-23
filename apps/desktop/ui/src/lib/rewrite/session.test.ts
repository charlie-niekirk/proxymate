import { describe, expect, it } from "vitest";

import type { RuleTestResult_Serialize } from "@/bindings";
import { insert, rename, setValue, type JNode } from "@/lib/json/doc";
import type { Action, Rule } from "@/lib/rules";

import { NO_PINS } from "./diff";
import { bodySource, buildActions, headerRows, initialEdits, rebuild, type Edits, type Source } from "./session";

const enc = (s: string) => new TextEncoder().encode(s);

function source(responseBody: string, requestBody = "", contentType = "application/json"): Source {
  const query = [{ id: 900, name: "page", value: "1" }];
  return {
    store: 0,
    flow: 1,
    target: { method: "POST", scheme: "https", host: "api.test", port: 443, path: "/x", query: [["page", "1"]] },
    url: "https://api.test/x?page=1",
    request: {
      headers: headerRows([{ name: "Content-Type", value: contentType }]),
      query,
      body: bodySource(enc(requestBody), contentType, "none", "request", query),
      contentType,
    },
    response: {
      status: 200,
      headers: headerRows([{ name: "Content-Type", value: "application/json" }, { name: "Content-Length", value: "10" }]),
      body: bodySource(enc(responseBody), "application/json", "json", "response", []),
      contentType: "application/json",
    },
  };
}

const keyId = (n: JNode, key: string) => (n.t === "obj" ? n.entries.find((e) => e.key === key)!.id : -1);

function editResponse(src: Source, edit: (doc: JNode) => JNode): Edits {
  const edits = initialEdits(src);
  const body = edits.response!.body;
  if (body.kind !== "json") throw new Error("not json");
  return { ...edits, response: { ...edits.response!, body: { ...body, doc: edit(body.doc) } } };
}

describe("buildActions", () => {
  it("nothing edited is no actions", () => {
    const src = source('{"a":1}');
    expect(buildActions(src, initialEdits(src), NO_PINS).actions).toEqual([]);
  });

  it("edit a value, rename a key and add one", () => {
    const src = source('{"id":12345678901234567890,"name":"Ana","role":"user"}');
    const edits = editResponse(src, (doc) => {
      let d = setValue(doc, [keyId(doc, "name")], { t: "str", v: "Bea" });
      d = rename(d, [keyId(d, "role")], "kind");
      return insert(d, [], null, { t: "bool", v: true }, "admin").root;
    });
    expect(buildActions(src, edits, NO_PINS).actions).toEqual([
      { type: "JsonRemove", phase: "response", pointer: "/role" },
      { type: "JsonSet", phase: "response", pointer: "/name", value: null, raw: '"Bea"' },
      { type: "JsonSet", phase: "response", pointer: "/kind", value: null, raw: '"user"' },
      { type: "JsonSet", phase: "response", pointer: "/admin", value: null, raw: "true" },
    ]);
  });

  it("the whole-body toggle sets the body, keeping key order and numbers", () => {
    const src = source('{\n  "b": 1.50,\n  "a": 12345678901234567890\n}');
    const edits = editResponse(src, (doc) => setValue(doc, [keyId(doc, "b")], { t: "num", raw: "2.50" }));
    const body = edits.response!.body;
    if (body.kind !== "json") throw new Error("not json");
    const whole = { ...edits, response: { ...edits.response!, body: { ...body, whole: true } } };
    expect(buildActions(src, whole, NO_PINS).actions).toEqual([
      { type: "SetBody", phase: "response", body: '{\n  "b": 2.50,\n  "a": 12345678901234567890\n}', content_type: null },
    ]);
  });

  it("GraphQL variables and form fields", () => {
    const gql = source("{}", '{"query":"query Q($n: Int) { a }","variables":{"n":1}}');
    const edits = initialEdits(gql);
    const vars = edits.request.body;
    if (vars.kind !== "graphql") throw new Error(`not graphql: ${vars.kind}`);
    edits.request.body = { ...vars, vars: setValue(vars.vars, [keyId(vars.vars, "n")], { t: "num", raw: "5" }) };
    expect(buildActions(gql, edits, NO_PINS).actions).toEqual([
      { type: "JsonSet", phase: "request", pointer: "/variables/n", value: null, raw: "5" },
    ]);

    const form = source("{}", "a=1&b=two+words", "application/x-www-form-urlencoded");
    const f = initialEdits(form);
    const rows = f.request.body;
    if (rows.kind !== "form") throw new Error("not a form");
    f.request.body = { ...rows, rows: rows.rows.map((r) => (r.name === "b" ? { ...r, value: "x" } : r)) };
    expect(buildActions(form, f, NO_PINS).actions).toEqual([{ type: "FormSet", phase: "request", name: "b", value: "x" }]);
  });
});

describe("rebuild", () => {
  const dry = (src: Source, responseBody: string, extra: Partial<RuleTestResult_Serialize> = {}): RuleTestResult_Serialize => ({
    matched: true,
    mismatch: null,
    response_matched: true,
    request: { method: "POST", url: src.url, version: "HTTP/1.1", headers: src.request.headers },
    request_body: "",
    response: { status: 200, reason: "OK", version: "HTTP/1.1", headers: [{ name: "Content-Type", value: "application/json" }, { name: "X-New", value: "1" }] },
    response_body: responseBody,
    notes: [],
    request_body_truncated: false,
    response_body_truncated: false,
    ...extra,
  });

  it("reopens a saved rule as the same actions, pins and kept ones", () => {
    const src = source('{"name":"Ana","n":1,"items":[1]}');
    const actions: Action[] = [
      { type: "SetHeader", phase: "response", name: "X-New", value: "1" },
      // Equal to the flow's value: kept because it is pinned.
      { type: "JsonSet", phase: "response", pointer: "/n", value: null, raw: "1" },
      { type: "JsonSet", phase: "response", pointer: "/name", value: null, raw: '"Bea"' },
      { type: "JsonSet", phase: "response", pointer: "/items/-", value: null, raw: "2" },
      { type: "JsonSet", phase: "response", pointer: "/missing/x", value: null, raw: "0" },
      { type: "Delay", phase: "response", ms: 100 },
    ];
    const rule: Rule = { id: "r", name: "r", enabled: true, order: 1, origin: { kind: "preset", preset: "rewrite" }, matcher: { methods: [], scheme: null, host: null, port: null, path: null, query: [], headers: [], client_app: null, response: null }, actions };
    const r = rebuild(src, rule, dry(src, '{"name":"Bea","n":1,"items":[1,2],"missing":{"x":0}}'));
    const built = buildActions(src, r.edits, r.pins, r.kept).actions;
    expect(built).toEqual([actions[0], actions[2], actions[1], actions[3], actions[4], actions[5]]);
  });

  it("keeps body actions it can't rebuild (a truncated dry run)", () => {
    const src = source('{"a":1}');
    const actions: Action[] = [{ type: "JsonSet", phase: "response", pointer: "/a", value: null, raw: "2" }];
    const rule: Rule = { id: "r", name: "r", enabled: true, order: 1, origin: { kind: "user" }, matcher: { methods: [], scheme: null, host: null, port: null, path: null, query: [], headers: [], client_app: null, response: null }, actions };
    const r = rebuild(src, rule, dry(src, '{"a":', { response_body_truncated: true }));
    expect(r.kept).toEqual(actions);
  });
});
