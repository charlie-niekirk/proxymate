import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { xml } from "@codemirror/lang-xml";
import { StreamLanguage, type StringStream } from "@codemirror/language";
import type { Extension } from "@codemirror/state";

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

export function languageExtension(lang: CodeLanguage): Extension {
  switch (lang) {
    case "json":
      return json();
    case "xml":
      return xml();
    case "html":
      return html({ matchClosingTags: true, autoCloseTags: false });
    case "javascript":
      return javascript();
    case "css":
      return css();
    case "graphql":
      return graphqlLanguage;
    case "http":
      return httpLanguage;
    case "text":
      return [];
  }
}
