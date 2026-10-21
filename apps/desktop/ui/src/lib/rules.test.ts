import { describe, expect, it } from "vitest";

import {
  ACTION_TYPES,
  GLOBAL_THROTTLE_RULE_ID,
  THROTTLE_PRESETS,
  actionSummary,
  activeThrottle,
  curlCommand,
  defaultAction,
  emptyMatcher,
  formatStatusRanges,
  isBuiltin,
  matcherSummary,
  moveItem,
  newRule,
  originLabel,
  parseStatusRanges,
  patternLabel,
  withThrottle,
  type Rule,
} from "@/lib/rules";

describe("rules helpers", () => {
  it("builds a default for every action type", () => {
    for (const { type } of ACTION_TYPES) {
      const action = defaultAction(type);
      expect(action.type).toBe(type);
      expect(actionSummary(action)).not.toBe("");
    }
  });

  it("labels patterns and summarises matchers", () => {
    expect(patternLabel(null)).toBe("any");
    expect(patternLabel({ kind: "regex", value: "^/a", case_insensitive: true })).toBe("/^/a/i");
    expect(patternLabel({ kind: "exact", value: "x", case_insensitive: false })).toBe("= x");
    const m = {
      ...emptyMatcher(),
      methods: ["GET", "POST"],
      host: { kind: "glob" as const, value: "*.example.com", case_insensitive: true },
      path: { kind: "exact" as const, value: "/v1/users", case_insensitive: false },
      headers: [{ name: "x-a", value: null, negate: false }],
    };
    expect(matcherSummary(m)).toBe("GET|POST *.example.com (Aa) /v1/users +1");
    expect(matcherSummary(emptyMatcher())).toBe("*");
  });

  it("labels origins, with the AI badge for agents", () => {
    expect(originLabel({ kind: "user" })).toBeNull();
    expect(originLabel({ kind: "preset", preset: "map_local" })).toBe("Map Local");
    expect(originLabel({ kind: "agent", client: "claude-code", session: null, created_at_ms: 0 })).toBe("AI · claude-code");
  });

  it("parses and formats status ranges", () => {
    expect(parseStatusRanges("2xx, 404, 500-599")).toEqual([
      { from: 200, to: 299 },
      { from: 404, to: 404 },
      { from: 500, to: 599 },
    ]);
    expect(parseStatusRanges("")).toEqual([]);
    expect(parseStatusRanges("abc")).toBeNull();
    expect(parseStatusRanges("500-400")).toBeNull();
    expect(formatStatusRanges([{ from: 200, to: 299 }, { from: 404, to: 404 }, { from: 500, to: 503 }])).toBe("2xx, 404, 500-503");
  });

  it("switches the global throttle between presets and off", () => {
    const rule: Rule = { ...newRule(), id: GLOBAL_THROTTLE_RULE_ID, enabled: false, actions: [defaultAction("Throttle")] };
    expect(isBuiltin(rule)).toBe(true);
    expect(activeThrottle(rule)).toBeNull();
    const edge = THROTTLE_PRESETS.find((p) => p.name === "Edge")!;
    const on = withThrottle(rule, edge);
    expect(on.enabled).toBe(true);
    expect(activeThrottle(on)?.name).toBe("Edge");
    expect(activeThrottle(withThrottle(on, null))).toBeNull();
    const custom = { ...on, actions: [{ type: "Throttle" as const, download_kbps: 1, upload_kbps: 2, latency_ms: 3 }] };
    expect(activeThrottle(custom)?.name).toBe("Custom");
  });

  it("moves list items", () => {
    expect(moveItem(["a", "b", "c"], 0, 2)).toEqual(["b", "c", "a"]);
    expect(moveItem(["a", "b", "c"], 2, 0)).toEqual(["c", "a", "b"]);
  });

  it("builds a basic curl command", () => {
    const cmd = curlCommand({
      request: {
        method: "POST",
        url: "https://api.example.com/v1/it's",
        version: "HTTP/2",
        headers: [
          { name: ":authority", value: "api.example.com" },
          { name: "host", value: "api.example.com" },
          { name: "content-type", value: "application/json" },
          { name: "content-length", value: "2" },
        ],
      },
    });
    expect(cmd).toBe(`curl -X POST 'https://api.example.com/v1/it'\\''s' -H 'content-type: application/json'`);
  });
});
