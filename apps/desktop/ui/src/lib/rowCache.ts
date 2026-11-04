import type { RowsPage_Serialize } from "@/bindings";
import { decodeRow, type Row } from "@/lib/flow";

export const PAGE_SIZE = 128;
/** Pages further than this from the visible window are dropped. */
const KEEP_DISTANCE = 16;
/** `get_rows` answers at most this many rows per call (the engine's cap). */
export const MAX_ROWS_PER_CALL = 512;
/** Adjacent missing pages are fetched together, up to that cap. */
export const MAX_FETCH_PAGES = MAX_ROWS_PER_CALL / PAGE_SIZE;

export type FetchPage = (start: number, count: number) => Promise<RowsPage_Serialize>;
/** Runs `run` once before the next paint (the default) or whenever a test says. */
export type Schedule = (run: () => void) => void;

/**
 * The next frame. Measured in R2 against coalescing at the end of each task:
 * no difference in the perf probe's scroll runs, fast flings included.
 */
const nextFrame: Schedule = (run) => {
  if (typeof requestAnimationFrame === "function") requestAnimationFrame(run);
  else setTimeout(run, 0);
};

/**
 * Non-reactive row cache for the virtualised table.
 *
 * Rows are fetched in fixed pages with `get_rows`. Invalidated pages keep
 * serving their stale rows until the refetch lands, so live updates and
 * refilters never flash empty rows. `onUpdate` fires whenever a page lands;
 * the table re-renders from `get()`.
 *
 * Fetches are coalesced per animation frame (R2): however many scroll
 * events and deltas ask in one frame, the rows missing around the latest
 * visible window go out as one `get_rows` per run of adjacent pages.
 */
export class RowCache {
  private pages = new Map<number, Row[]>();
  private stale = new Map<number, Row[]>();
  private inflight = new Set<number>();
  private generation = 0;
  private visible: [number, number] = [0, -1];
  private scheduled = false;
  len = 0;
  private readonly fetchPage: FetchPage;
  private readonly onUpdate: () => void;
  private readonly schedule: Schedule;

  constructor(fetchPage: FetchPage, onUpdate: () => void, schedule: Schedule = nextFrame) {
    this.fetchPage = fetchPage;
    this.onUpdate = onUpdate;
    this.schedule = schedule;
  }

  get(index: number): Row | undefined {
    if (index >= this.len) return undefined;
    const page = Math.floor(index / PAGE_SIZE);
    const offset = index % PAGE_SIZE;
    return this.pages.get(page)?.[offset] ?? this.stale.get(page)?.[offset];
  }

  /** The index of flow `id` among the loaded rows, if it is loaded. */
  indexOf(id: number): number | undefined {
    for (const map of [this.pages, this.stale]) {
      for (const [p, rows] of map) {
        const i = rows.findIndex((r) => r.id === id);
        if (i >= 0) return p * PAGE_SIZE + i;
      }
    }
    return undefined;
  }

  /** Makes sure rows `first..=last` are loaded, or will be by the next frame. */
  ensure(first: number, last: number): void {
    this.visible = [first, last];
    const range = this.wanted();
    if (!range) return;
    const [firstPage, lastPage] = range;
    for (const map of [this.pages, this.stale]) {
      for (const p of map.keys()) {
        if (p < firstPage - KEEP_DISTANCE || p > lastPage + KEEP_DISTANCE) map.delete(p);
      }
    }
    if (this.scheduled) return;
    this.scheduled = true;
    this.schedule(() => {
      this.scheduled = false;
      this.fetchMissing();
    });
  }

  /** The pages around the visible window, with one page of read-ahead on each side so fast scrolls don't show gaps. */
  private wanted(): [number, number] | null {
    const [first, last] = this.visible;
    if (this.len === 0 || last < first) return null;
    const lastIndex = Math.min(last + PAGE_SIZE, this.len - 1);
    return [Math.floor(Math.max(0, first - PAGE_SIZE) / PAGE_SIZE), Math.floor(lastIndex / PAGE_SIZE)];
  }

  /** Fetches the wanted pages that are missing, adjacent ones together. */
  private fetchMissing(): void {
    const range = this.wanted();
    if (!range) return;
    const [firstPage, lastPage] = range;
    let run: number | null = null;
    let count = 0;
    for (let p = firstPage; p <= lastPage + 1; p++) {
      const missing = p <= lastPage && !this.pages.has(p) && !this.inflight.has(p);
      if (missing && run !== null && count < MAX_FETCH_PAGES) {
        count++;
        continue;
      }
      if (run !== null) void this.load(run, count);
      run = missing ? p : null;
      count = missing ? 1 : 0;
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

  /** Loads `count` pages from `first` in one fetch. */
  private async load(first: number, count: number): Promise<void> {
    const generation = this.generation;
    const pages = Array.from({ length: count }, (_, i) => first + i);
    for (const p of pages) this.inflight.add(p);
    let res: RowsPage_Serialize;
    try {
      res = await this.fetchPage(first * PAGE_SIZE, count * PAGE_SIZE);
    } catch (err) {
      console.error("get_rows failed", err);
      return;
    } finally {
      if (generation === this.generation) for (const p of pages) this.inflight.delete(p);
    }
    if (generation !== this.generation) return;
    const rows = res.rows.map(decodeRow);
    let again = false;
    for (const p of pages) {
      const offset = (p - first) * PAGE_SIZE;
      const pageRows = rows.slice(offset, offset + PAGE_SIZE);
      this.pages.set(p, pageRows);
      this.stale.delete(p);
      // Rows were appended while this page was in flight: fetch it again.
      if (pageRows.length < PAGE_SIZE && p * PAGE_SIZE + pageRows.length < this.len) {
        this.invalidatePage(p);
        again = true;
      }
    }
    if (again) this.reload();
    this.onUpdate();
  }
}
