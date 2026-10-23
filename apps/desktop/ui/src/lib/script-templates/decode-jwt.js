// Decodes JSON Web Tokens: a Bearer token in the Authorization header, and
// any body field that holds one. The signature isn't checked.
const JWT = /^[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*$/;

function decode(token) {
  const [header, payload] = token.split(".");
  const claims = Proxymate.base64DecodeJson(payload);
  const times = {};
  for (const claim of ["iat", "nbf", "exp"]) {
    if (typeof claims[claim] === "number") times[claim] = new Date(claims[claim] * 1000).toISOString();
  }
  return { header: Proxymate.base64DecodeJson(header), payload: claims, times };
}

function tokens(value, path, found) {
  if (typeof value === "string" && JWT.test(value)) found.push([path, value]);
  else if (value !== null && typeof value === "object" && found.length < 10) {
    for (const [key, child] of Object.entries(value)) tokens(child, path ? path + "." + key : key, found);
  }
  return found;
}

export function onView(context, flow) {
  const message = flow[context.part];
  const found = [];
  const auth = [].concat(message.headers.authorization ?? []);
  for (const value of auth) {
    const token = value.replace(/^Bearer\s+/i, "");
    if (JWT.test(token)) found.push(["Authorization", token]);
  }
  tokens(message.body, "", found);
  return found.map(([where, token]) => {
    try {
      return { title: "JWT · " + where, body: decode(token) };
    } catch (e) {
      return { title: "JWT · " + where, kind: "text", body: "Not a JWT: " + e.message };
    }
  });
}
