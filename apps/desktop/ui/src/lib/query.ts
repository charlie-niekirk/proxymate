import { QueryClient } from "@tanstack/react-query";

/**
 * Shared TanStack Query client for flow detail and bodies. Exported so
 * non-React code (the view delta handler) can invalidate what changed.
 */
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // The engine pushes changes; nothing goes stale on a timer.
      staleTime: Infinity,
      gcTime: 30_000,
      retry: false,
      refetchOnWindowFocus: false,
    },
  },
});

/** A flow's summary and detail, in a store (ids are per store). */
export const flowKey = (store: number, id: number | null) => ["flow", store, id] as const;
/** Live state of a flow's message log (counts, open/closed); refetched as the flow changes. */
export const streamKey = (store: number, id: number | null) => ["stream", store, id] as const;
export const documentsKey = ["documents"] as const;
export const proxyStatusKey = ["proxy-status"] as const;
export const caInfoKey = ["ca-info"] as const;
export const sslSettingsKey = ["ssl-settings"] as const;
export const trustStatusKey = ["trust-status"] as const;
export const systemProxyKey = ["system-proxy"] as const;
export const rulesKey = ["rules"] as const;
export const ruleSettingsKey = ["rule-settings"] as const;
export const deviceAccessKey = ["device-access"] as const;
export const simulatorKey = ["simulator"] as const;
export const androidKey = ["android"] as const;
export const mcpTokenKey = ["mcp-token"] as const;
export const mcpAuditKey = ["mcp-audit"] as const;
export const proxySettingsKey = ["proxy-settings"] as const;
export const storageSettingsKey = ["storage-settings"] as const;
