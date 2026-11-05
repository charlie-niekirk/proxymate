import { useQuery } from "@tanstack/react-query";

import { queries } from "@/lib/queries";

/** CA trust, checked once at launch and again whenever asked. */
export function useTrustStatus() {
  return useQuery(queries.trustStatus());
}
