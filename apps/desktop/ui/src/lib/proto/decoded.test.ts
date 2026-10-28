import { describe, expect, it } from "vitest";

import { badgeMap, mappingFor, preview, withMapping } from "./decoded";

describe("decoded bodies", () => {
  it("groups badges by pointer", () => {
    const map = badgeMap({
      badges: [
        { pointer: "/7", label: "key: int" },
        { pointer: "/7", label: "ext 5" },
        { pointer: "", label: "1 unknown field" },
      ],
    });
    expect(map.get("/7")).toEqual(["key: int", "ext 5"]);
    expect(map.get("")).toEqual(["1 unknown field"]);
  });

  it("previews messages on one line", () => {
    expect(preview('{\n  "a": 1,\n  "b": [\n    2\n  ]\n}')).toBe('{ "a": 1, "b": [ 2 ] }');
    expect(preview("x".repeat(200), 10)).toHaveLength(10);
  });

  it("sets, replaces and clears a flow's mapping", () => {
    const url = "https://api.example.com/v1/orders?x=1";
    let list = withMapping([], url, "response", "shop.v1.Order", "m1");
    expect(list).toHaveLength(1);
    expect(list[0]!.matcher.host?.value).toBe("api.example.com");
    expect(list[0]!.matcher.path?.value).toBe("/v1/orders");
    list = withMapping(list, url, "response", "shop.v1.LineItem", "m2");
    expect(list.map((m) => [m.id, m.message_type])).toEqual([["m1", "shop.v1.LineItem"]]);
    expect(mappingFor(list, url, "request")).toBeUndefined();
    expect(withMapping(list, url, "response", null, "m3")).toEqual([]);
  });
});
