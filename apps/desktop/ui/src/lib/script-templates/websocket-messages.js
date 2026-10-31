// Rewrites WebSocket messages: marks JSON the server sends and drops the
// client's heartbeats. Return null to drop a message, a string or bytes to
// replace it, or the message (changed in place) to send it on.
export function onWsMessage(context, message) {
  const body = message.body;
  if (message.direction === "sent" && body && body.type === "ping") {
    return null;
  }
  if (message.direction === "received" && body && typeof body === "object") {
    body.viaProxymate = true;
  }
  return message;
}
