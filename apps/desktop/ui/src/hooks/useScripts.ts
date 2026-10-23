import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import { backend } from "@/lib/backend";
import { scriptsKey } from "@/lib/query";
import { normalizeScript, type Script } from "@/lib/scripts";

/** Every script; the engine pushes changes (`store/mcp.ts`). */
export function useScripts(): Script[] {
  const { data } = useQuery({ queryKey: scriptsKey, queryFn: () => backend.listScripts() });
  return useMemo(() => (data ?? []).map(normalizeScript), [data]);
}

/** Script names by id, for rule summaries. */
export function useScriptNames(): ReadonlyMap<string, string> {
  const scripts = useScripts();
  return useMemo(() => new Map(scripts.map((s) => [s.id, s.name])), [scripts]);
}

/**
 * Changes whenever an enabled view script does, so cached view results
 * refetch after an edit.
 */
export function useViewScriptsVersion(): string {
  const scripts = useScripts();
  return useMemo(
    () =>
      scripts
        .filter((s) => s.kind === "view" && s.enabled)
        .map((s) => `${s.id}@${s.updated_at_ms ?? 0}`)
        .join(","),
    [scripts],
  );
}
