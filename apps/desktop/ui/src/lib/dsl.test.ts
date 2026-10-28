import { describe, expect, it } from "vitest";

import { accept, hasDeepTerms, suggest, termAt, bodySearchTerms } from "./dsl";

const ctx = { hosts: ["api.github.com", "github.com", "localhost"], domains: ["github.com"], apps: ["Safari", "Google Chrome"] };

describe("dsl autocomplete", () => {
  it("finds the term at the caret, quotes included", () => {
    expect(termAt("host:a sta", 10)).toEqual({ start: 7, term: "sta" });
    expect(termAt('app:"Google Ch', 14)).toEqual({ start: 0, term: 'app:"Google Ch' });
  });

  it("suggests keys, numeric ones with operators", () => {
    expect(suggest("st", 2, ctx).map((s) => s.label)).toEqual(["status:", "state:"]);
    expect(suggest("du", 2, ctx).map((s) => s.label)).toEqual(["dur>", "dur<"]);
    expect(suggest("-ho", 3, ctx)[0]?.insert).toBe("-host:");
    expect(suggest("dur>5", 5, ctx)).toEqual([]);
  });

  it("suggests known hosts, domains, apps and fixed values", () => {
    expect(suggest("host:git", 8, ctx).map((s) => s.label)).toEqual(["*.github.com", "github.com", "api.github.com"]);
    expect(suggest("host:", 5, ctx).map((s) => s.label)).toContain("localhost");
    expect(suggest("app:goo", 7, ctx).map((s) => s.insert)).toEqual(['app:"Google Chrome"']);
    expect(suggest("method:get,po", 13, ctx).map((s) => s.insert)).toEqual(["method:get,POST"]);
    expect(suggest("status:4", 8, ctx).map((s) => s.label).slice(0, 2)).toEqual(["4xx", "400"]);
    expect(suggest("is:p", 4, ctx).map((s) => s.label)).toEqual(["pinned", "paused"]);
  });

  it("replaces the term and moves the caret", () => {
    const key = suggest("ho", 2, ctx)[0]!;
    expect(accept("ho api", 2, key)).toEqual({ text: "host: api", caret: 5 });
    const value = suggest("x host:gi", 9, ctx)[0]!;
    expect(accept("x host:gi", 9, value)).toEqual({ text: "x host:*.github.com ", caret: 20 });
  });

  it("spots deep terms", () => {
    expect(hasDeepTerms("host:a body:token")).toBe(true);
    expect(hasDeepTerms("-reqheader:x-debug")).toBe(true);
    expect(hasDeepTerms("somebody:x body")).toBe(false);
  });
});

describe("bodySearchTerms", () => {
  it("takes body terms for the right side and bare text", () => {
    expect(bodySearchTerms('host:*.example.com body:needle resbody:"two words" -body:skip token', "response")).toEqual([
      "two words",
      "needle",
      "token",
    ]);
    expect(bodySearchTerms("reqbody:abc resbody:xyz", "request")).toEqual(["abc"]);
    expect(bodySearchTerms("status:4xx dur>500 label:red is:commented", "response")).toEqual([]);
    expect(bodySearchTerms("body:", "response")).toEqual([]);
  });
});
