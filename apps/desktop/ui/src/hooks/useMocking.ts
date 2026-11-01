import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import type { MockSource } from "@/bindings";
import { backend } from "@/lib/backend";
import { mockSourcesKey } from "@/lib/query";

/** The mock library (M20). */
export function useMockSources(): MockSource[] {
  const { data } = useQuery({ queryKey: mockSourcesKey, queryFn: () => backend.listMockSources() });
  return data ?? [];
}

/** Mock library names by id, for rule summaries. */
export function useMockSourceNames(): ReadonlyMap<string, string> {
  const sources = useMockSources();
  return useMemo(() => new Map(sources.map((s) => [s.id, s.name])), [sources]);
}
