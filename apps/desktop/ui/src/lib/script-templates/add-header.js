// Adds a header to every request the rule matches.
export function onRequest(context, url, request) {
  request.headers["x-proxymate"] = "1";
  return request;
}

// Responses can be changed too:
// export function onResponse(context, url, request, response) {
//   response.headers["cache-control"] = "no-store";
//   return response;
// }
