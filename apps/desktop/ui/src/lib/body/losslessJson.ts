/**
 * A JSON parser for the JSON tree that keeps every number literal exactly as
 * written. `JSON.parse` rounds 64-bit ids (12345678901234567890 becomes
 * 12345678901234567000), drops trailing zeros (`1.50`) and turns `1e400`
 * into Infinity; a debugging proxy must show what was on the wire.
 *
 * Values are plain JS: objects, arrays, strings, booleans and null as usual.
 * A number whose text a JS number reproduces exactly (an integer of up to 15
 * digits, by far the common case) stays a `number`; any other becomes a
 * {@link JsonNumber} holding the literal. JsonNumbers are String wrapper
 * objects because structured clone keeps those, so a tree parsed in the
 * worker arrives on the main thread without a revival pass.
 */

declare const jsonNumberBrand: unique symbol;

/** A number literal kept as its exact source text (see the module doc). */
// eslint-disable-next-line @typescript-eslint/no-wrapper-object-types -- a String object survives structured clone; see above.
export type JsonNumber = String & { readonly [jsonNumberBrand]: true };

export function isJsonNumber(v: unknown): v is JsonNumber {
  return v instanceof String;
}

/** A number literal as a value: a JS number if that is exact, else a JsonNumber. */
export function jsonNumber(raw: string): number | JsonNumber {
  // Up to 15 digits, no fraction or exponent: the double is exact and prints back the same.
  const digits = raw.charCodeAt(0) === 45 ? raw.length - 1 : raw.length;
  if (digits <= 15 && raw !== "-0" && /^-?\d+$/.test(raw)) return Number(raw);
  return new String(raw) as JsonNumber;
}

/** The exact text of a JSON number value. */
export function numberText(v: number | JsonNumber): string {
  return typeof v === "number" ? String(v) : v.valueOf();
}

/** Invalid JSON, with where it went wrong (0-based offset, 1-based line and column). */
export class JsonSyntaxError extends SyntaxError {
  readonly position: number;
  readonly line: number;
  readonly column: number;

  constructor(message: string, position: number, line: number, column: number) {
    super(`${message} at line ${line}, column ${column}`);
    this.name = "JsonSyntaxError";
    this.position = position;
    this.line = line;
    this.column = column;
  }
}

// Character codes.
const C_TAB = 9;
const C_LF = 10;
const C_CR = 13;
const C_SPACE = 32;
const C_QUOTE = 34;
const C_PLUS = 43;
const C_COMMA = 44;
const C_MINUS = 45;
const C_DOT = 46;
const C_ZERO = 48;
const C_NINE = 57;
const C_COLON = 58;
const C_UPPER_E = 69;
const C_L_BRACKET = 91;
const C_BACKSLASH = 92;
const C_R_BRACKET = 93;
const C_LOWER_E = 101;
const C_L_BRACE = 123;
const C_R_BRACE = 125;

const ESCAPES: Record<number, string> = { 34: '"', 92: "\\", 47: "/", 98: "\b", 102: "\f", 110: "\n", 114: "\r", 116: "\t" };

interface Frame {
  array: unknown[] | null;
  object: Record<string, unknown> | null;
  key: string;
}

/**
 * Parses JSON text losslessly (see the module doc). A leading BOM is
 * ignored. Throws a {@link JsonSyntaxError} with the position of the
 * problem. Iterative, so deep nesting can't overflow the stack.
 */
export function parseLossless(text: string): unknown {
  const n = text.length;
  let i = text.charCodeAt(0) === 0xfeff ? 1 : 0;

  const fail = (message: string, at = i): never => {
    let line = 1;
    let lineStart = 0;
    for (let j = 0; j < at && j < n; j++) {
      if (text.charCodeAt(j) === C_LF) {
        line++;
        lineStart = j + 1;
      }
    }
    throw new JsonSyntaxError(message, at, line, at - lineStart + 1);
  };
  const found = (at = i) => (at >= n ? "end of input" : `'${String.fromCodePoint(text.codePointAt(at)!)}'`);
  const unexpected = (what: string): never => fail(`Expected ${what} but found ${found()}`);

  const skipWs = () => {
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c !== C_SPACE && c !== C_LF && c !== C_CR && c !== C_TAB) return;
      i++;
    }
  };

  /** A string; `i` is on its opening quote. */
  const string = (): string => {
    const open = i;
    let start = ++i;
    // Fast path: no escapes, one slice.
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === C_QUOTE) return text.slice(start, i++);
      if (c === C_BACKSLASH) break;
      if (c < 0x20) fail("Unescaped control character in string");
      i++;
    }
    let out = "";
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === C_QUOTE) {
        out += text.slice(start, i++);
        return out;
      }
      if (c < 0x20) fail("Unescaped control character in string");
      if (c !== C_BACKSLASH) {
        i++;
        continue;
      }
      out += text.slice(start, i);
      const e = text.charCodeAt(i + 1);
      if (e === 117 /* u */) {
        const hex = text.slice(i + 2, i + 6);
        if (!/^[0-9a-fA-F]{4}$/.test(hex)) fail("Invalid \\u escape");
        out += String.fromCharCode(parseInt(hex, 16));
        i += 6;
      } else {
        const ch = ESCAPES[e];
        if (ch === undefined) fail(i + 1 >= n ? "Unterminated string" : "Invalid escape", i);
        out += ch;
        i += 2;
      }
      start = i;
    }
    return fail("Unterminated string", open);
  };

  const digits = () => {
    const start = i;
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c < C_ZERO || c > C_NINE) break;
      i++;
    }
    if (i === start) unexpected("a digit");
  };

  /** A number; `i` is on its `-` or first digit. */
  const number = (): number | JsonNumber => {
    const start = i;
    if (text.charCodeAt(i) === C_MINUS) i++;
    if (text.charCodeAt(i) === C_ZERO) {
      i++;
      const c = text.charCodeAt(i);
      if (c >= C_ZERO && c <= C_NINE) fail("Leading zeros are not allowed");
    } else digits();
    if (text.charCodeAt(i) === C_DOT) {
      i++;
      digits();
    }
    const e = text.charCodeAt(i);
    if (e === C_LOWER_E || e === C_UPPER_E) {
      i++;
      const sign = text.charCodeAt(i);
      if (sign === C_PLUS || sign === C_MINUS) i++;
      digits();
    }
    return jsonNumber(text.slice(start, i));
  };

  const literal = (word: string, value: unknown) => {
    if (!text.startsWith(word, i)) unexpected("a value");
    i += word.length;
    return value;
  };

  /** `"key":` inside an object, leaving `i` after the colon. */
  const key = (): string => {
    skipWs();
    if (text.charCodeAt(i) !== C_QUOTE) unexpected("a property name in double quotes");
    const k = string();
    skipWs();
    if (text.charCodeAt(i) !== C_COLON) unexpected("':' after a property name");
    i++;
    return k;
  };

  const stack: Frame[] = [];
  for (;;) {
    // One value, or the opening of a non-empty container.
    skipWs();
    let value: unknown;
    const c = text.charCodeAt(i);
    if (c === C_L_BRACE) {
      i++;
      skipWs();
      if (text.charCodeAt(i) === C_R_BRACE) {
        i++;
        value = {};
      } else {
        stack.push({ array: null, object: {}, key: key() });
        continue;
      }
    } else if (c === C_L_BRACKET) {
      i++;
      skipWs();
      if (text.charCodeAt(i) === C_R_BRACKET) {
        i++;
        value = [];
      } else {
        stack.push({ array: [], object: null, key: "" });
        continue;
      }
    } else if (c === C_QUOTE) value = string();
    else if (c === C_MINUS || (c >= C_ZERO && c <= C_NINE)) value = number();
    else if (c === 116 /* t */) value = literal("true", true);
    else if (c === 102 /* f */) value = literal("false", false);
    else if (c === 110 /* n */) value = literal("null", null);
    else unexpected("a value");

    // Hand the value to its container, closing containers as they end.
    for (;;) {
      const top = stack[stack.length - 1];
      if (!top) {
        skipWs();
        if (i < n) fail(`Unexpected ${found()} after the JSON value`);
        return value;
      }
      if (top.array) top.array.push(value);
      else if (top.key === "__proto__") {
        // Plain assignment would set the prototype; JSON.parse makes an own property.
        Object.defineProperty(top.object, top.key, { value, writable: true, enumerable: true, configurable: true });
      } else top.object![top.key] = value;
      skipWs();
      const d = text.charCodeAt(i);
      if (d === C_COMMA) {
        i++;
        if (top.object) top.key = key();
        break;
      }
      if (d === (top.array ? C_R_BRACKET : C_R_BRACE)) {
        i++;
        value = top.array ?? top.object;
        stack.pop();
        continue;
      }
      unexpected(top.array ? "',' or ']' in an array" : "',' or '}' in an object");
    }
  }
}

/**
 * JSON text for a value from {@link parseLossless}, numbers exactly as
 * they were written. `indent` as in `JSON.stringify`.
 */
export function stringifyLossless(value: unknown, indent = 2): string {
  const pad = " ".repeat(indent);
  const out: string[] = [];
  const write = (v: unknown, prefix: string) => {
    if (v === null || typeof v === "boolean") out.push(String(v));
    else if (typeof v === "number") out.push(Number.isFinite(v) ? String(v) : "null");
    else if (typeof v === "string") out.push(JSON.stringify(v));
    else if (isJsonNumber(v)) out.push(v.valueOf());
    else if (Array.isArray(v)) {
      if (v.length === 0) return void out.push("[]");
      const inner = prefix + pad;
      out.push("[");
      v.forEach((item, i) => {
        out.push(i === 0 ? "" : ",", indent ? "\n" + inner : "");
        write(item === undefined ? null : item, inner);
      });
      out.push(indent ? "\n" + prefix : "", "]");
    } else if (typeof v === "object") {
      const keys = Object.keys(v);
      if (keys.length === 0) return void out.push("{}");
      const inner = prefix + pad;
      out.push("{");
      keys.forEach((k, i) => {
        out.push(i === 0 ? "" : ",", indent ? "\n" + inner : "", JSON.stringify(k), indent ? ": " : ":");
        write((v as Record<string, unknown>)[k], inner);
      });
      out.push(indent ? "\n" + prefix : "", "}");
    } else out.push("null");
  };
  write(value, "");
  return out.join("");
}
