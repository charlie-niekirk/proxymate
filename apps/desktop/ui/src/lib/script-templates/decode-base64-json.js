// Finds string fields that hold base64-encoded JSON (standard or URL-safe,
// padded or not) and shows each one decoded.
// To look at one field only, name it: const FIELDS = ["payload", "data.token"];
const FIELDS = [];

function decoded(text) {
  if (typeof text !== "string" || text.length < 8 || !/^[A-Za-z0-9+/_-]+={0,2}$/.test(text)) return undefined;
  try {
    const value = Proxymate.base64DecodeJson(text);
    return value !== null && typeof value === "object" ? value : undefined;
  } catch {
    return undefined;
  }
}

function walk(value, path, views) {
  const json = decoded(value);
  if (json !== undefined) {
    views.push({ title: path || "body", body: json });
  } else if (value !== null && typeof value === "object" && views.length < 20) {
    for (const [key, child] of Object.entries(value)) walk(child, path ? path + "." + key : key, views);
  }
  return views;
}

export function onView(context, flow) {
  const body = flow[context.part]?.body;
  if (body === null || body === undefined) return null;
  if (FIELDS.length > 0) {
    return FIELDS.map((field) => ({ title: field, body: decoded(field.split(".").reduce((v, k) => v?.[k], body)) }))
      .filter((view) => view.body !== undefined);
  }
  return walk(body, "", []);
}
