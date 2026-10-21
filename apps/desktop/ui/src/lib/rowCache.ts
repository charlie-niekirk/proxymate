import type { RowsPage_Serialize } from "@/bindings";
import { decodeRow, type Row } from "@/lib/flow";

export const PAGE_SIZE = 128;
/** Pages further than this from the visible window are dropped. */
const KEEP_DISTANCE = 16;

export type FetchPage = (start: number, count: number) => Promise<RowsPage_Serialize>;

/**
 * Non-reactive row cache for the virtualised table.
 *
 * Rows are fetched in fixed pages with `get_rows`. Invalidated pages keep
 * serving their stale rows until the refetch lands, so live updates and
 * refilters never flash empty rows. `onUpdate` fires whenever a page lands;
 * the table re-renders from `get()`.
 */
export class RowCache {
  private pages = new Map<number, Row[]>();
  private stale = new Map<number, Row[]>();
  private inflight = new Set<number>();
  private generation = 0;
  private visible: [number, number] = [0, -1];
  len = 0;
  private readonly fetchPage: FetchPage;
  private readonly onUpdate: () => void;

  constructor(fetchPage: FetchPage, onUpdate: () => void) {
    this.fetchPage = fetchPage;
    this.onUpdate = onUpdate;
  }

  get(index: number): Row | undefined {
    if (index >= this.len) return undefined;
    const page = Math.floor(index / PAGE_SIZE);
    const offset = index % PAGE_SIZE;
    return this.pages.get(page)?.[offset] ?? this.stale.get(page)?.[offset];
  }

  /** Makes sure rows `first..=last` are loaded or loading. */
  ensure(first: number, last: number): void {
    this.visible = [first, last];
    if (this.len === 0 || last < first) return;
    // One page of read-ahead on each side keeps fast scrolls from showing gaps.
    const lastIndex = Math.min(last + PAGE_SIZE, this.len - 1);
    const firstPage = Math.floor(Math.max(0, first - PAGE_SIZE) / PAGE_SIZE);
    const lastPage = Math.floor(lastIndex / PAGE_SIZE);
    for (let p = firstPage; p <= lastPage; p++) {
      if (!this.pages.has(p) && !this.inflight.has(p)) void this.load(p);
    }
    for (const map of [this.pages, this.stale]) {
      for (const p of map.keys()) {
        if (p < firstPage - KEEP_DISTANCE || p > lastPage + KEEP_DISTANCE) map.delete(p);
      }
    }
  }

  /** Rows were inserted, removed or reordered. Keeps stale rows for display. */
  reset(len: number): void {
    this.generation++;
    this.stale = new Map([...this.stale, ...this.pages]);
    this.pages.clear();
    this.inflight.clear();
    this.len = len;
    this.reload();
  }

  /** Rows were appended at the end; refetch the page that was partially filled. */
  append(len: number): void {
    const oldLen = this.len;
    this.len = len;
    if (oldLen % PAGE_SIZE !== 0) this.invalidatePage(Math.floor(oldLen / PAGE_SIZE));
    this.reload();
  }

  /** Summaries of these flow ids changed in place. */
  invalidateIds(ids: readonly number[]): void {
    if (ids.length === 0) return;
    const wanted = new Set(ids);
    for (const [p, rows] of this.pages) {
      if (rows.some((r) => wanted.has(r.id))) this.invalidatePage(p);
    }
    this.reload();
  }

  private invalidatePage(page: number): void {
    const rows = this.pages.get(page);
    if (rows) this.stale.set(page, rows);
    this.pages.delete(page);
  }

  private reload(): void {
    this.ensure(...this.visible);
  }

  private async load(page: number): Promise<void> {
    const generation = this.generation;
    this.inflight.add(page);
    let res: RowsPage_Serialize;
    try {
      res = await this.fetchPage(page * PAGE_SIZE, PAGE_SIZE);
    } catch (err) {
      console.error("get_rows failed", err);
      return;
    } finally {
      if (generation === this.generation) this.inflight.delete(page);
    }
    if (generation !== this.generation) return;
    this.pages.set(page, res.rows.map(decodeRow));
    this.stale.delete(page);
    // Rows were appended while this page was in flight: fetch it again.
    if (res.rows.length < PAGE_SIZE && page * PAGE_SIZE + res.rows.length < this.len) {
      this.invalidatePage(page);
      this.reload();
    }
    this.onUpdate();
  }
}
