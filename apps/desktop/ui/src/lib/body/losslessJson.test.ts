import { describe, expect, it } from "vitest";

import { isJsonNumber, JsonSyntaxError, numberText, parseLossless, stringifyLossless, type JsonNumber } from "./losslessJson";

const num = (v: unknown) => {
  if (typeof v === "number") return numberText(v);
  if (isJsonNumber(v)) return numberText(v);
  throw new Error(`not a number: ${String(v)}`);
};

const error = (text: string): JsonSyntaxError => {
  try {
    parseLossless(text);
  } catch (e) {
    if (e instanceof JsonSyntaxError) return e;
    throw e;
  }
  throw new Error(`parsed: ${text}`);
};

describe("parseLossless", () => {
  it("parses like JSON.parse for ordinary documents", () => {
    const text = '{"a":[1,-2,0,true,false,null,"s"],"b":{"c":{}},"d":[],"e":"x\\ty"}';
    expect(parseLossless(text)).toEqual(JSON.parse(text));
    expect(parseLossless(" \t\n\r[ 1 , 2 ]\n")).toEqual([1, 2]);
    expect(parseLossless('"top"')).toBe("top");
    expect(parseLossless("null")).toBe(null);
  });

  it("keeps big integers exactly", () => {
    const v = parseLossless('{"id":12345678901234567890,"neg":-9223372036854775808,"safe":123456789012345}') as Record<string, unknown>;
    expect(isJsonNumber(v.id)).toBe(true);
    expect(num(v.id)).toBe("12345678901234567890");
    expect(num(v.neg)).toBe("-9223372036854775808");
    // Short integers stay plain numbers.
    expect(v.safe).toBe(123456789012345);
    expect(parseLossless("9007199254740993")).toSatisfy((n: unknown) => num(n) === "9007199254740993");
  });

  it("keeps decimals and exponents as written", () => {
    const v = parseLossless("[1.50, 1e400, -0, 0.1, 2E-7, 1e+2, 0]") as unknown[];
    expect(v.map(num)).toEqual(["1.50", "1e400", "-0", "0.1", "2E-7", "1e+2", "0"]);
    expect(v[6]).toBe(0);
    expect(isJsonNumber(v[0])).toBe(true);
  });

  it("decodes escapes, including unicode and surrogate pairs", () => {
    expect(parseLossless('"a\\"b\\\\c\\/d\\b\\f\\n\\r\\t"')).toBe('a"b\\c/d\b\f\n\r\t');
    expect(parseLossless('"\\u00e9\\u4E2D"')).toBe("é中");
    expect(parseLossless('"\\ud83d\\ude00 ok"')).toBe("😀 ok");
    expect(parseLossless('"raw é 中 😀"')).toBe("raw é 中 😀");
    expect(parseLossless('{"k\\u0041y":1}')).toEqual({ kAy: 1 });
  });

  it("handles nesting, deep nesting and a BOM", () => {
    expect(parseLossless('{"a":[{"b":[[],[{}],{"c":[1,[2,[3]]]}]}]}')).toEqual({ a: [{ b: [[], [{}], { c: [1, [2, [3]]] }] }] });
    const depth = 100_000;
    let deep = parseLossless("[".repeat(depth) + "]".repeat(depth));
    let levels = 0;
    while (Array.isArray(deep) && deep.length) {
      deep = deep[0];
      levels++;
    }
    expect(levels).toBe(depth - 1);
    expect(parseLossless('\uFEFF{"a":1}')).toEqual({ a: 1 });
  });

  it("makes __proto__ an own property and lets the last duplicate key win", () => {
    const v = parseLossless('{"__proto__":{"x":1},"a":1,"a":2}') as Record<string, unknown>;
    expect(Object.getPrototypeOf(v)).toBe(Object.prototype);
    expect(Object.keys(v)).toEqual(["__proto__", "a"]);
    expect(v.a).toBe(2);
  });

  it("reports where invalid JSON goes wrong", () => {
    const e = error('{\n  "a": 1,\n  "b": tru\n}');
    expect(e.line).toBe(3);
    expect(e.column).toBe(8);
    expect(e.position).toBe(19);
    expect(e.message).toBe("Expected a value but found 't' at line 3, column 8");

    expect(error("[1, 2").message).toBe("Expected ',' or ']' in an array but found end of input at line 1, column 6");
    expect(error('{"a" 1}').message).toBe("Expected ':' after a property name but found '1' at line 1, column 6");
    expect(error("{'a':1}").position).toBe(1);
    expect(error("[1,]").position).toBe(3);
    expect(error("[01]").message).toMatch(/^Leading zeros/);
    expect(error("[1.]").position).toBe(3);
    expect(error("[-]").position).toBe(2);
    expect(error('"abc').message).toBe("Unterminated string at line 1, column 1");
    expect(error('"a\nb"').position).toBe(2);
    expect(error('"\\x"').position).toBe(1);
    expect(error('"\\u12G4"').message).toMatch(/^Invalid \\u escape/);
    expect(error("{} x").message).toBe("Unexpected 'x' after the JSON value at line 1, column 4");
    expect(error("").message).toBe("Expected a value but found end of input at line 1, column 1");
  });

  it("survives structured clone (the worker hand-off)", () => {
    const v = structuredClone(parseLossless('{"id":12345678901234567890,"n":[1.50]}')) as { id: JsonNumber; n: JsonNumber[] };
    expect(isJsonNumber(v.id)).toBe(true);
    expect(numberText(v.id)).toBe("12345678901234567890");
    expect(numberText(v.n[0]!)).toBe("1.50");
  });
});

describe("stringifyLossless", () => {
  it("writes numbers exactly as parsed", () => {
    const text = '{"id":12345678901234567890,"price":1.50,"big":1e400,"list":[1,"two",null,true,{}],"empty":[]}';
    expect(stringifyLossless(parseLossless(text), 0)).toBe(text);
    expect(stringifyLossless(parseLossless(text))).toBe(
      [
        "{",
        '  "id": 12345678901234567890,',
        '  "price": 1.50,',
        '  "big": 1e400,',
        '  "list": [',
        "    1,",
        '    "two",',
        "    null,",
        "    true,",
        "    {}",
        "  ],",
        '  "empty": []',
        "}",
      ].join("\n"),
    );
  });

  it("matches JSON.stringify for ordinary values", () => {
    const v = { a: [1, 2.5, "x\ny"], b: { c: null, d: false } };
    expect(stringifyLossless(v)).toBe(JSON.stringify(v, null, 2));
    expect(stringifyLossless("s")).toBe('"s"');
  });
});
