import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import type { MockSource } from "@/bindings";
import { queries } from "@/lib/queries";

/** The mock library (M20). */
export function useMockSources(): MockSource[] {
  const { data } = useQuery(queries.mockSources());
  return data ?? [];
}

/** Mock library names by id, for rule summaries. */
export function useMockSourceNames(): ReadonlyMap<string, string> {
  const sources = useMockSources();
  return useMemo(() => new Map(sources.map((s) => [s.id, s.name])), [sources]);
}
