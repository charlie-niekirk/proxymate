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

export const flowKey = (id: number | null) => ["flow", id] as const;
export const proxyStatusKey = ["proxy-status"] as const;
export const caInfoKey = ["ca-info"] as const;
export const sslSettingsKey = ["ssl-settings"] as const;
export const trustStatusKey = ["trust-status"] as const;
export const systemProxyKey = ["system-proxy"] as const;
