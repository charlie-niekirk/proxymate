import { useQuery } from "@tanstack/react-query";

import type { BodyInfo, BodyPart } from "@/bindings";
import { backend } from "@/lib/backend";

export interface LoadedBody {
  bytes: Uint8Array;
  /** Set when the bytes were decoded from this content-encoding. */
  decodedFrom?: string;
  /** Set when decoding failed and `bytes` are the raw, encoded body. */
  decodeError?: string;
}

/**
 * Loads a body once it has finished streaming. With `decoded`, an encoded
 * body is decoded in Rust; if that fails the raw bytes come back instead,
 * with the error.
 */
export function useBody(id: number, part: BodyPart, info: BodyInfo, decoded: boolean, finished: boolean) {
  const encoding = decoded ? info.encoding : null;
  return useQuery({
    // Not under flowKey(id): live-flow refreshes must not refetch bodies.
    queryKey: ["body", id, part, Boolean(encoding), info.captured, finished],
    enabled: finished && info.captured > 0,
    queryFn: async (): Promise<LoadedBody> => {
      if (!encoding) return { bytes: new Uint8Array(await backend.getBody(id, part, false)) };
      try {
        return { bytes: new Uint8Array(await backend.getBody(id, part, true)), decodedFrom: encoding };
      } catch (err) {
        const bytes = new Uint8Array(await backend.getBody(id, part, false));
        return { bytes, decodeError: err instanceof Error ? err.message : String(err) };
      }
    },
  });
}
