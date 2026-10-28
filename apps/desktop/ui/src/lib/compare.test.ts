import { describe, expect, it } from "vitest";

import { changedCount, diffHeaders } from "./compare";

const h = (name: string, value: string) => ({ name, value });

describe("diffHeaders", () => {
  it("compares by name, whatever the order or case", () => {
    const rows = diffHeaders(
      [h("Accept", "*/*"), h("X-A", "1"), h("Date", "Mon"), h("Cookie", "a=1"), h("cookie", "b=2")],
      [h("date", "Tue"), h("accept", "*/*"), h("X-B", "2"), h("Cookie", "a=1, b=2")],
    );
    expect(rows.map((r) => [r.name, r.kind])).toEqual([
      ["Accept", "same"],
      ["X-A", "removed"],
      ["Date", "changed"],
      ["Cookie", "same"],
      ["X-B", "added"],
    ]);
    expect(changedCount(rows)).toBe(3);
  });

  it("can leave out volatile headers", () => {
    const rows = diffHeaders([h("date", "Mon"), h("x-request-id", "1"), h("a", "1")], [h("date", "Tue"), h("a", "2")], true);
    expect(rows.map((r) => r.name)).toEqual(["a"]);
  });
});
