/**
 * CodeMirror languages for the viewers and editors. The two small stream
 * modes (GraphQL, raw HTTP) are always here; the Lezer grammars (JSON, XML,
 * HTML, JavaScript, CSS) are big, so each loads on first use and the editor
 * swaps it into a `Compartment` when it arrives (`applyLanguage`).
 */
import { StreamLanguage, type StringStream } from "@codemirror/language";
import type { Compartment, Extension } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

export type CodeLanguage = "json" | "xml" | "html" | "javascript" | "css" | "graphql" | "http" | "text";

const GRAPHQL_KEYWORDS =
  /^(?:query|mutation|subscription|fragment|on|type|interface|union|enum|input|scalar|schema|extend|directive|implements|repeatable)\b/;

/** GraphQL documents: keywords, fields, `$variables`, `@directives`, types. */
export const graphqlLanguage = StreamLanguage.define<{ blockString: boolean }>({
  name: "graphql",
  startState: () => ({ blockString: false }),
  token(stream, state) {
    if (state.blockString) {
      if (stream.skipTo('"""')) {
        stream.match('"""');
        state.blockString = false;
      } else stream.skipToEnd();
      return "string";
    }
    if (stream.eatSpace()) return null;
    if (stream.match("#")) {
      stream.skipToEnd();
      return "comment";
    }
    if (stream.match('"""')) {
      state.blockString = true;
      return "string";
    }
    if (stream.match(/^"(?:[^"\\]|\\.)*"?/)) return "string";
    if (stream.match(/^\$[_A-Za-z]\w*/)) return "variableName.special";
    if (stream.match(/^@[_A-Za-z]\w*/)) return "meta";
    if (stream.match(/^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/)) return "number";
    if (stream.match(/^(?:true|false|null)\b/)) return "bool";
    if (stream.match(GRAPHQL_KEYWORDS)) return "keyword";
    if (stream.match(/^[_A-Za-z]\w*/)) {
      const word = stream.current();
      return /^[A-Z]/.test(word) ? "typeName" : "propertyName";
    }
    if (stream.match("...")) return "punctuation";
    const ch = stream.next();
    return ch && "{}()[]".includes(ch) ? "bracket" : "punctuation";
  },
  languageData: { commentTokens: { line: "#" } },
});

interface HttpState {
  line: number;
  body: boolean;
  afterColon: boolean;
}

/** A raw HTTP message: start line, headers, then a lightly tokenised body. */
export const httpLanguage = StreamLanguage.define<HttpState>({
  name: "http",
  startState: () => ({ line: 0, body: false, afterColon: false }),
  blankLine(state) {
    if (state.line > 0) state.body = true;
  },
  token(stream: StringStream, state) {
    if (stream.sol()) {
      state.afterColon = false;
      if (!state.body) state.line++;
    }
    if (state.body) return bodyToken(stream);
    if (state.line === 1) {
      if (stream.eatSpace()) return null;
      if (stream.match(/^HTTP\/[\d.]+/)) return "meta";
      if (stream.match(/^\d{3}\b/)) return "number";
      if (stream.match(/^[A-Z]+(?=\s)/) && stream.column() === 0) return "keyword";
      if (stream.match(/^\S+/)) return stream.current().includes("/") ? "string" : null;
      stream.next();
      return null;
    }
    if (!state.afterColon) {
      if (stream.match(/^[^:]+/)) return "propertyName";
      if (stream.eat(":")) {
        state.afterColon = true;
        return "punctuation";
      }
    }
    stream.skipToEnd();
    return "string";
  },
});

function bodyToken(stream: StringStream): string | null {
  if (stream.eatSpace()) return null;
  if (stream.match(/^"(?:[^"\\]|\\.)*"(?=\s*:)/)) return "propertyName";
  if (stream.match(/^"(?:[^"\\]|\\.)*"?/)) return "string";
  if (stream.match(/^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/)) return "number";
  if (stream.match(/^(?:true|false|null)\b/)) return "bool";
  if (stream.match(/^<\/?[\w:-]+/) || stream.match(/^\/?>/)) return "tagName";
  if (stream.match(/^[\w-]+/)) return null;
  stream.next();
  return "punctuation";
}

type LoadedLanguage = "json" | "xml" | "html" | "javascript" | "css";

const LOADERS: Record<LoadedLanguage, () => Promise<Extension>> = {
  json: () => import("@codemirror/lang-json").then((m) => m.json()),
  xml: () => import("@codemirror/lang-xml").then((m) => m.xml()),
  html: () => import("@codemirror/lang-html").then((m) => m.html({ matchClosingTags: true, autoCloseTags: false })),
  javascript: () => import("@codemirror/lang-javascript").then((m) => m.javascript()),
  css: () => import("@codemirror/lang-css").then((m) => m.css()),
};

const loaded = new Map<CodeLanguage, Extension>();
const loading = new Map<CodeLanguage, Promise<Extension>>();

/** The language's extension if it is available right now, else null (see `loadLanguage`). */
export function readyLanguage(lang: CodeLanguage): Extension | null {
  switch (lang) {
    case "graphql":
      return graphqlLanguage;
    case "http":
      return httpLanguage;
    case "text":
      return [];
    default:
      return loaded.get(lang) ?? null;
  }
}

/** The language's extension, loading its grammar the first time (later calls share it). */
export function loadLanguage(lang: CodeLanguage): Promise<Extension> {
  const ready = readyLanguage(lang);
  if (ready !== null) return Promise.resolve(ready);
  let pending = loading.get(lang);
  if (!pending) {
    pending = LOADERS[lang as LoadedLanguage]().then((ext) => {
      loaded.set(lang, ext);
      return ext;
    });
    // A failed chunk load may work next time.
    pending.catch(() => loading.delete(lang));
    loading.set(lang, pending);
  }
  return pending;
}

/**
 * Keeps `compartment` of a live editor on `lang`: the extension now if it
 * is ready, otherwise nothing until its grammar arrives. Returns a cancel
 * for when the editor goes away (or the language changes first).
 */
export function applyLanguage(view: EditorView, compartment: Compartment, lang: CodeLanguage): () => void {
  const ready = readyLanguage(lang);
  if (ready !== null) {
    if (compartment.get(view.state) !== ready) view.dispatch({ effects: compartment.reconfigure(ready) });
    return () => {};
  }
  let live = true;
  loadLanguage(lang).then(
    (ext) => {
      if (live) view.dispatch({ effects: compartment.reconfigure(ext) });
    },
    // The text stays plain; highlighting is a nicety.
    () => {},
  );
  return () => {
    live = false;
  };
}
