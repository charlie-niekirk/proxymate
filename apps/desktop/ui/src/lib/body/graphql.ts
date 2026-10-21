/** GraphQL operations from a request body or a GET query string. */

export interface GraphQlOperation {
  operationName: string | null;
  /** `query`, `mutation`, `subscription`, or null for a persisted query with no text. */
  type: string | null;
  query: string;
  variables: unknown;
  extensions: unknown;
}

function operationType(query: string): string | null {
  const q = query.replace(/#[^\n]*/g, "").trimStart();
  if (q.startsWith("{")) return "query";
  return /^(query|mutation|subscription)\b/.exec(q)?.[1] ?? null;
}

function nameIn(query: string): string | null {
  return /\b(?:query|mutation|subscription)\s+([_A-Za-z]\w*)/.exec(query)?.[1] ?? null;
}

function fromObject(o: unknown): GraphQlOperation | null {
  if (o === null || typeof o !== "object" || Array.isArray(o)) return null;
  const r = o as Record<string, unknown>;
  if (typeof r.query !== "string" && r.extensions === undefined) return null;
  const query = typeof r.query === "string" ? r.query : "";
  return {
    operationName: (typeof r.operationName === "string" && r.operationName) || nameIn(query),
    type: operationType(query),
    query,
    variables: r.variables ?? null,
    extensions: r.extensions ?? null,
  };
}

/** The operations in a body (one, or a batch), or null if it isn't GraphQL. */
export function parseGraphQlBody(text: string, contentType: string | null): GraphQlOperation[] | null {
  if ((contentType ?? "").toLowerCase().startsWith("application/graphql") && !text.trimStart().startsWith("{\"")) {
    return [{ operationName: nameIn(text), type: operationType(text), query: text, variables: null, extensions: null }];
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  const ops = (Array.isArray(value) ? value : [value]).map(fromObject);
  return ops.length > 0 && ops.every((op) => op !== null) ? (ops as GraphQlOperation[]) : null;
}

/** A GET operation from `?query=…&variables=…&operationName=…`. */
export function parseGraphQlQueryString(url: string): GraphQlOperation | null {
  const q = url.indexOf("?");
  if (q < 0) return null;
  const params = new URLSearchParams(url.slice(q + 1));
  const query = params.get("query");
  if (query === null) return null;
  const json = (key: string) => {
    const raw = params.get(key);
    if (raw === null) return null;
    try {
      return JSON.parse(raw) as unknown;
    } catch {
      return raw;
    }
  };
  return fromObject({ query, operationName: params.get("operationName"), variables: json("variables"), extensions: json("extensions") });
}
