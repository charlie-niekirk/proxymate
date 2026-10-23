// Proxymate scripts: the API. A script is an ES module that exports the
// entry points of its kind. Scripts run in an embedded QuickJS with no
// network, files, `require`, `import` of other modules or timers.
//
//   Rewrite scripts: onRequest and/or onResponse (run by a rule's Script action)
//   Mock scripts:    onMock (run by a rule's Script action)
//   View scripts:    onView (run when the inspector shows a matching body)
//
// Limits: onRequest, onResponse and onMock get 1 s, onView 3 s (Settings ›
// Advanced); 256 MB of memory per worker. A script that throws or runs out
// of time is skipped: the flow goes on unchanged and the error shows in the
// inspector and the script console.

/** Header and query values: a string, or a list for a repeated name. */
type Values = string | string[];

/** What a body is: parsed JSON (integers beyond 2^53 as BigInt), a form's
 * fields, text, or null for binary. */
type Body = any;

interface ProxymateMessage {
  /** Lower-case names. A repeated header is a `string[]`, in order. */
  headers: Record<string, Values>;
  /**
   * The decoded body, parsed on first read: JSON (by Content-Type, or text
   * that starts with `{` or `[`) as objects with BigInt for big integers,
   * `application/x-www-form-urlencoded` as an object, other UTF-8 as a
   * string, binary as `null`. Change it in place or assign a new value.
   */
  body: Body;
  /** The decoded body as UTF-8 text. */
  text: string | null;
  /** The decoded body's bytes. */
  rawBody: Uint8Array | null;
  /** True when the body isn't available to scripts (over 16 MB, or an event
   * stream); `body`, `text` and `rawBody` are then null and can't be set. */
  readonly bodyTooLarge: boolean;
}

interface ProxymateRequest extends ProxymateMessage {
  /** Upper-case, e.g. `"POST"`. */
  method: string;
  /** The absolute URL. Setting it replaces scheme, host, port, path and query. */
  url: string;
  /** `"http"` or `"https"`. */
  scheme: string;
  host: string;
  port: number;
  /** Without the query, e.g. `/v1/items`. */
  path: string;
  /** Decoded query parameters. */
  query: Record<string, Values>;
}

interface ProxymateResponse extends ProxymateMessage {
  status: number;
}

interface ProxymateContext {
  readonly scriptId: string;
  readonly scriptName: string;
  /** The flow, when it is recorded. */
  readonly flowId: number | null;
  readonly phase: "request" | "response" | "mock" | "view";
  /** The rule whose action runs the script (rewrite and mock scripts). */
  readonly rule: { readonly id: string; readonly name: string } | null;
  /** View scripts: which body is being viewed. */
  readonly part: "request" | "response" | null;
  /**
   * Kept between runs of this script on its worker. It starts over when the
   * script is saved, and when the script engine restarts after a timeout or
   * running out of memory.
   */
  readonly shared: Record<string, any>;
}

interface ProxymateFlow {
  readonly id: number | null;
  readonly part: "request" | "response" | null;
  /** Read-only. */
  readonly request: Readonly<ProxymateRequest>;
  /** Read-only; null before the response arrives. */
  readonly response: Readonly<ProxymateResponse> | null;
}

/** A mock answer. `body` objects are sent as JSON. */
interface ProxymateMockResponse {
  status?: number;
  headers?: Record<string, Values>;
  body?: Body;
  text?: string;
  rawBody?: Uint8Array;
}

/** One extra inspector view. */
interface ProxymateView {
  title?: string;
  /** Defaults from the body: objects `json`, strings `text`, bytes `hex`. */
  kind?: "json" | "text" | "xml" | "html" | "hex" | "image";
  body?: any;
  text?: string;
  rawBody?: Uint8Array;
  /** For `image`, e.g. `image/png`. */
  mime?: string;
}

/** Rewrite scripts: change the request before it is sent. Change `request`
 * in place (or return a new one); an untouched request is sent byte for byte. */
declare function onRequest(
  context: ProxymateContext,
  url: string,
  request: ProxymateRequest,
): ProxymateRequest | void | Promise<ProxymateRequest | void>;

/** Rewrite scripts: change the response before the client gets it. */
declare function onResponse(
  context: ProxymateContext,
  url: string,
  request: Readonly<ProxymateRequest>,
  response: ProxymateResponse,
): ProxymateResponse | void | Promise<ProxymateResponse | void>;

/** Mock scripts: answer without contacting the upstream. Return null (or
 * nothing) to let the request go on. A string is sent as text, bytes as
 * binary, any other object without status/headers/body as JSON. */
declare function onMock(
  context: ProxymateContext,
  url: string,
  request: Readonly<ProxymateRequest>,
): ProxymateMockResponse | string | Uint8Array | object | null | void | Promise<any>;

/** View scripts: extra views of a body, shown in the inspector's viewer menu. */
declare function onView(
  context: ProxymateContext,
  flow: ProxymateFlow,
): ProxymateView | ProxymateView[] | null | void | Promise<ProxymateView | ProxymateView[] | null | void>;

declare const Proxymate: {
  /** Standard or URL-safe base64, padded or not. */
  base64Decode(text: string): Uint8Array;
  base64DecodeText(text: string): string;
  /** Parses the decoded text as JSON, keeping big integers as BigInt. */
  base64DecodeJson(text: string): any;
  base64Encode(data: string | Uint8Array, options?: { urlSafe?: boolean }): string;
  /** Undoes a Content-Encoding. */
  decompress(data: Uint8Array, encoding: "gzip" | "deflate" | "br" | "zstd"): Uint8Array;
  /** JSON.parse that keeps integers beyond 2^53 exact, as BigInt. */
  parseJson(text: string): any;
  /** JSON.stringify that writes BigInt as plain numbers. */
  stringifyJson(value: any, space?: number | string): string;
};

type DigestEncoding = "hex" | "base64" | "bytes";

declare const crypto: {
  /** Hex by default. */
  sha1(data: string | Uint8Array, encoding?: DigestEncoding): string;
  sha256(data: string | Uint8Array, encoding?: DigestEncoding): string;
  sha512(data: string | Uint8Array, encoding?: DigestEncoding): string;
  hmac(
    algorithm: "sha1" | "sha256" | "sha384" | "sha512",
    key: string | Uint8Array,
    data: string | Uint8Array,
    encoding?: DigestEncoding,
  ): string;
  randomUUID(): string;
};

/** Writes to the script console (Scripts › Console). */
declare const console: {
  debug(...args: any[]): void;
  log(...args: any[]): void;
  info(...args: any[]): void;
  warn(...args: any[]): void;
  error(...args: any[]): void;
  trace(...args: any[]): void;
};

declare function atob(data: string): string;
declare function btoa(data: string): string;

declare class TextEncoder {
  readonly encoding: "utf-8";
  encode(input?: string): Uint8Array;
}

declare class TextDecoder {
  constructor(label?: "utf-8" | "utf8", options?: { fatal?: boolean; ignoreBOM?: boolean });
  readonly encoding: "utf-8";
  readonly fatal: boolean;
  readonly ignoreBOM: boolean;
  decode(input?: Uint8Array | ArrayBuffer | ArrayBufferView): string;
}

declare class URLSearchParams {
  constructor(init?: string | Record<string, string> | Iterable<[string, string]> | URLSearchParams);
  readonly size: number;
  append(name: string, value: string): void;
  delete(name: string, value?: string): void;
  get(name: string): string | null;
  getAll(name: string): string[];
  has(name: string, value?: string): boolean;
  set(name: string, value: string): void;
  sort(): void;
  forEach(callback: (value: string, key: string, parent: URLSearchParams) => void, thisArg?: any): void;
  keys(): IterableIterator<string>;
  values(): IterableIterator<string>;
  entries(): IterableIterator<[string, string]>;
  [Symbol.iterator](): IterableIterator<[string, string]>;
  toString(): string;
}

/** WHATWG URL parsing. */
declare class URL {
  constructor(url: string, base?: string);
  static canParse(url: string, base?: string): boolean;
  href: string;
  readonly origin: string;
  protocol: string;
  username: string;
  password: string;
  host: string;
  hostname: string;
  port: string;
  pathname: string;
  search: string;
  hash: string;
  readonly searchParams: URLSearchParams;
  toString(): string;
  toJSON(): string;
}

declare class DOMException extends Error {
  constructor(message?: string, name?: string);
}

declare const performance: {
  /** Milliseconds, for timing a script. */
  now(): number;
};
