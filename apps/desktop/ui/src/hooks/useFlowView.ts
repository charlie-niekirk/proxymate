import { useEffect, useReducer, useRef, useState } from "react";

import type { ViewDelta } from "@/bindings";
import { backend } from "@/lib/backend";
import { flowKey, queryClient, streamKey } from "@/lib/query";
import { RowCache } from "@/lib/rowCache";
import { useUi } from "@/store/ui";

interface Live {
  view: number;
  cache: RowCache;
}

/**
 * Owns one engine view for the flow table: creates it over the active store
 * (again when the tab changes), follows its 20 Hz deltas over a Channel,
 * applies filter/sort changes and keeps the row cache in sync. Returns the
 * cache (null until the view exists), the view id and a version that bumps
 * whenever rows land or the view changes.
 */
export function useFlowView() {
  const filter = useUi((s) => s.filter);
  const sort = useUi((s) => s.sort);
  const store = useUi((s) => s.store);
  const [version, bump] = useReducer((n: number) => n + 1, 0);
  const [live, setLive] = useState<Live | null>(null);
  // Filter change start time, until the first rows of the new result are painted.
  const pendingFilter = useRef<{ t0: number; engineMs: number } | null>(null);
  const initial = useRef({ filter, sort });

  // Create the view (once per store) and subscribe to its deltas.
  useEffect(() => {
    let cancelled = false;
    let created: number | null = null;
    const { filter, sort } = useUi.getState();
    initial.current = { filter, sort };
    void (async () => {
      const info = await backend.createView(store, filter, sort);
      created = info.view;
      if (cancelled) {
        void backend.dropView(info.view);
        return;
      }
      const cache = new RowCache(
        (start, count) => backend.getRows(info.view, start, count),
        () => {
          bump();
          const pending = pendingFilter.current;
          if (pending) {
            pendingFilter.current = null;
            reportFilterTiming(pending);
          }
        },
      );
      cache.reset(info.len);
      useUi.getState().setStats({ len: info.len, total: info.total });
      setLive({ view: info.view, cache });
      useUi.getState().setView(info.view);
      await backend.subscribe(info.view, (d: ViewDelta) => {
        if (cancelled) return;
        if (d.structural) cache.reset(d.len);
        else {
          if (d.changed.length > 0) cache.invalidateIds(d.changed);
          if (d.len !== cache.len) cache.append(d.len);
        }
        useUi.getState().setStats({ len: d.len, total: d.total });
        refreshInspected(store, d);
        bump();
      });
    })();
    return () => {
      cancelled = true;
      setLive(null);
      if (created !== null) void backend.dropView(created);
    };
  }, [store]);

  // Filter changes: refilter synchronously in Rust, then refetch visible rows.
  const filterSeq = useRef(0);
  useEffect(() => {
    if (live === null || filter === initial.current.filter) return;
    const seq = ++filterSeq.current;
    const t0 = performance.now();
    void backend.setFilter(live.view, filter).then((info) => {
      if (seq !== filterSeq.current) return;
      const pending = { t0, engineMs: info.elapsed_us / 1000 };
      if (info.len === 0) reportFilterTiming(pending);
      else pendingFilter.current = pending;
      live.cache.reset(info.len);
      useUi.getState().setStats({ len: info.len, total: info.total });
      bump();
    });
  }, [live, filter]);

  useEffect(() => {
    if (live === null || sort === initial.current.sort) return;
    void backend.setSort(live.view, sort).then((info) => {
      live.cache.reset(info.len);
      bump();
    });
  }, [live, sort]);

  return { cache: live?.cache ?? null, view: live?.view ?? null, version };
}

/** Refetches the inspected flow if this delta may have changed it. */
function refreshInspected(store: number, d: ViewDelta) {
  const { selectedId, select } = useUi.getState();
  if (selectedId === null) return;
  if (d.total === 0) {
    // Cleared.
    select(null);
    return;
  }
  // A selection made before the flow reached the store found nothing; retry as rows arrive.
  const missing = d.appended > 0 && queryClient.getQueryData(flowKey(store, selectedId)) === null;
  if (d.structural || d.changed.includes(selectedId) || missing) {
    void queryClient.invalidateQueries({ queryKey: flowKey(store, selectedId) });
    void queryClient.invalidateQueries({ queryKey: streamKey(store, selectedId) });
  }
}

/**
 * Records filter-change-to-paint latency. The task queued from the next
 * animation frame runs after that frame (with the new rows) is painted.
 */
function reportFilterTiming({ t0, engineMs }: { t0: number; engineMs: number }) {
  requestAnimationFrame(() =>
    setTimeout(() => {
      const endToEndMs = performance.now() - t0;
      useUi.getState().setFilterTiming({ engineMs, endToEndMs });
      if (import.meta.env.DEV) {
        console.debug(`filter: engine ${engineMs.toFixed(2)} ms, end to end ${endToEndMs.toFixed(1)} ms`);
      }
    }, 0),
  );
}
