// Answers without contacting the server. Return null to let the request go on.
export function onMock(context, url, request) {
  context.shared.calls = (context.shared.calls ?? 0) + 1;
  return {
    status: 200,
    headers: { "x-mocked-by": "proxymate" },
    body: { ok: true, path: request.path, query: request.query, call: context.shared.calls },
  };
}
