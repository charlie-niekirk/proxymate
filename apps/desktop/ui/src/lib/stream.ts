import type { FlowDetail_Serialize, Header, WsClose, WsOpcode } from "@/bindings";
import { headerValue } from "@/lib/http";

/** Which message panel a flow gets, if any. */
export type StreamKind = "websocket" | "sse";

export function streamKind(
  flow: { scheme: string; status: number | null },
  detail: Pick<FlowDetail_Serialize, "response"> | null,
): StreamKind | null {
  if ((flow.scheme === "ws" || flow.scheme === "wss") && flow.status === 101) return "websocket";
  if (detail?.response && isEventStream(detail.response.headers)) return "sse";
  return null;
}

export function isEventStream(headers: Header[]): boolean {
  const type = headerValue(headers, "content-type");
  return type?.split(";")[0]?.trim().toLowerCase() === "text/event-stream";
}

const CLOSE_NAMES: Record<number, string> = {
  1000: "Normal Closure",
  1001: "Going Away",
  1002: "Protocol Error",
  1003: "Unsupported Data",
  1005: "No Status",
  1006: "Abnormal Closure",
  1007: "Invalid Payload",
  1008: "Policy Violation",
  1009: "Message Too Big",
  1010: "Missing Extension",
  1011: "Internal Error",
  1012: "Service Restart",
  1013: "Try Again Later",
  1015: "TLS Handshake",
};

/** `1000 Normal Closure`, or the bare code for application codes. */
export function closeCodeLabel(code: number): string {
  const name = CLOSE_NAMES[code];
  return name ? `${code} ${name}` : String(code);
}

/** One line for the panel header: who closed, how and why. */
export function describeClose(close: WsClose): string {
  const by = close.by === "sent" ? "client" : "server";
  const parts = [`Closed by ${by}`, closeCodeLabel(close.code)];
  if (close.reason) parts.push(close.reason);
  return parts.join(" · ");
}

export const OPCODE_LABEL: Record<WsOpcode, string> = {
  text: "Text",
  binary: "Binary",
  ping: "Ping",
  pong: "Pong",
  close: "Close",
};
