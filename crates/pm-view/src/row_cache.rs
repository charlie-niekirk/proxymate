//! The flow table's non-reactive row cache, ported from `lib/rowCache.ts`.
//!
//! Rows are fetched in fixed pages of [`PAGE_SIZE`] with `get_rows`, with
//! one page of read-ahead on each side of the visible window; pages further
//! than [`KEEP_DISTANCE`] pages from it are dropped. Invalidated pages keep
//! serving their stale rows until the refetch lands, so live updates and
//! refilters never flash empty rows.
//!
//! The cache is a plain state machine: it never fetches itself. A mutation
//! that wants rows returns `true` ("schedule a flush"); the owner then runs
//! [`RowCache::take_fetches`] once per frame, so however many scrolls and
//! deltas ask in a frame, the rows missing around the latest window go out
//! as one `get_rows` per run of adjacent pages (at most
//! [`MAX_ROWS_PER_CALL`] rows each). Each answer comes back through
//! [`RowCache::land`]. FlowGrid runs the fetches on gpui's background
//! executor; the tests run them inline.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use pm_model::FlowId;

pub const PAGE_SIZE: usize = 128;
/// Pages further than this from the visible window are dropped.
pub const KEEP_DISTANCE: usize = 16;
/// `get_rows` answers at most this many rows per call (the engine's cap).
pub const MAX_ROWS_PER_CALL: usize = 512;
/// Adjacent missing pages are fetched together, up to that cap.
pub const MAX_FETCH_PAGES: usize = MAX_ROWS_PER_CALL / PAGE_SIZE;

/// A cached row: anything that knows its flow id.
pub trait CachedRow {
    fn flow_id(&self) -> FlowId;
}

impl CachedRow for pm_model::FlowRow {
    fn flow_id(&self) -> FlowId {
        self.0
    }
}

/// One `get_rows(start, count)` the cache wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fetch {
    generation: u64,
    first_page: usize,
    pages: usize,
}

impl Fetch {
    pub fn start(&self) -> usize {
        self.first_page * PAGE_SIZE
    }

    pub fn count(&self) -> usize {
        self.pages * PAGE_SIZE
    }
}

pub struct RowCache<R> {
    pages: HashMap<usize, Arc<[R]>>,
    stale: HashMap<usize, Arc<[R]>>,
    inflight: HashSet<usize>,
    generation: u64,
    visible: Range<usize>,
    scheduled: bool,
    len: usize,
}

impl<R> Default for RowCache<R> {
    fn default() -> Self {
        Self {
            pages: HashMap::new(),
            stale: HashMap::new(),
            inflight: HashSet::new(),
            generation: 0,
            visible: 0..0,
            scheduled: false,
            len: 0,
        }
    }
}

impl<R: CachedRow> RowCache<R> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rows in the view.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The window last asked for.
    pub fn visible(&self) -> Range<usize> {
        self.visible.clone()
    }

    pub fn get(&self, index: usize) -> Option<&R> {
        if index >= self.len {
            return None;
        }
        let (page, offset) = (index / PAGE_SIZE, index % PAGE_SIZE);
        self.pages
            .get(&page)
            .and_then(|rows| rows.get(offset))
            .or_else(|| self.stale.get(&page).and_then(|rows| rows.get(offset)))
    }

    /// The index of flow `id` among the loaded rows, if it is loaded.
    pub fn index_of(&self, id: FlowId) -> Option<usize> {
        [&self.pages, &self.stale].into_iter().find_map(|map| {
            map.iter().find_map(|(p, rows)| {
                rows.iter()
                    .position(|r| r.flow_id() == id)
                    .map(|i| p * PAGE_SIZE + i)
            })
        })
    }

    /// The ids of rows `range`, when every one of them is loaded.
    pub fn ids_in(&self, range: Range<usize>) -> Option<Vec<FlowId>> {
        range.map(|i| self.get(i).map(CachedRow::flow_id)).collect()
    }

    /// Makes sure rows `range` are loaded, or will be after the next flush.
    /// True when a flush needs scheduling.
    pub fn ensure(&mut self, range: Range<usize>) -> bool {
        self.visible = range;
        let Some((first_page, last_page)) = self.wanted() else {
            return false;
        };
        let keep = |p: &usize| *p + KEEP_DISTANCE >= first_page && *p <= last_page + KEEP_DISTANCE;
        self.pages.retain(|p, _| keep(p));
        self.stale.retain(|p, _| keep(p));
        if self.scheduled {
            return false;
        }
        self.scheduled = true;
        true
    }

    /// The pages around the visible window, with one page of read-ahead on
    /// each side so fast scrolls don't show gaps.
    fn wanted(&self) -> Option<(usize, usize)> {
        let Range { start, end } = self.visible;
        if self.len == 0 || end <= start {
            return None;
        }
        let last = (end - 1 + PAGE_SIZE).min(self.len - 1);
        Some((start.saturating_sub(PAGE_SIZE) / PAGE_SIZE, last / PAGE_SIZE))
    }

    /// The flush: the wanted pages that are missing, adjacent ones
    /// together, marked in flight.
    pub fn take_fetches(&mut self) -> Vec<Fetch> {
        self.scheduled = false;
        let Some((first_page, last_page)) = self.wanted() else {
            return Vec::new();
        };
        let mut fetches = Vec::new();
        let mut run: Option<(usize, usize)> = None;
        for p in first_page..=last_page + 1 {
            let missing = p <= last_page && !self.pages.contains_key(&p) && !self.inflight.contains(&p);
            if let (true, Some((_, count))) = (missing, run.as_mut())
                && *count < MAX_FETCH_PAGES
            {
                *count += 1;
                continue;
            }
            if let Some((first, count)) = run.take() {
                fetches.push(Fetch {
                    generation: self.generation,
                    first_page: first,
                    pages: count,
                });
            }
            if missing {
                run = Some((p, 1));
            }
        }
        for f in &fetches {
            self.inflight.extend(f.first_page..f.first_page + f.pages);
        }
        fetches
    }

    /// Rows were inserted, removed or reordered (a structural delta, a new
    /// filter or sort). Keeps the old rows for display until replaced.
    pub fn reset(&mut self, len: usize) -> bool {
        self.generation += 1;
        let pages = std::mem::take(&mut self.pages);
        self.stale.extend(pages);
        self.inflight.clear();
        self.len = len;
        self.reload()
    }

    /// Rows were appended at the end: refetch the page that was partly filled.
    pub fn append(&mut self, len: usize) -> bool {
        let old = self.len;
        self.len = len;
        if !old.is_multiple_of(PAGE_SIZE) {
            self.invalidate_page(old / PAGE_SIZE);
        }
        self.reload()
    }

    /// Summaries of these flows changed in place.
    pub fn invalidate_ids(&mut self, ids: &[FlowId]) -> bool {
        if ids.is_empty() {
            return false;
        }
        let wanted: HashSet<FlowId> = ids.iter().copied().collect();
        let hit: Vec<usize> = self
            .pages
            .iter()
            .filter(|(_, rows)| rows.iter().any(|r| wanted.contains(&r.flow_id())))
            .map(|(p, _)| *p)
            .collect();
        for p in hit {
            self.invalidate_page(p);
        }
        self.reload()
    }

    fn invalidate_page(&mut self, page: usize) {
        if let Some(rows) = self.pages.remove(&page) {
            self.stale.insert(page, rows);
        }
    }

    fn reload(&mut self) -> bool {
        let visible = self.visible.clone();
        self.ensure(visible)
    }

    /// A fetch failed: its pages can be asked for again.
    pub fn fail(&mut self, fetch: Fetch) {
        if fetch.generation == self.generation {
            for p in fetch.first_page..fetch.first_page + fetch.pages {
                self.inflight.remove(&p);
            }
        }
    }

    /// A fetch's rows arrived. `None` when a reset made them stale (they are
    /// dropped); otherwise whether a flush needs scheduling (rows were
    /// appended while the page was in flight, so it goes out again).
    pub fn land(&mut self, fetch: Fetch, rows: Vec<R>) -> Option<bool> {
        if fetch.generation != self.generation {
            return None;
        }
        let mut rows = rows.into_iter();
        let mut again = false;
        for p in fetch.first_page..fetch.first_page + fetch.pages {
            self.inflight.remove(&p);
            let page: Arc<[R]> = rows.by_ref().take(PAGE_SIZE).collect();
            let short = page.len() < PAGE_SIZE && p * PAGE_SIZE + page.len() < self.len;
            self.pages.insert(p, page);
            self.stale.remove(&p);
            if short {
                self.invalidate_page(p);
                again = true;
            }
        }
        Some(again && self.reload())
    }

    /// Pages held (fresh, stale): for tests and the perf probe.
    pub fn held_pages(&self) -> (usize, usize) {
        (self.pages.len(), self.stale.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Row(FlowId);

    impl CachedRow for Row {
        fn flow_id(&self) -> FlowId {
            self.0
        }
    }

    /// A fake view over `ids`: answers fetches, recording each one.
    struct FakeView {
        ids: Vec<FlowId>,
        requests: Vec<(usize, usize)>,
    }

    impl FakeView {
        fn new(n: u32) -> Self {
            Self {
                ids: (0..n).collect(),
                requests: Vec::new(),
            }
        }

        fn rows(&mut self, f: Fetch) -> Vec<Row> {
            self.requests.push((f.start(), f.count()));
            let end = (f.start() + f.count()).min(self.ids.len());
            self.ids.get(f.start()..end).unwrap_or_default().iter().map(|&id| Row(id)).collect()
        }

        /// The next frame: every scheduled fetch, answered at once.
        fn flush(&mut self, cache: &mut RowCache<Row>) {
            for f in cache.take_fetches() {
                let rows = self.rows(f);
                cache.land(f, rows);
            }
        }
    }

    #[test]
    fn loads_only_the_pages_around_the_visible_window() {
        let mut view = FakeView::new(100_000);
        let mut cache = RowCache::new();
        cache.reset(view.ids.len());
        assert!(cache.ensure(50_000..50_041));
        view.flush(&mut cache);
        // The visible page plus one page of read-ahead on each side, in one fetch.
        assert_eq!(view.requests, [(49_792, 3 * PAGE_SIZE)]);
        assert_eq!(cache.get(50_000), Some(&Row(50_000)));
        assert_eq!(cache.get(0), None);
        assert_eq!(cache.index_of(50_001), Some(50_001));
    }

    #[test]
    fn serves_stale_rows_while_a_reset_refetches_then_replaces_them() {
        let mut view = FakeView::new(300);
        let mut cache = RowCache::new();
        cache.reset(300);
        cache.ensure(0..21);
        view.flush(&mut cache);
        view.ids.reverse();
        assert!(cache.reset(300));
        assert_eq!(cache.get(0), Some(&Row(0)), "stale, no blank flash");
        view.flush(&mut cache);
        assert_eq!(cache.get(0), Some(&Row(299)));
    }

    #[test]
    fn a_fetch_from_before_a_reset_is_dropped() {
        let mut view = FakeView::new(300);
        let mut cache = RowCache::new();
        cache.reset(300);
        cache.ensure(0..21);
        let old = cache.take_fetches();
        cache.reset(300);
        let rows = view.rows(old[0]);
        assert_eq!(cache.land(old[0], rows), None);
        assert_eq!(cache.get(0), None);
        view.flush(&mut cache);
        assert_eq!(cache.get(0), Some(&Row(0)));
    }

    #[test]
    fn refetches_the_partial_last_page_when_rows_are_appended() {
        let mut view = FakeView::new(10);
        let mut cache = RowCache::new();
        cache.reset(10);
        cache.ensure(0..41);
        view.flush(&mut cache);
        assert_eq!(cache.get(10), None);
        view.ids.extend([10, 11]);
        assert!(cache.append(12));
        assert_eq!(cache.get(5), Some(&Row(5)), "the partial page stays visible");
        view.flush(&mut cache);
        assert_eq!(cache.get(11), Some(&Row(11)));
    }

    #[test]
    fn a_page_short_of_the_view_goes_out_again() {
        let mut view = FakeView::new(10);
        let mut cache = RowCache::new();
        cache.reset(10);
        cache.ensure(0..10);
        let fetch = cache.take_fetches()[0];
        let rows = view.rows(fetch);
        // Two rows were appended while the page was in flight.
        view.ids.extend([10, 11]);
        cache.append(12);
        assert!(cache.land(fetch, rows).is_some());
        assert_eq!(cache.get(9), Some(&Row(9)));
        view.flush(&mut cache);
        assert_eq!(cache.get(11), Some(&Row(11)));
    }

    #[test]
    fn refetches_pages_holding_changed_ids() {
        let mut view = FakeView::new(PAGE_SIZE as u32 * 3);
        let mut cache = RowCache::new();
        cache.reset(view.ids.len());
        cache.ensure(0..PAGE_SIZE * 3 + 1);
        view.flush(&mut cache);
        view.requests.clear();
        assert!(cache.invalidate_ids(&[PAGE_SIZE as u32 + 5]));
        view.flush(&mut cache);
        assert_eq!(view.requests, [(PAGE_SIZE, PAGE_SIZE)]);
        assert!(!cache.invalidate_ids(&[]));
    }

    #[test]
    fn coalesces_the_asks_of_one_frame_into_fetches_for_the_latest_window() {
        let mut view = FakeView::new(100_000);
        let mut cache = RowCache::new();
        cache.reset(view.ids.len());
        let mut scheduled = 0;
        // A fast scroll: many windows before the frame, only the last one counts.
        for first in (0..20_000).step_by(500) {
            scheduled += cache.ensure(first..first + 41) as usize;
        }
        let at = 470 * PAGE_SIZE;
        scheduled += cache.ensure(at..at + PAGE_SIZE * 8) as usize;
        assert_eq!(view.requests, []);
        assert_eq!(scheduled, 1, "one flush for the frame");
        view.flush(&mut cache);
        // 10 pages wanted (8 visible and 2 read-ahead): runs of at most MAX_FETCH_PAGES.
        let counts: Vec<usize> = view.requests.iter().map(|r| r.1).collect();
        assert_eq!(counts, [4, 4, 2].map(|n| n * PAGE_SIZE));
        assert_eq!(view.requests[0].0, at - PAGE_SIZE);
        assert!(counts.iter().all(|&c| c <= MAX_ROWS_PER_CALL));
        assert_eq!(cache.get(at), Some(&Row(at as u32)));
        let last = at + PAGE_SIZE * 9 - 1;
        assert_eq!(cache.get(last), Some(&Row(last as u32)));
    }

    #[test]
    fn evicts_pages_far_from_the_window() {
        let mut view = FakeView::new(100_000);
        let mut cache = RowCache::new();
        cache.reset(100_000);
        cache.ensure(0..40);
        view.flush(&mut cache);
        assert!(cache.get(0).is_some());
        cache.ensure(60_000..60_040);
        assert_eq!(cache.get(0), None, "page 0 is more than 16 pages away");
        view.flush(&mut cache);
        assert_eq!(cache.held_pages(), (4, 0), "pages 467 to 470");
    }

    #[test]
    fn a_failed_fetch_can_be_asked_for_again() {
        let mut view = FakeView::new(300);
        let mut cache = RowCache::new();
        cache.reset(300);
        cache.ensure(0..20);
        let fetches = cache.take_fetches();
        assert_eq!(cache.take_fetches(), [], "in flight");
        cache.fail(fetches[0]);
        assert!(cache.ensure(0..20));
        view.flush(&mut cache);
        assert_eq!(cache.ids_in(0..3), Some(vec![0, 1, 2]));
        assert_eq!(cache.ids_in(0..300), None);
    }
}
