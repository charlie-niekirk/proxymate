import { formatJson } from "@/lib/body/jsonFormat";

/** `parse` gives the value; `pretty` re-indents the text losslessly, if it is valid JSON. */
export function jsonOp(text: string, op: "parse" | "pretty"): unknown {
  const value: unknown = JSON.parse(text.replace(/^\uFEFF/, ""));
  return op === "pretty" ? (formatJson(text) ?? JSON.stringify(value, null, 2)) : value;
}
