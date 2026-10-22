import { formatJson } from "@/lib/body/jsonFormat";
import { parseLossless } from "@/lib/body/losslessJson";

/**
 * `parse` gives the value, numbers kept exactly (`lib/body/losslessJson`);
 * `pretty` re-indents the text losslessly, if it is valid JSON.
 */
export function jsonOp(text: string, op: "parse" | "pretty"): unknown {
  if (op === "parse") return parseLossless(text);
  const body = text.replace(/^﻿/, "");
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
