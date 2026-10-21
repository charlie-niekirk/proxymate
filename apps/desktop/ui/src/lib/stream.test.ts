import { describe, expect, it } from "vitest";

import { closeCodeLabel, describeClose, isEventStream, streamKind } from "@/lib/stream";

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
