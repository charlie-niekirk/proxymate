import { useEffect, useReducer, useRef, useState } from "react";

import type { FilterSpec_Deserialize, ViewDelta_Serialize } from "@/bindings";
import { errorText, reportError } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { flowKey, queryClient, streamKey } from "@/lib/query";
import { RowCache } from "@/lib/rowCache";
import { useUi } from "@/store/ui";

interface Live {
  view: number;
  cache: RowCache;
}

/**
 * Typing in the search box refilters once the keys pause this long (R2);
 * chips, scopes and clearing the box apply at once.
 */
export const FILTER_DEBOUNCE_MS = 120;

/** True if `next` differs from `prev` only in a non-empty search text. */
function typedOnly(prev: FilterSpec_Deserialize, next: FilterSpec_Deserialize): boolean {
  return (
    next.text !== "" &&
    prev.protocol === next.protocol &&
    prev.host === next.host &&
    prev.client === next.client &&
    prev.content.length === next.content.length &&
    prev.content.every((c, i) => next.content[i] === c)
  );
}

/**
 * Owns one engine view for the flow table: creates it over the active store
 * (again when the tab changes), follows its 20 Hz deltas over a Channel,
 * applies filter/sort changes and keeps the row cache in sync. Returns the
 * cache (null until the view exists), the view id, a version that bumps
 * whenever rows land or the view changes, and why the view couldn't be
 * opened (with `retry`), so the table never waits for traffic that can't come.
 */
export function useFlowView() {
  const filter = useUi((s) => s.filter);
  const sort = useUi((s) => s.sort);
  const store = useUi((s) => s.store);
  const [version, bump] = useReducer((n: number) => n + 1, 0);
  const [live, setLive] = useState<Live | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, retry] = useReducer((n: number) => n + 1, 0);
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
        void backend.dropView(info.view).catch(() => {});
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
      useUi.getState().setDeep(info.deep);
      setLive({ view: info.view, cache });
      useUi.getState().setView(info.view);
      await backend.subscribe(info.view, (d: ViewDelta_Serialize) => {
        if (cancelled) return;
        if (d.structural) cache.reset(d.len);
        else {
          if (d.changed.length > 0) cache.invalidateIds(d.changed);
          if (d.len !== cache.len) cache.append(d.len);
        }
        useUi.getState().setStats({ len: d.len, total: d.total });
        useUi.getState().setDeep(d.deep);
        refreshInspected(store, d);
        bump();
      });
    })().catch((err: unknown) => {
      if (cancelled) return;
      setError(errorText(err));
      reportError("Couldn't open the flow list", err);
    });
    // The sidebar counts the store the table shows.
    backend.setSidebarStore(store).catch((err: unknown) => reportError("Couldn't update the sidebar", err));
    return () => {
      cancelled = true;
      setLive(null);
      // A new store or a retry starts without the old error.
      setError(null);
      if (created !== null) void backend.dropView(created).catch(() => {});
    };
  }, [store, attempt]);

  // Filter changes: refilter synchronously in Rust, then refetch visible rows.
  // Typing is debounced; the timing starts when the filter is sent.
  const filterSeq = useRef(0);
  const sentFilter = useRef(filter);
  useEffect(() => {
    if (live === null) return;
    if (filter === initial.current.filter) {
      sentFilter.current = filter;
      return;
    }
    const apply = () => {
      sentFilter.current = filter;
      const seq = ++filterSeq.current;
      const t0 = performance.now();
      backend.setFilter(live.view, filter).then((info) => {
        if (seq !== filterSeq.current) return;
        const pending = { t0, engineMs: info.elapsed_us / 1000 };
        if (info.len === 0) reportFilterTiming(pending);
        else pendingFilter.current = pending;
        live.cache.reset(info.len);
        useUi.getState().setStats({ len: info.len, total: info.total });
        useUi.getState().setDeep(info.deep);
        bump();
      }, reportError.bind(null, "Couldn't apply the filter"));
    };
    if (!typedOnly(sentFilter.current, filter)) {
      apply();
      return;
    }
    const timer = setTimeout(apply, FILTER_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [live, filter]);

  // Like filters, only the latest sort's reply may reset the rows.
  const sortSeq = useRef(0);
  useEffect(() => {
    if (live === null || sort === initial.current.sort) return;
    const seq = ++sortSeq.current;
    backend.setSort(live.view, sort).then((info) => {
      if (seq !== sortSeq.current) return;
      live.cache.reset(info.len);
      bump();
    }, reportError.bind(null, "Couldn't sort the flows"));
  }, [live, sort]);

  return { cache: live?.cache ?? null, view: live?.view ?? null, version, error, retry };
}

/** Refetches the inspected flow if this delta may have changed it. */
function refreshInspected(store: number, d: ViewDelta_Serialize) {
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
