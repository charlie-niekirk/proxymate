import { describe, expect, it } from "vitest";

import {
  canResend,
  closeCodeLabel,
  describeClose,
  isEventStream,
  messageFlags,
  parseHex,
  resendOf,
  streamKind,
  wsSendOf,
} from "@/lib/stream";

describe("streamKind", () => {
  it("picks the WebSocket panel for switched ws flows only", () => {
    expect(streamKind({ scheme: "wss", status: 101 }, null)).toBe("websocket");
    expect(streamKind({ scheme: "ws", status: 403 }, null)).toBeNull();
    expect(streamKind({ scheme: "ws", status: null }, null)).toBeNull();
  });

  it("picks the SSE panel for event-stream responses", () => {
    const response = (type: string) => ({
      response: { status: 200, reason: "OK", version: "HTTP/1.1" as const, headers: [{ name: "Content-Type", value: type }] },
    });
    expect(streamKind({ scheme: "https", status: 200 }, response("text/event-stream; charset=utf-8"))).toBe("sse");
    expect(streamKind({ scheme: "https", status: 200 }, response("text/plain"))).toBeNull();
    expect(isEventStream([])).toBe(false);
  });
});

describe("close descriptions", () => {
  it("names standard codes and keeps application ones bare", () => {
    expect(closeCodeLabel(1000)).toBe("1000 Normal Closure");
    expect(closeCodeLabel(4000)).toBe("4000");
    expect(describeClose({ code: 1001, reason: "bye", by: "received" })).toBe("Closed by server · 1001 Going Away · bye");
    expect(describeClose({ code: 1006, reason: "", by: "sent" })).toBe("Closed by client · 1006 Abnormal Closure");
  });
});

describe("sending messages", () => {
  it("reads hex in the usual spellings", () => {
    expect(Array.from(parseHex("de ad BE ef")!)).toEqual([0xde, 0xad, 0xbe, 0xef]);
    expect(Array.from(parseHex("0x00ff,0x10")!)).toEqual([0, 255, 16]);
    expect(Array.from(parseHex("")!)).toEqual([]);
    expect(parseHex("abc")).toBeNull();
    expect(parseHex("zz")).toBeNull();
  });

  it("builds composer messages", () => {
    expect(wsSendOf("sent", "text", "hi")).toEqual({ direction: "sent", payload: "hi", binary: false });
    expect(wsSendOf("received", "hex", "00 01 02")).toEqual({ direction: "received", payload: "AAEC", binary: true });
    expect(typeof wsSendOf("sent", "hex", "nope")).toBe("string");
  });

  it("resends text as text and binary as base64", () => {
    expect(resendOf({ direction: "sent", opcode: "text" }, new TextEncoder().encode("é"))).toEqual({
      direction: "sent",
      payload: "é",
      binary: false,
    });
    expect(resendOf({ direction: "received", opcode: "binary" }, new Uint8Array([0, 1, 2]))).toEqual({
      direction: "received",
      payload: "AAEC",
      binary: true,
    });
    expect(canResend({ opcode: "ping", truncated: false })).toBe(false);
    expect(canResend({ opcode: "binary", truncated: true })).toBe(false);
  });

  it("lists a message's flags", () => {
    expect(messageFlags({ edited: true, injected: false, dropped: true })).toEqual(["Edited", "Dropped"]);
    expect(messageFlags({ edited: false, injected: true, dropped: false })).toEqual(["Injected"]);
  });
});
