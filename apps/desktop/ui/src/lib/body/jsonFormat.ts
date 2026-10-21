/**
 * Re-indents JSON text without parsing it into values, so numbers keep
 * their exact digits (`1.50`, 64-bit ids) and invalid JSON still comes out
 * readable. Returns null if the text doesn't look like JSON at all.
 */
export function formatJson(src: string, indent = "  "): string | null {
  const text = src.replace(/^\uFEFF/, "").trim();
  if (text[0] !== "{" && text[0] !== "[") return null;
  const out: string[] = [];
  let depth = 0;
  let i = 0;
  const newline = () => out.push("\n", indent.repeat(depth));
  while (i < text.length) {
    const c = text[i]!;
    if (c === '"') {
      // Copy the string verbatim, escapes included.
      let j = i + 1;
      while (j < text.length && text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      out.push(text.slice(i, j + 1));
      i = j + 1;
      continue;
    }
    if (c === "{" || c === "[") {
      // Keep empty containers on one line.
      let j = i + 1;
      while (j < text.length && /\s/.test(text[j]!)) j++;
      if (text[j] === (c === "{" ? "}" : "]")) {
        out.push(c, text[j]!);
        i = j + 1;
        continue;
      }
      out.push(c);
      depth++;
      newline();
    } else if (c === "}" || c === "]") {
      depth = Math.max(0, depth - 1);
      newline();
      out.push(c);
    } else if (c === ",") {
      out.push(",");
      newline();
    } else if (c === ":") {
      out.push(": ");
    } else if (!/\s/.test(c)) {
      // A literal: copy up to the next structural character.
      let j = i;
      while (j < text.length && !/[\s{}[\],:"]/.test(text[j]!)) j++;
      out.push(text.slice(i, j));
      i = j;
      continue;
    }
    i++;
  }
  return out.join("");
}
