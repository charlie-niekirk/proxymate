import { useQuery } from "@tanstack/react-query";

import { backend } from "@/lib/backend";
import { trustStatusKey } from "@/lib/query";

/** CA trust, checked once at launch and again whenever asked. */
export function useTrustStatus() {
  return useQuery({ queryKey: trustStatusKey, queryFn: () => backend.getTrustStatus() });
}
