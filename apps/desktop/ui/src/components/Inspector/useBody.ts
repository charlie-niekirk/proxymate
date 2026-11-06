import { useQuery } from "@tanstack/react-query";

import type { BodyInfo, BodyPart } from "@/bindings";
import { backend } from "@/lib/backend";
import { useUi } from "@/store/ui";
import { bodyKey, bodyUrlKey, type LoadedBody } from "@/lib/query";

export type { LoadedBody } from "@/lib/query";

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
    queryKey: bodyKey(store, id, part, Boolean(encoding), info.captured, finished, max),
    enabled: enabled && finished && info.captured > 0,
    queryFn: async ({ signal }): Promise<LoadedBody> => {
      if (!encoding) return { bytes: new Uint8Array(await backend.getBody(store, id, part, false, max, signal)) };
      try {
        return { bytes: new Uint8Array(await backend.getBody(store, id, part, true, max, signal)), decodedFrom: encoding };
      } catch (err) {
        if (signal.aborted) throw err;
        const bytes = new Uint8Array(await backend.getBody(store, id, part, false, max, signal));
        return { bytes, decodeError: err instanceof Error ? err.message : String(err) };
      }
    },
  });
}

/** A URL for the body (see `Backend.bodyUrl`), once it has finished streaming. */
export function useBodyUrl(id: number, part: BodyPart, info: BodyInfo, finished: boolean, enabled = true) {
  const store = useUi((s) => s.store);
  return useQuery({
    queryKey: bodyUrlKey(store, id, part, info.captured, finished),
    enabled: enabled && finished && info.captured > 0,
    queryFn: () => backend.bodyUrl(store, id, part, info.encoding !== null),
  });
}
