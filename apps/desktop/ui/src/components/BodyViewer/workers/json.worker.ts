/// <reference lib="webworker" />
// Parses (or pretty-prints) large JSON bodies off the main thread.
import { jsonOp } from "./jsonOp";

export interface JsonRequest {
  id: number;
  bytes: Uint8Array;
  op: "parse" | "pretty";
}

export type JsonResponse = { id: number; ok: true; result: unknown } | { id: number; ok: false; error: string };

self.onmessage = (e: MessageEvent<JsonRequest>) => {
  const { id, bytes, op } = e.data;
  try {
    postMessage({ id, ok: true, result: jsonOp(new TextDecoder().decode(bytes), op) } satisfies JsonResponse);
  } catch (err) {
    postMessage({ id, ok: false, error: err instanceof Error ? err.message : String(err) } satisfies JsonResponse);
  }
};

