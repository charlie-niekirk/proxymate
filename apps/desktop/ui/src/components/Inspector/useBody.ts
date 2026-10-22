import { useQuery } from "@tanstack/react-query";

import type { BodyInfo, BodyPart } from "@/bindings";
import { backend } from "@/lib/backend";
import { useUi } from "@/store/ui";

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
 * with the error. With `max`, only the first `max` bytes are loaded.
 */
export function useBody(
  id: number,
  part: BodyPart,
  info: BodyInfo,
  decoded: boolean,
  finished: boolean,
  max: number | null = null,
  enabled = true,
) {
  const store = useUi((s) => s.store);
  const encoding = decoded ? info.encoding : null;
  return useQuery({
    // Not under flowKey(id): live-flow refreshes must not refetch bodies.
    queryKey: ["body", store, id, part, Boolean(encoding), info.captured, finished, max],
    enabled: enabled && finished && info.captured > 0,
    queryFn: async (): Promise<LoadedBody> => {
      if (!encoding) return { bytes: new Uint8Array(await backend.getBody(store, id, part, false, max)) };
      try {
        return { bytes: new Uint8Array(await backend.getBody(store, id, part, true, max)), decodedFrom: encoding };
      } catch (err) {
        const bytes = new Uint8Array(await backend.getBody(store, id, part, false, max));
        return { bytes, decodeError: err instanceof Error ? err.message : String(err) };
      }
    },
  });
}

/** A URL for the body (see `Backend.bodyUrl`), once it has finished streaming. */
export function useBodyUrl(id: number, part: BodyPart, info: BodyInfo, finished: boolean, enabled = true) {
  const store = useUi((s) => s.store);
  return useQuery({
    queryKey: ["body-url", store, id, part, info.captured, finished],
    enabled: enabled && finished && info.captured > 0,
    queryFn: () => backend.bodyUrl(store, id, part, info.encoding !== null),
  });
}
