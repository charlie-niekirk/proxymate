import { formatJson } from "@/lib/body/jsonFormat";
import { parseLossless } from "@/lib/body/losslessJson";
import { parseDocFrom } from "@/lib/json/doc";

/**
 * `parse` gives the value, numbers kept exactly (`lib/body/losslessJson`);
 * `pretty` re-indents the text losslessly, if it is valid JSON; `doc`
 * builds the JSON editor's document (`lib/json/doc`), ids from `firstId`.
 */
export function jsonOp(text: string, op: "parse" | "pretty" | "doc", firstId = 1): unknown {
  if (op === "parse") return parseLossless(text);
  if (op === "doc") return parseDocFrom(text, firstId);
  const body = text.replace(/^\uFEFF/, "");
  try {
    // The native parser is the fastest validator.
    JSON.parse(body);
  } catch (err) {
    // Throws the same positioned error the tree shows.
    parseLossless(body);
    throw err;
  }
  // A top-level scalar is already its own pretty form, digits intact.
  return formatJson(body) ?? body.trim();
}
