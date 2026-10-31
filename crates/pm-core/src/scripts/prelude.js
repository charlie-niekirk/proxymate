// The JS half of the script API (`proxymate.d.ts` documents it). Evaluated
// once per worker context as a function of the Rust built-ins (`native`);
// it installs the globals and returns the internals the worker calls.
// Nothing internal is left on `globalThis`.
(function (native) {
  "use strict";
  const N = native;
  const internal = Symbol("proxymate");

  // ---- Bytes and text ----

  function toBytes(data) {
    if (data instanceof Uint8Array) return data;
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (data === undefined || data === null) return new Uint8Array(0);
    return N.utf8Encode(String(data));
  }

  class TextEncoder {
    get encoding() {
      return "utf-8";
    }
    encode(input = "") {
      return N.utf8Encode(String(input));
    }
  }

  class TextDecoder {
    constructor(label = "utf-8", options = {}) {
      const name = String(label).trim().toLowerCase();
      if (name !== "utf-8" && name !== "utf8" && name !== "unicode-1-1-utf-8") {
        throw new RangeError(`TextDecoder: only utf-8 is supported, not ${label}`);
      }
      this.fatal = Boolean(options && options.fatal);
      this.ignoreBOM = Boolean(options && options.ignoreBOM);
    }
    get encoding() {
      return "utf-8";
    }
    decode(input) {
      if (input === undefined) return "";
      return N.utf8Decode(toBytes(input), this.fatal, this.ignoreBOM);
    }
  }

  // ---- JSON that keeps integers beyond 2^53 exact (as BigInt) ----

  const INTEGER = /^-?\d+$/;

  function parseJson(text) {
    return JSON.parse(text, (key, value, context) =>
      typeof value === "number" &&
      !Number.isSafeInteger(value) &&
      context &&
      typeof context.source === "string" &&
      INTEGER.test(context.source)
        ? BigInt(context.source)
        : value,
    );
  }

  function stringifyJson(value, space) {
    return JSON.stringify(value, (key, v) => (typeof v === "bigint" ? JSON.rawJSON(v.toString()) : v), space);
  }

  // ---- URLSearchParams (application/x-www-form-urlencoded) ----

  function formDecode(s) {
    const plus = s.replace(/\+/g, " ");
    try {
      return decodeURIComponent(plus);
    } catch {
      return N.percentDecode(plus);
    }
  }

  function formEncode(s) {
    return encodeURIComponent(s)
      .replace(/%20/g, "+")
      .replace(/[!'()~]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase());
  }

  function parsePairs(query) {
    const pairs = [];
    for (const part of query.split("&")) {
      if (!part) continue;
      const eq = part.indexOf("=");
      pairs.push(eq < 0 ? [formDecode(part), ""] : [formDecode(part.slice(0, eq)), formDecode(part.slice(eq + 1))]);
    }
    return pairs;
  }

  class URLSearchParams {
    constructor(init = "") {
      this[internal] = { list: [], onChange: null };
      const list = this[internal].list;
      if (init instanceof URLSearchParams) {
        list.push(...init[internal].list.map(([k, v]) => [k, v]));
      } else if (typeof init === "object" && init !== null) {
        if (typeof init[Symbol.iterator] === "function") {
          for (const pair of init) {
            const [k, v] = [...pair];
            list.push([String(k), String(v)]);
          }
        } else {
          for (const k of Object.keys(init)) list.push([k, String(init[k])]);
        }
      } else {
        const s = String(init);
        list.push(...parsePairs(s.startsWith("?") ? s.slice(1) : s));
      }
    }
    _changed() {
      const onChange = this[internal].onChange;
      if (onChange) onChange(this.toString());
    }
    get size() {
      return this[internal].list.length;
    }
    append(name, value) {
      this[internal].list.push([String(name), String(value)]);
      this._changed();
    }
    delete(name, value) {
      const n = String(name);
      this[internal].list = this[internal].list.filter(
        ([k, v]) => !(k === n && (value === undefined || v === String(value))),
      );
      this._changed();
    }
    get(name) {
      const hit = this[internal].list.find(([k]) => k === String(name));
      return hit ? hit[1] : null;
    }
    getAll(name) {
      return this[internal].list.filter(([k]) => k === String(name)).map(([, v]) => v);
    }
    has(name, value) {
      return this[internal].list.some(([k, v]) => k === String(name) && (value === undefined || v === String(value)));
    }
    set(name, value) {
      const n = String(name);
      const list = this[internal].list;
      const first = list.findIndex(([k]) => k === n);
      if (first < 0) {
        list.push([n, String(value)]);
      } else {
        list[first] = [n, String(value)];
        this[internal].list = list.filter(([k], i) => k !== n || i === first);
      }
      this._changed();
    }
    sort() {
      this[internal].list.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
      this._changed();
    }
    forEach(callback, thisArg) {
      for (const [k, v] of this[internal].list) callback.call(thisArg, v, k, this);
    }
    *keys() {
      for (const [k] of this[internal].list) yield k;
    }
    *values() {
      for (const [, v] of this[internal].list) yield v;
    }
    *entries() {
      for (const [k, v] of this[internal].list) yield [k, v];
    }
    [Symbol.iterator]() {
      return this.entries();
    }
    toString() {
      return this[internal].list.map(([k, v]) => formEncode(k) + "=" + formEncode(v)).join("&");
    }
  }

  // ---- URL (parsed by the Rust `url` crate, WHATWG rules) ----

  const URL_FIELDS = ["protocol", "username", "password", "host", "hostname", "port", "pathname", "search", "hash"];

  class URL {
    constructor(url, base) {
      const parts = base === undefined ? N.parseUrl(String(url)) : N.parseUrl(String(url), String(base));
      this[internal] = { parts, params: null };
    }
    static canParse(url, base) {
      try {
        new URL(url, base);
        return true;
      } catch {
        return false;
      }
    }
    get href() {
      return this[internal].parts.href;
    }
    set href(value) {
      this[internal].parts = N.parseUrl(String(value));
      this._syncParams();
    }
    get origin() {
      return this[internal].parts.origin;
    }
    get searchParams() {
      const state = this[internal];
      if (!state.params) {
        state.params = new URLSearchParams(state.parts.search);
        state.params[internal].onChange = (search) => {
          state.parts = N.setUrl(state.parts.href, "search", search);
        };
      }
      return state.params;
    }
    _syncParams() {
      const state = this[internal];
      if (state.params) state.params[internal].list = parsePairs(state.parts.search.replace(/^\?/, ""));
    }
    toString() {
      return this.href;
    }
    toJSON() {
      return this.href;
    }
  }
  for (const field of URL_FIELDS) {
    Object.defineProperty(URL.prototype, field, {
      get() {
        return this[internal].parts[field];
      },
      set(value) {
        this[internal].parts = N.setUrl(this[internal].parts.href, field, String(value));
        if (field === "search") this._syncParams();
      },
      enumerable: true,
    });
  }

  // ---- console ----

  const CONSOLE_MAX = 16 * 1024;

  function inspect(value, depth) {
    if (typeof value === "string") return depth === 0 ? value : JSON.stringify(value);
    if (typeof value === "bigint") return value.toString() + "n";
    if (typeof value === "function") return `[Function ${value.name || "anonymous"}]`;
    if (typeof value === "symbol") return value.toString();
    if (value === undefined) return "undefined";
    if (value instanceof Error) return value.stack ? `${value.name}: ${value.message}\n${value.stack}` : String(value);
    if (value instanceof Uint8Array) {
      const head = Array.from(value.subarray(0, 32)).join(", ");
      return `Uint8Array(${value.length}) [${head}${value.length > 32 ? ", …" : ""}]`;
    }
    if (value instanceof URL || value instanceof URLSearchParams) return `${value.constructor.name} { ${value} }`;
    try {
      const text = stringifyJson(value, 2);
      return text === undefined ? String(value) : text;
    } catch {
      return String(value);
    }
  }

  function log(level) {
    return (...args) => {
      let text = args.map((a) => inspect(a, 0)).join(" ");
      if (text.length > CONSOLE_MAX) text = text.slice(0, CONSOLE_MAX) + "…";
      N.console(level, text);
    };
  }

  const console = {
    debug: log("debug"),
    log: log("log"),
    info: log("info"),
    warn: log("warn"),
    error: log("error"),
    trace: log("debug"),
  };

  // ---- crypto ----

  function encode(bytes, encoding) {
    if (encoding === "bytes") return bytes;
    if (encoding === "base64") return N.base64Encode(bytes, false);
    if (encoding === undefined || encoding === "hex") {
      let out = "";
      for (const b of bytes) out += (b < 16 ? "0" : "") + b.toString(16);
      return out;
    }
    throw new TypeError(`unknown encoding ${encoding}; use "hex", "base64" or "bytes"`);
  }

  const crypto = {
    sha1: (data, encoding) => encode(N.digest("sha1", toBytes(data)), encoding),
    sha256: (data, encoding) => encode(N.digest("sha256", toBytes(data)), encoding),
    sha512: (data, encoding) => encode(N.digest("sha512", toBytes(data)), encoding),
    hmac: (algorithm, key, data, encoding) =>
      encode(N.hmac(String(algorithm), toBytes(key), toBytes(data)), encoding),
    randomUUID: () => N.randomUUID(),
  };

  // ---- Proxymate helpers ----

  const Proxymate = {
    base64Decode: (text) => N.base64Decode(String(text)),
    base64DecodeText: (text) => N.utf8Decode(N.base64Decode(String(text)), false, false),
    base64DecodeJson: (text) => parseJson(N.utf8Decode(N.base64Decode(String(text)), false, false)),
    base64Encode: (data, options) => N.base64Encode(toBytes(data), Boolean(options && options.urlSafe)),
    decompress: (data, encoding) => N.decompress(toBytes(data), String(encoding)),
    parseJson: (text) => parseJson(String(text)),
    stringifyJson: (value, space) => stringifyJson(value, space),
  };

  // ---- Messages ----

  const state = new WeakMap();

  function headerObject(list) {
    const out = {};
    for (const [name, value] of list) {
      const key = name.toLowerCase();
      if (!(key in out)) out[key] = value;
      else if (Array.isArray(out[key])) out[key].push(value);
      else out[key] = [out[key], value];
    }
    return out;
  }

  function headerList(object) {
    const list = [];
    if (!object || typeof object !== "object") return list;
    for (const key of Object.keys(object)) {
      const value = object[key];
      if (value === undefined || value === null) continue;
      for (const v of Array.isArray(value) ? value : [value]) list.push([key.toLowerCase(), String(v)]);
    }
    return list;
  }

  // A query object's pairs, in key order, repeated keys expanded.
  function pairList(object) {
    const list = [];
    if (!object || typeof object !== "object") return list;
    for (const key of Object.keys(object)) {
      const value = object[key];
      if (value === undefined || value === null) continue;
      for (const v of Array.isArray(value) ? value : [value]) list.push([key, String(v)]);
    }
    return list;
  }

  function sameList(a, b) {
    return a.length === b.length && a.every(([k, v], i) => b[i][0] === k && b[i][1] === v);
  }

  function contentType(headers) {
    const value = headers && typeof headers === "object" ? headers["content-type"] : undefined;
    return String(Array.isArray(value) ? value[0] : value || "").toLowerCase();
  }

  function isJsonType(ct) {
    return /[/+]json\b/.test(ct) || ct.startsWith("application/json");
  }

  function isFormType(ct) {
    return ct.startsWith("application/x-www-form-urlencoded");
  }

  function formObject(text) {
    const out = {};
    for (const [k, v] of parsePairs(text)) {
      if (!(k in out)) out[k] = v;
      else if (Array.isArray(out[k])) out[k].push(v);
      else out[k] = [out[k], v];
    }
    return out;
  }

  function formText(object) {
    const params = new URLSearchParams();
    for (const key of Object.keys(object)) {
      const value = object[key];
      if (value === undefined || value === null) continue;
      for (const v of Array.isArray(value) ? value : [value]) params.append(key, v);
    }
    return params.toString();
  }

  function textOf(bytes, fatal) {
    return N.utf8Decode(bytes, fatal, false);
  }

  // What `body` is for these bytes: parsed JSON, a form object, text, or
  // null for binary.
  function parseBody(bytes, ct) {
    if (bytes.length === 0) return isJsonType(ct) ? null : "";
    let text;
    try {
      text = textOf(bytes, true);
    } catch {
      return null;
    }
    if (isFormType(ct)) return formObject(text);
    const head = text.trimStart()[0];
    if (isJsonType(ct) || ((head === "{" || head === "[") && !/^text\/|xml|html/.test(ct))) {
      try {
        return parseJson(text);
      } catch {
        return text;
      }
    }
    return text;
  }

  // The canonical text of a parsed body, to tell whether it was changed.
  function canonical(value, ct) {
    if (value === null || typeof value !== "object") return undefined;
    if (isFormType(ct)) return formText(value);
    return stringifyJson(value);
  }

  // Bytes for a body a script assigned.
  function bodyBytes(value, ct) {
    if (value === undefined || value === null) return new Uint8Array(0);
    if (value instanceof Uint8Array || ArrayBuffer.isView(value) || value instanceof ArrayBuffer) {
      return new Uint8Array(toBytes(value));
    }
    if (typeof value === "string") return N.utf8Encode(value);
    if (typeof value === "object" && isFormType(ct)) return N.utf8Encode(formText(value));
    return N.utf8Encode(stringifyJson(value));
  }

  function bodyAccessors(msg, s, readonly) {
    const ct = () => contentType(readonly ? s.originalHeaders : msg.headers);
    const current = () => (s.set ? bodyBytes(s.set.value, ct()) : s.bytes);
    Object.defineProperty(msg, "body", {
      enumerable: true,
      get() {
        if (s.set && s.set.kind === "body") return s.set.value;
        if (s.tooLarge) return null;
        if (!s.parsed) {
          const value = parseBody(current(), ct());
          s.parsed = { value, canonical: s.set ? undefined : canonical(value, ct()) };
        }
        return s.parsed.value;
      },
      set(value) {
        if (readonly) return;
        s.set = { kind: "body", value };
        s.parsed = null;
        s.raw = null;
      },
    });
    Object.defineProperty(msg, "text", {
      enumerable: true,
      get() {
        if (s.set && s.set.kind === "text") return s.set.value;
        if (s.tooLarge) return null;
        return textOf(current(), false);
      },
      set(value) {
        if (readonly) return;
        s.set = { kind: "text", value: value === null || value === undefined ? "" : String(value) };
        s.parsed = null;
        s.raw = null;
      },
    });
    Object.defineProperty(msg, "rawBody", {
      enumerable: true,
      get() {
        if (s.set && s.set.kind === "raw") return s.set.value;
        if (s.tooLarge) return null;
        if (!s.raw) s.raw = new Uint8Array(current());
        return s.raw;
      },
      set(value) {
        if (readonly) return;
        s.set = { kind: "raw", value: value === null || value === undefined ? new Uint8Array(0) : toBytes(value) };
        s.parsed = null;
        s.raw = null;
      },
    });
  }

  function makeMessage(raw, isRequest, readonly) {
    const msg = {};
    const originalHeaders = headerObject(raw.headers);
    const s = {
      source: raw,
      bytes: raw.body || new Uint8Array(0),
      tooLarge: Boolean(raw.bodyTooLarge),
      originalHeaders,
      headerList: headerList(originalHeaders),
      set: null,
      parsed: null,
      raw: null,
    };
    if (isRequest) {
      const query = formObject(raw.query);
      s.queryList = pairList(query);
      msg.method = raw.method;
      msg.url = raw.url;
      msg.scheme = raw.scheme;
      msg.host = raw.host;
      msg.port = raw.port;
      msg.path = raw.path;
      msg.query = query;
    } else {
      msg.status = raw.status;
    }
    msg.headers = readonly ? originalHeaders : headerObject(raw.headers);
    msg.bodyTooLarge = s.tooLarge;
    bodyAccessors(msg, s, readonly);
    state.set(msg, s);
    if (readonly) Object.freeze(msg);
    return msg;
  }

  // A WebSocket message (M19): `direction`, `type` and the payload as
  // `body`, `text` and `rawBody`, parsed like a body without headers.
  function makeWsMessage(raw) {
    const msg = {};
    const s = {
      source: raw,
      bytes: raw.body || new Uint8Array(0),
      tooLarge: false,
      originalHeaders: {},
      headerList: [],
      set: null,
      parsed: null,
      raw: null,
    };
    Object.defineProperty(msg, "direction", { value: raw.direction, enumerable: true });
    msg.type = raw.type;
    bodyAccessors(msg, s, false);
    state.set(msg, s);
    return msg;
  }

  function wsType(value) {
    const type = String(value);
    if (type !== "text" && type !== "binary") throw new TypeError(`${value} isn't a message type: use "text" or "binary"`);
    return type;
  }

  // What onWsMessage left: null drops the message; the message itself (or
  // nothing) keeps it with any changes; a string, bytes or another object
  // replaces it.
  function finishWs(msg, value) {
    if (value === null) return null;
    if (value === undefined || value === msg) {
      const s = state.get(msg);
      const type = wsType(msg.type);
      const body = finishBody(msg, s);
      if (body === null && type === s.source.type) return { type, unchanged: true };
      return { type, body };
    }
    if (typeof value === "string") return { type: "text", body: N.utf8Encode(value) };
    if (value instanceof Uint8Array || ArrayBuffer.isView(value) || value instanceof ArrayBuffer) {
      return { type: "binary", body: new Uint8Array(toBytes(value)) };
    }
    if (typeof value !== "object") throw new TypeError("onWsMessage must return the message, a replacement or null");
    if (state.has(value)) return finishWs(value, undefined);
    const known = ["type", "body", "text", "rawBody"].some((k) => k in value);
    if (!known) return { type: "text", body: N.utf8Encode(stringifyJson(value)) };
    let body;
    if ("rawBody" in value) body = bodyBytes(value.rawBody, "");
    else if ("text" in value) body = N.utf8Encode(String(value.text ?? ""));
    else if ("body" in value) body = bodyBytes(value.body, "application/json");
    else body = null;
    const binary = "rawBody" in value && !("type" in value);
    return { type: "type" in value ? wsType(value.type) : binary ? "binary" : "text", body };
  }

  // Copies the fields of an object a script returned instead of the
  // message it was given.
  function adopt(msg, value) {
    if (value === undefined || value === msg) return msg;
    if (value === null || typeof value !== "object") throw new TypeError("return the message (or nothing) to keep it");
    if (state.has(value)) return value;
    for (const key of ["method", "url", "scheme", "host", "port", "path", "query", "status", "headers"]) {
      if (key in value) msg[key] = value[key];
    }
    for (const key of ["body", "text", "rawBody"]) {
      if (key in value) msg[key] = value[key];
    }
    return msg;
  }

  function sameBytes(a, b) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }

  // The changed body's bytes, or null when the body is as it came.
  function finishBody(msg, s) {
    if (s.tooLarge) return null;
    const ct = contentType(msg.headers);
    if (s.set) return bodyBytes(s.set.value, ct);
    if (s.parsed && s.parsed.canonical !== undefined) {
      const now = canonical(s.parsed.value, ct);
      if (now !== s.parsed.canonical) return N.utf8Encode(now);
    }
    if (s.raw && !sameBytes(s.raw, s.bytes)) return s.raw;
    return null;
  }

  function finishHeaders(msg, s) {
    const now = headerList(msg.headers);
    return sameList(now, s.headerList) ? null : now;
  }

  function finishRequest(msg) {
    const s = state.get(msg);
    const raw = s.source;
    const out = { headers: finishHeaders(msg, s), body: finishBody(msg, s), method: String(msg.method || raw.method) };
    if (String(msg.url) !== raw.url) {
      const url = new URL(String(msg.url));
      out.scheme = url.protocol.replace(/:$/, "");
      out.host = url.hostname.replace(/^\[|\]$/g, "");
      out.port = url.port ? Number(url.port) : url.protocol === "https:" ? 443 : 80;
      out.path = url.pathname || "/";
      out.query = url.search.replace(/^\?/, "");
      return out;
    }
    out.scheme = String(msg.scheme);
    out.host = String(msg.host);
    out.port = Number(msg.port);
    out.path = String(msg.path || "/");
    const query = pairList(msg.query);
    out.query = sameList(query, s.queryList) ? raw.query : formText(msg.query || {});
    return out;
  }

  function finishResponse(msg) {
    const s = state.get(msg);
    return { status: Number(msg.status), headers: finishHeaders(msg, s), body: finishBody(msg, s) };
  }

  function mockAnswer(value) {
    if (value === undefined || value === null) return null;
    if (typeof value === "string") {
      return { status: 200, headers: [["content-type", "text/plain; charset=utf-8"]], body: N.utf8Encode(value) };
    }
    if (value instanceof Uint8Array || ArrayBuffer.isView(value) || value instanceof ArrayBuffer) {
      return { status: 200, headers: [["content-type", "application/octet-stream"]], body: new Uint8Array(toBytes(value)) };
    }
    if (typeof value !== "object") throw new TypeError("onMock must return a response object, a string, bytes or null");
    const spec = ["status", "headers", "body", "text", "rawBody"].some((k) => k in value) ? value : { body: value };
    const headers = headerList(spec.headers);
    const has = (name) => headers.some(([k]) => k === name);
    let body;
    if ("rawBody" in spec) {
      body = bodyBytes(spec.rawBody, "");
      if (!has("content-type")) headers.push(["content-type", "application/octet-stream"]);
    } else if ("text" in spec) {
      body = N.utf8Encode(String(spec.text ?? ""));
      if (!has("content-type")) headers.push(["content-type", "text/plain; charset=utf-8"]);
    } else {
      const b = spec.body;
      const ct = contentType(headerObject(headers));
      body = bodyBytes(b, ct);
      if (!has("content-type") && b !== undefined && b !== null) {
        if (typeof b === "string") headers.push(["content-type", "text/plain; charset=utf-8"]);
        else if (b instanceof Uint8Array || ArrayBuffer.isView(b) || b instanceof ArrayBuffer)
          headers.push(["content-type", "application/octet-stream"]);
        else headers.push(["content-type", "application/json"]);
      }
    }
    const status = spec.status === undefined ? 200 : Number(spec.status);
    if (!Number.isInteger(status) || status < 100 || status > 999) throw new RangeError(`${spec.status} isn't a status code`);
    return { status, headers, body };
  }

  const VIEW_KINDS = ["json", "text", "xml", "html", "hex", "image"];

  function views(value) {
    if (value === undefined || value === null) return [];
    const list = Array.isArray(value) ? value : [value];
    return list.map((item, i) => {
      if (item === null || typeof item !== "object") {
        return { title: `View ${i + 1}`, kind: "text", text: String(item), bytes: null, mime: null };
      }
      let body = "rawBody" in item ? item.rawBody : "text" in item ? item.text : item.body;
      const binary = body instanceof Uint8Array || ArrayBuffer.isView(body) || body instanceof ArrayBuffer;
      let kind = item.kind ? String(item.kind).toLowerCase() : binary ? "hex" : typeof body === "object" && body !== null ? "json" : "text";
      if (!VIEW_KINDS.includes(kind)) throw new TypeError(`unknown view kind ${item.kind}; use ${VIEW_KINDS.join(", ")}`);
      const title = item.title === undefined ? `View ${i + 1}` : String(item.title);
      if (kind === "hex" || kind === "image") {
        return { title, kind, text: null, bytes: binary ? new Uint8Array(toBytes(body)) : bodyBytes(body, ""), mime: item.mime ? String(item.mime) : null };
      }
      if (binary) body = textOf(toBytes(body), false);
      else if (kind === "json" && typeof body !== "string") body = stringifyJson(body === undefined ? null : body, 2);
      else if (typeof body !== "string") body = body === undefined || body === null ? "" : String(body);
      return { title, kind, text: body, bytes: null, mime: null };
    });
  }

  // Runs one entry point. Always async, so the worker drains one promise.
  async function invoke(fn, phase, info, shared, a, b) {
    const context = Object.freeze({ ...info, shared });
    switch (phase) {
      case "request": {
        const request = makeMessage(a, true, false);
        return finishRequest(adopt(request, await fn(context, a.url, request)));
      }
      case "response": {
        const request = makeMessage(a, true, true);
        const response = makeMessage(b, false, false);
        return finishResponse(adopt(response, await fn(context, a.url, request, response)));
      }
      case "mock": {
        const request = makeMessage(a, true, true);
        return mockAnswer(await fn(context, a.url, request));
      }
      case "view": {
        const flow = Object.freeze({
          id: info.flowId,
          part: info.part,
          request: makeMessage(a, true, true),
          response: b ? makeMessage(b, false, true) : null,
        });
        return views(await fn(context, flow));
      }
      case "websocket": {
        const message = makeWsMessage(a);
        return finishWs(message, await fn(context, message));
      }
      default:
        throw new Error(`unknown phase ${phase}`);
    }
  }

  const globals = { TextEncoder, TextDecoder, URL, URLSearchParams, console, crypto, Proxymate };
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true, enumerable: false });
  }
  Object.freeze(Proxymate);
  Object.freeze(crypto);
  return { invoke };
});
