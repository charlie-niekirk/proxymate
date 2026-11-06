import { describe, expect, expectTypeOf, it } from "vitest";

import type { ProxyStatus_Serialize, Rule_Serialize } from "@/bindings";
import * as q from "@/lib/query";
import { queries } from "@/lib/queries";

/** One key from every factory and constant in lib/query.ts. */
const SAMPLES: [string, readonly unknown[]][] = Object.entries(q).flatMap(([name, value]): [string, readonly unknown[]][] => {
  if (Array.isArray(value)) return [[name, value]];
  if (typeof value === "function" && name.endsWith("Key")) {
    const args = Array.from({ length: value.length }, (_, i) => (i === 0 ? 0 : i === 2 ? "response" : 1));
    return [[name, (value as (...a: unknown[]) => readonly unknown[])(...args)]];
  }
  return [];
});

/** Keys that only prefix others, for invalidating a family. */
const PREFIXES = new Set(["streamKey", "storeFlowsKey", "storeBodiesKey", "decodedRootKey", "operationRootKey", "mcpAuditKey", "wsPagesKey", "ssePagesKey"]);

describe("query keys", () => {
  it("covers every key the module exports", () => {
    expect(SAMPLES.length).toBeGreaterThan(50);
  });

  it("gives each shape its own root: no two full keys share a root with a different length", () => {
    const byRoot = new Map<unknown, Map<number, string>>();
    for (const [name, key] of SAMPLES) {
      if (PREFIXES.has(name)) continue;
      // flowKey's summary and detail keys extend it on purpose (one invalidation refreshes all three).
      if (name === "flowSummaryKey" || name === "flowDetailKey") continue;
      const lengths = byRoot.get(key[0]) ?? new Map<number, string>();
      const clash = [...lengths].find(([len]) => len !== key.length);
      expect(clash, `${name} and ${clash?.[1]} share the root ${String(key[0])}`).toBeUndefined();
      lengths.set(key.length, name);
      byRoot.set(key[0], lengths);
    }
  });

  it("puts every prefix at the start of the keys it invalidates", () => {
    expect(q.flowSummaryKey(1, 2).slice(0, 3)).toEqual(q.flowKey(1, 2));
    expect(q.flowDetailKey(1, 2).slice(0, 3)).toEqual(q.flowKey(1, 2));
    expect(q.flowKey(1, 2).slice(0, 2)).toEqual(q.storeFlowsKey(1));
    expect(q.bodyKey(1, 2, "response", true, 10, true, null).slice(0, 2)).toEqual(q.storeBodiesKey(1));
    expect(q.sseHeadKey(1, 2, "x").slice(0, 3)).toEqual(q.streamKey(1, 2));
    expect(q.wsHeadKey(1, 2, { direction: null, text: "" }).slice(0, 3)).toEqual(q.streamKey(1, 2));
    expect(q.wsPagesKey(1, 2, { direction: null, text: "" }).slice(0, 3)).toEqual(q.wsPagesKey(1, 2));
    expect(q.decodedKey(1, 2, "response", 3, true)[0]).toBe(q.decodedRootKey[0]);
    expect(q.operationKey(1, 2)[0]).toBe(q.operationRootKey[0]);
    expect(q.mcpAuditPageKey(50)[0]).toBe(q.mcpAuditKey[0]);
  });

  it("each query option fetches under its own key", () => {
    expect(queries.proxyStatus().queryKey).toBe(q.proxyStatusKey);
    expect(queries.inspected(0, 3).queryKey).toEqual(q.flowKey(0, 3));
    expect(queries.inspected(0, null).enabled).toBe(false);
  });

  it("carries each key's data type", () => {
    expectTypeOf(q.queryClient.getQueryData(q.proxyStatusKey)).toEqualTypeOf<ProxyStatus_Serialize | undefined>();
    expectTypeOf(q.queryClient.getQueryData(q.rulesKey)).toEqualTypeOf<Rule_Serialize[] | undefined>();
    // @ts-expect-error: a key only takes its own type.
    q.queryClient.setQueryData(q.proxyStatusKey, { nope: true });
  });
});
