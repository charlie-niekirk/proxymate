//! Incrementally maintained, filtered and sorted views over the summary arena.

use std::cmp::Ordering;

use pm_model::{FilterSpec, FlowId, FlowRow, FlowSummary, SortDir, SortKey, SortSpec, ViewDelta, ViewId};
use rayon::prelude::*;

use super::filter::CompiledFilter;

/// Above this many flows a full refilter or sort runs on the rayon pool.
const PARALLEL_THRESHOLD: usize = 50_000;

/// One live view: `{CompiledFilter, SortSpec, indices}` plus the change
/// accumulated since the last published [`ViewDelta`].
pub struct View {
    id: ViewId,
    spec: FilterSpec,
    filter: CompiledFilter,
    sort: SortSpec,
    /// Flow ids in display order.
    indices: Vec<FlowId>,
    /// Membership bitset indexed by flow id, for O(1) "is it in the view?".
    members: Vec<u64>,
    // Pending delta.
    seq: u32,
    appended: u32,
    changed: Vec<FlowId>,
    structural: bool,
    last_total: u32,
    // Deferred work for the end of an ingest batch.
    needs_resort: bool,
    needs_rebuild: bool,
}

impl View {
    pub fn new(id: ViewId, spec: FilterSpec, sort: SortSpec, flows: &[FlowSummary]) -> Self {
        let mut view = View {
            id,
            filter: CompiledFilter::compile(&spec),
            spec,
            sort,
            indices: Vec::new(),
            members: Vec::new(),
            seq: 0,
            appended: 0,
            changed: Vec::new(),
            structural: true,
            last_total: u32::MAX,
            needs_resort: false,
            needs_rebuild: false,
        };
        view.rebuild(flows);
        view
    }

    pub fn len(&self) -> u32 {
        self.indices.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn spec(&self) -> &FilterSpec {
        &self.spec
    }

    pub fn set_filter(&mut self, spec: FilterSpec, flows: &[FlowSummary]) {
        self.filter = CompiledFilter::compile(&spec);
        self.spec = spec;
        self.rebuild(flows);
    }

    pub fn set_sort(&mut self, sort: SortSpec, flows: &[FlowSummary]) {
        self.sort = sort;
        self.sort_indices(flows);
        self.mark_structural();
    }

    /// Forces the next delta to be structural (e.g. a new subscriber).
    pub fn mark_structural(&mut self) {
        self.structural = true;
        self.appended = 0;
        self.changed.clear();
    }

    /// Full linear refilter and sort.
    pub fn rebuild(&mut self, flows: &[FlowSummary]) {
        self.indices = if self.filter.is_match_all() {
            (0..flows.len() as FlowId).collect()
        } else if flows.len() >= PARALLEL_THRESHOLD {
            flows
                .par_iter()
                .filter(|f| self.filter.matches(f))
                .map(|f| f.id)
                .collect()
        } else {
            flows.iter().filter(|f| self.filter.matches(f)).map(|f| f.id).collect()
        };
        self.sort_indices(flows);
        self.members = vec![0; flows.len().div_ceil(64)];
        for &id in &self.indices {
            set_bit(&mut self.members, id);
        }
        self.needs_rebuild = false;
        self.mark_structural();
    }

    /// Called after `flows[first..]` were appended to the arena.
    pub fn on_insert(&mut self, flows: &[FlowSummary], first: usize) {
        let before = self.indices.len();
        for f in &flows[first..] {
            if self.filter.matches(f) {
                self.indices.push(f.id);
                set_bit(&mut self.members, f.id);
            }
        }
        let added = (self.indices.len() - before) as u32;
        if added == 0 {
            return;
        }
        if self.is_arrival_order() {
            self.appended += added;
        } else {
            self.needs_resort = true;
        }
    }

    /// Called after a flow already in the arena was replaced by `new`.
    pub fn on_update(&mut self, old: &FlowSummary, new: &FlowSummary) {
        let was = get_bit(&self.members, new.id);
        let is = self.filter.matches(new);
        match (was, is) {
            (true, true) => {
                if !self.is_arrival_order() && compare_key(self.sort.key, old, new) != Ordering::Equal {
                    self.needs_resort = true;
                } else {
                    self.changed.push(new.id);
                }
            }
            (false, false) => {}
            // Membership changed: rows move, so rebuild once at the end of the batch.
            _ => self.needs_rebuild = true,
        }
    }

    /// Applies deferred work at the end of an ingest batch.
    pub fn finish_batch(&mut self, flows: &[FlowSummary]) {
        if self.needs_rebuild {
            self.rebuild(flows);
        } else if self.needs_resort {
            self.sort_indices(flows);
            self.mark_structural();
        }
    }

    /// Drains the pending change into a delta, if there is anything to report.
    pub fn take_delta(&mut self, total: u32) -> Option<ViewDelta> {
        if !self.structural && self.appended == 0 && self.changed.is_empty() && total == self.last_total {
            return None;
        }
        self.seq += 1;
        self.last_total = total;
        let mut changed = std::mem::take(&mut self.changed);
        changed.sort_unstable();
        changed.dedup();
        let delta = ViewDelta {
            view: self.id,
            seq: self.seq,
            total,
            len: self.len(),
            appended: std::mem::take(&mut self.appended),
            changed,
            structural: std::mem::take(&mut self.structural),
        };
        Some(delta)
    }

    pub fn rows(&self, flows: &[FlowSummary], start: u32, count: u32) -> Vec<FlowRow> {
        let start = (start as usize).min(self.indices.len());
        let end = start.saturating_add(count as usize).min(self.indices.len());
        self.indices[start..end]
            .iter()
            .map(|&id| FlowRow::from(&flows[id as usize]))
            .collect()
    }

    fn is_arrival_order(&self) -> bool {
        self.sort.key == SortKey::Id && self.sort.dir == SortDir::Asc
    }

    fn sort_indices(&mut self, flows: &[FlowSummary]) {
        self.needs_resort = false;
        let SortSpec { key, dir } = self.sort;
        if key == SortKey::Id {
            // Ids are pushed in arrival order, so indices are already ascending.
            if dir == SortDir::Asc {
                self.indices.sort_unstable();
            } else {
                self.indices.sort_unstable_by(|a, b| b.cmp(a));
            }
            return;
        }
        let cmp = |a: &FlowId, b: &FlowId| {
            let (fa, fb) = (&flows[*a as usize], &flows[*b as usize]);
            let ord = compare_key(key, fa, fb).then(a.cmp(b));
            if dir == SortDir::Desc { ord.reverse() } else { ord }
        };
        if self.indices.len() >= PARALLEL_THRESHOLD {
            self.indices.par_sort_unstable_by(cmp);
        } else {
            self.indices.sort_unstable_by(cmp);
        }
    }
}

fn compare_key(key: SortKey, a: &FlowSummary, b: &FlowSummary) -> Ordering {
    match key {
        SortKey::Id => a.id.cmp(&b.id),
        SortKey::Url => a
            .host
            .cmp(&b.host)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.query.cmp(&b.query)),
        SortKey::Method => a.method.cmp(&b.method),
        SortKey::Status => a.status.cmp(&b.status),
        SortKey::Duration => a.duration_ms.cmp(&b.duration_ms),
        SortKey::ReqSize => a.req_size.cmp(&b.req_size),
        SortKey::RespSize => a.resp_size.cmp(&b.resp_size),
    }
}

fn set_bit(bits: &mut Vec<u64>, id: FlowId) {
    let word = id as usize / 64;
    if word >= bits.len() {
        bits.resize(word + 1, 0);
    }
    bits[word] |= 1 << (id % 64);
}

fn get_bit(bits: &[u64], id: FlowId) -> bool {
    bits.get(id as usize / 64).is_some_and(|w| w & (1 << (id % 64)) != 0)
}
