import type { FlowDetail_Serialize, Header, WsClose, WsDirection, WsMessage_Serialize, WsOpcode, WsSend } from "@/bindings";
import { base64 } from "@/lib/copyFormats";
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

/** Bytes from hex text (`de ad be ef`, `0xdead`, any spacing), or null if it isn't hex. */
export function parseHex(text: string): Uint8Array | null {
  const digits = text.replace(/0x/gi, "").replace(/[\s,:]/g, "");
  if (digits.length % 2 !== 0 || /[^0-9a-f]/i.test(digits)) return null;
  const out = new Uint8Array(digits.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(digits.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** How the composer reads what was typed. */
export type ComposeMode = "text" | "hex";

/** A message for `Engine::ws_send` from the composer; an error string if it can't be sent. */
export function wsSendOf(direction: WsDirection, mode: ComposeMode, input: string): WsSend | string {
  if (mode === "text") return { direction, payload: input, binary: false };
  const bytes = parseHex(input);
  if (bytes === null) return "That isn't hex: use pairs of digits like 0a ff 3c.";
  return { direction, payload: base64(bytes), binary: true };
}

/** Resend: the recorded message again, its payload as it was forwarded. */
export function resendOf(message: Pick<WsMessage_Serialize, "direction" | "opcode">, payload: Uint8Array): WsSend {
  if (message.opcode === "text") {
    return { direction: message.direction, payload: new TextDecoder().decode(payload), binary: false };
  }
  return { direction: message.direction, payload: base64(payload), binary: true };
}

/** The badges a message row shows (M19). */
export function messageFlags(m: Pick<WsMessage_Serialize, "edited" | "injected" | "dropped">): ("Edited" | "Injected" | "Dropped")[] {
  const out: ("Edited" | "Injected" | "Dropped")[] = [];
  if (m.injected) out.push("Injected");
  if (m.edited) out.push("Edited");
  if (m.dropped) out.push("Dropped");
  return out;
}

/** True for messages Resend can send again. */
export function canResend(m: Pick<WsMessage_Serialize, "opcode" | "truncated">): boolean {
  return (m.opcode === "text" || m.opcode === "binary") && !m.truncated;
}
