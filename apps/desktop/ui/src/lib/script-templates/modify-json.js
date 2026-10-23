// Changes a field of a JSON response. Untouched fields keep their exact
// text, and integers beyond 2^53 arrive as BigInt.
export function onResponse(context, url, request, response) {
  if (response.body && typeof response.body === "object") {
    response.body.debug = true;
  }
  return response;
}
