/**
 * GraphQL operations from a request body or a GET query string. Parsed
 * into `lib/json/doc` nodes, so variables keep their member order and exact
 * numbers (a 64-bit id stays as sent).
 */

import { parseDoc, toJson, type JNode } from "@/lib/json/doc";

export interface GraphQlOperation {
  operationName: string | null;
  /** `query`, `mutation`, `subscription`, or null for a persisted query with no text. */
  type: string | null;
  query: string;
  /** As sent; null when absent or `null`. */
  variables: JNode | null;
  extensions: JNode | null;
}

/** Pretty JSON of an operation's variables or extensions, as sent. */
export function jsonText(node: JNode | null): string | null {
  return node === null ? null : toJson(node, 2);
}

function operationType(query: string): string | null {
  const q = query.replace(/#[^\n]*/g, "").trimStart();
  if (q.startsWith("{")) return "query";
  return /^(query|mutation|subscription)\b/.exec(q)?.[1] ?? null;
}

function nameIn(query: string): string | null {
  return /\b(?:query|mutation|subscription)\s+([_A-Za-z]\w*)/.exec(query)?.[1] ?? null;
}

/** The last member `key` (as JSON readers take it), null for none or `null`. */
function member(node: JNode, key: string): JNode | null {
  if (node.t !== "obj") return null;
  const v = node.entries.findLast((e) => e.key === key)?.v;
  return v && v.t !== "null" ? v : null;
}

function fromNode(node: JNode): GraphQlOperation | null {
  if (node.t !== "obj") return null;
  const q = member(node, "query");
  const extensions = member(node, "extensions");
  if (q?.t !== "str" && extensions === null) return null;
  const query = q?.t === "str" ? q.v : "";
  const name = member(node, "operationName");
  return {
    operationName: (name?.t === "str" && name.v) || nameIn(query),
    type: operationType(query),
    query,
    variables: member(node, "variables"),
    extensions,
  };
}

/** The operations in a body (one, or a batch), or null if it isn't GraphQL. */
export function parseGraphQlBody(text: string, contentType: string | null): GraphQlOperation[] | null {
  if ((contentType ?? "").toLowerCase().startsWith("application/graphql") && !text.trimStart().startsWith("{\"")) {
    return [{ operationName: nameIn(text), type: operationType(text), query: text, variables: null, extensions: null }];
  }
  let value: JNode;
  try {
    value = parseDoc(text);
  } catch {
    return null;
  }
  const ops = (value.t === "arr" ? value.items.map((i) => i.v) : [value]).map(fromNode);
  return ops.length > 0 && ops.every((op) => op !== null) ? (ops as GraphQlOperation[]) : null;
}

/** A GET operation from `?query=…&variables=…&operationName=…`. */
export function parseGraphQlQueryString(url: string): GraphQlOperation | null {
  const q = url.indexOf("?");
  if (q < 0) return null;
  const params = new URLSearchParams(url.slice(q + 1));
  const query = params.get("query");
  if (query === null) return null;
  const json = (key: string): JNode | null => {
    const raw = params.get(key);
    if (raw === null) return null;
    try {
      const node = parseDoc(raw);
      return node.t === "null" ? null : node;
    } catch {
      return { t: "str", v: raw };
    }
  };
  const name = params.get("operationName");
  return {
    operationName: name || nameIn(query),
    type: operationType(query),
    query,
    variables: json("variables"),
    extensions: json("extensions"),
  };
}
