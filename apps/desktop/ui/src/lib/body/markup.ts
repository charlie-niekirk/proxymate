/**
 * Pretty-prints XML and HTML by re-indenting tags. A tokenizer, not a
 * parser: it never drops or reorders content, so malformed documents come
 * out readable rather than "fixed". Text inside script, style, pre and
 * textarea is kept verbatim.
 */

const VOID = new Set([
  "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr",
]);
const RAW = new Set(["script", "style", "pre", "textarea"]);

type Token =
  | { t: "open"; name: string; text: string; selfClosing: boolean }
  | { t: "close"; name: string; text: string }
  | { t: "other"; text: string }
  | { t: "text"; text: string };

const TAG = /<!--[\s\S]*?(?:-->|$)|<!\[CDATA\[[\s\S]*?(?:\]\]>|$)|<\?[\s\S]*?(?:\?>|$)|<![^>]*>?|<\/?[A-Za-z][^\s/>]*(?:"[^"]*"|'[^']*'|[^'">])*>?/g;

function tokenize(src: string, html: boolean): Token[] {
  const out: Token[] = [];
  let last = 0;
  TAG.lastIndex = 0;
  for (let m = TAG.exec(src); m !== null; m = TAG.exec(src)) {
    if (m.index > last) out.push({ t: "text", text: src.slice(last, m.index) });
    const text = m[0];
    last = TAG.lastIndex;
    if (text.startsWith("</")) {
      out.push({ t: "close", name: text.slice(2).replace(/[\s>].*$/s, "").toLowerCase(), text });
    } else if (/^<[A-Za-z]/.test(text)) {
      const name = text.slice(1).replace(/[\s/>].*$/s, "");
      const lower = name.toLowerCase();
      const selfClosing = text.endsWith("/>") || (html && VOID.has(lower));
      out.push({ t: "open", name: html ? lower : name, text, selfClosing });
      if (html && RAW.has(lower) && !selfClosing) {
        // Everything up to the matching close tag is content.
        const re = new RegExp(`</${lower}\\s*>`, "ig");
        re.lastIndex = last;
        const end = re.exec(src);
        const stop = end ? end.index : src.length;
        if (stop > last) out.push({ t: "text", text: src.slice(last, stop) });
        last = stop;
        TAG.lastIndex = stop;
      }
    } else {
      out.push({ t: "other", text });
    }
  }
  if (last < src.length) out.push({ t: "text", text: src.slice(last) });
  return out;
}

export function formatMarkup(src: string, { html = false, indent = "  " } = {}): string {
  const tokens = tokenize(src, html);
  const lines: string[] = [];
  const stack: string[] = [];
  const pad = () => indent.repeat(stack.length);
  const keyOf = (name: string) => (html ? name.toLowerCase() : name);
  for (let i = 0; i < tokens.length; i++) {
    const tok = tokens[i]!;
    switch (tok.t) {
      case "open": {
        if (tok.selfClosing) {
          lines.push(pad() + tok.text);
          break;
        }
        const next = tokens[i + 1];
        const after = tokens[i + 2];
        const raw = html && RAW.has(tok.name.toLowerCase());
        if (next?.t === "close" && keyOf(next.name) === keyOf(tok.name)) {
          lines.push(pad() + tok.text + next.text);
          i += 1;
          break;
        }
        if (
          next?.t === "text" &&
          after?.t === "close" &&
          keyOf(after.name) === keyOf(tok.name) &&
          (raw ? !next.text.includes("\n") : !/\n\s*\S[\s\S]*\n/.test(next.text.trim()) && next.text.trim().length <= 120)
        ) {
          const text = raw ? next.text : next.text.trim().replace(/\s*\n\s*/g, " ");
          lines.push(pad() + tok.text + text + after.text);
          i += 2;
          break;
        }
        lines.push(pad() + tok.text);
        stack.push(tok.name);
        if (raw && next?.t === "text") {
          lines.push(reindent(next.text, pad()));
          i += 1;
        }
        break;
      }
      case "close": {
        const at = stack.lastIndexOf(tok.name);
        if (at !== -1) stack.length = at;
        lines.push(pad() + tok.text);
        break;
      }
      case "other":
        lines.push(pad() + tok.text);
        break;
      case "text": {
        const text = tok.text.trim();
        if (text) for (const line of text.split(/\r?\n/)) if (line.trim()) lines.push(pad() + line.trim());
        break;
      }
    }
  }
  return lines.join("\n");
}

/** Re-indents a block of raw text (a script, a style) under `pad`, keeping its relative indentation. */
function reindent(text: string, pad: string): string {
  const lines = text.replace(/^\s*\n|\s+$/g, "").split(/\r?\n/);
  const common = Math.min(...lines.filter((l) => l.trim()).map((l) => l.length - l.trimStart().length));
  return lines.map((l) => (l.trim() ? pad + l.slice(Number.isFinite(common) ? common : 0) : "")).join("\n");
}
