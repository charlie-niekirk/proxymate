//! Incrementally maintained, filtered and sorted views over the summary arena.
//!
//! Views hold arena positions, not flow ids: after a clear or an eviction
//! the positions change and views rebuild, while ids keep counting up.
//!
//! A filter with header or body terms makes a deep view: its rows are the
//! flows that match the rest of the filter *and* a deep search found to
//! match. The view queues candidates (finished flows; in-flight ones once
//! they finish) and the engine's job reports what it found.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use std::cmp::Ordering;

use pm_model::{
    DeepProgress, FilterSpec, FlowId, FlowRow, FlowState, FlowSummary, SortDir, SortKey, SortSpec, ViewDelta, ViewId,
};

/// Index into the summary arena.
pub type Pos = u32;
use rayon::prelude::*;

use super::deep::DeepQuery;
use super::filter::CompiledFilter;
use crate::openapi::OpIndex;

/// Above this many flows a full refilter or sort runs on the rayon pool.
const PARALLEL_THRESHOLD: usize = 50_000;

/// A deep view's search state.
pub struct DeepState {
    query: Arc<DeepQuery>,
    /// Set when the filter changes or the view goes: the job stops.
    cancel: Arc<AtomicBool>,
    /// Candidates not handed to the job yet, oldest first.
    pending: VecDeque<FlowId>,
    /// Flows found to match.
    matched: HashSet<FlowId>,
    checked: u32,
    total: u32,
    /// Handed to the job, not reported back yet.
    in_flight: u32,
    dirty: bool,
}

impl Drop for DeepState {
    fn drop(&mut self) {
        self.cancel.store(true, AtomicOrdering::Release);
    }
}

impl DeepState {
    fn progress(&self) -> DeepProgress {
        DeepProgress {
            checked: self.checked,
            total: self.total,
            matched: self.matched.len() as u32,
            running: !self.pending.is_empty() || self.in_flight > 0,
        }
    }

    fn queue(&mut self, id: FlowId) {
        self.pending.push_back(id);
        self.total += 1;
        self.dirty = true;
    }
}

/// What a deep search job needs to run: the terms and its stop flag.
pub struct DeepJob {
    pub query: Arc<DeepQuery>,
    pub cancel: Arc<AtomicBool>,
}

fn is_finished(f: &FlowSummary) -> bool {
    !matches!(f.state, FlowState::Active | FlowState::Paused)
}

/// One live view: `{CompiledFilter, SortSpec, indices}` plus the change
/// accumulated since the last published [`ViewDelta`].
pub struct View {
    id: ViewId,
    spec: FilterSpec,
    filter: CompiledFilter,
    sort: SortSpec,
    /// Arena positions in display order.
    indices: Vec<Pos>,
    /// Membership bitset indexed by position, for O(1) "is it in the view?".
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
    deep: Option<DeepState>,
}

impl View {
    pub fn new(
        id: ViewId,
        spec: FilterSpec,
        sort: SortSpec,
        flows: &[FlowSummary],
        operations: Option<&Arc<OpIndex>>,
    ) -> Self {
        let mut view = View {
            id,
            filter: CompiledFilter::compile_with(&spec, operations),
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
            deep: None,
        };
        view.start_deep(flows);
        view.rebuild(flows);
        view
    }

    /// Starts a deep search if the filter has header or body terms: every
    /// finished flow matching the rest of the filter becomes a candidate.
    fn start_deep(&mut self, flows: &[FlowSummary]) {
        self.deep = self.filter.deep().map(|query| {
            let pending: VecDeque<FlowId> = flows
                .iter()
                .filter(|f| is_finished(f) && self.filter.matches(f))
                .map(|f| f.id)
                .collect();
            DeepState {
                query: query.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                total: pending.len() as u32,
                pending,
                matched: HashSet::new(),
                checked: 0,
                in_flight: 0,
                dirty: true,
            }
        });
    }

    /// The job to spawn for this view's deep search, if it has one.
    pub fn deep_job(&self) -> Option<DeepJob> {
        self.deep.as_ref().map(|d| DeepJob {
            query: d.query.clone(),
            cancel: d.cancel.clone(),
        })
    }

    pub fn deep_progress(&self) -> Option<DeepProgress> {
        self.deep.as_ref().map(DeepState::progress)
    }

    /// Hands up to `max` candidates to the job (oldest first). `None` if
    /// the search behind `cancel` is no longer this view's.
    pub fn take_deep_batch(&mut self, cancel: &Arc<AtomicBool>, max: usize) -> Option<Vec<FlowId>> {
        let deep = self.deep.as_mut().filter(|d| Arc::ptr_eq(&d.cancel, cancel))?;
        let n = deep.pending.len().min(max);
        let batch: Vec<FlowId> = deep.pending.drain(..n).collect();
        deep.in_flight += batch.len() as u32;
        Some(batch)
    }

    /// Takes the job's verdict on `checked` candidates: `hits` (with their
    /// arena positions, if still there) join the view.
    pub fn deep_results(
        &mut self,
        flows: &[FlowSummary],
        cancel: &Arc<AtomicBool>,
        checked: u32,
        hits: &[(FlowId, Option<Pos>)],
    ) {
        let Some(deep) = self.deep.as_mut().filter(|d| Arc::ptr_eq(&d.cancel, cancel)) else {
            return;
        };
        deep.in_flight = deep.in_flight.saturating_sub(checked);
        // A batch handed out before a clear reports after it.
        deep.checked = (deep.checked + checked).min(deep.total);
        deep.dirty = true;
        let mut in_order = true;
        let mut added = 0u32;
        for &(id, pos) in hits {
            deep.matched.insert(id);
            let Some(pos) = pos else { continue };
            if get_bit(&self.members, pos) || !self.filter.matches(&flows[pos as usize]) {
                continue;
            }
            in_order &= self.indices.last().is_none_or(|&l| pos > l);
            self.indices.push(pos);
            set_bit(&mut self.members, pos);
            added += 1;
        }
        if added == 0 {
            return;
        }
        if self.is_arrival_order() && in_order {
            self.appended += added;
        } else {
            self.sort_indices(flows);
            self.mark_structural();
        }
    }

    /// Forgets the deep search's candidates and matches (a clear).
    pub fn reset_deep(&mut self) {
        if let Some(deep) = &mut self.deep {
            deep.pending.clear();
            deep.matched.clear();
            deep.checked = 0;
            deep.total = 0;
            deep.in_flight = 0;
            deep.dirty = true;
        }
    }

    /// Forgets evicted flows' deep matches (`evicted` is sorted).
    pub fn forget_deep(&mut self, evicted: &[FlowId]) {
        if let Some(deep) = &mut self.deep {
            deep.matched.retain(|id| evicted.binary_search(id).is_err());
            deep.dirty = true;
        }
    }

    /// The filter, deep search included, accepts `f`.
    fn accepts(&self, f: &FlowSummary) -> bool {
        self.filter.matches(f) && self.deep.as_ref().is_none_or(|d| d.matched.contains(&f.id))
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

    pub fn set_filter(&mut self, spec: FilterSpec, flows: &[FlowSummary], operations: Option<&Arc<OpIndex>>) {
        self.filter = CompiledFilter::compile_with(&spec, operations);
        self.spec = spec;
        self.start_deep(flows);
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
            (0..flows.len() as Pos).collect()
        } else if flows.len() >= PARALLEL_THRESHOLD {
            flows
                .par_iter()
                .enumerate()
                .filter(|(_, f)| self.accepts(f))
                .map(|(pos, _)| pos as Pos)
                .collect()
        } else {
            (0..flows.len())
                .filter(|&pos| self.accepts(&flows[pos]))
                .map(|pos| pos as Pos)
                .collect()
        };
        self.sort_indices(flows);
        self.members = vec![0; flows.len().div_ceil(64)];
        for &pos in &self.indices {
            set_bit(&mut self.members, pos);
        }
        self.needs_rebuild = false;
        self.mark_structural();
    }

    /// Called after `flows[first..]` were appended to the arena.
    pub fn on_insert(&mut self, flows: &[FlowSummary], first: usize) {
        let before = self.indices.len();
        for (pos, f) in flows.iter().enumerate().skip(first) {
            if !self.filter.matches(f) {
                continue;
            }
            if let Some(deep) = &mut self.deep {
                // Joins the view once the deep search finds it matches.
                if is_finished(f) {
                    deep.queue(f.id);
                }
                continue;
            }
            self.indices.push(pos as Pos);
            set_bit(&mut self.members, pos as Pos);
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

    /// Called after the flow at `pos` was replaced by `new`.
    pub fn on_update(&mut self, pos: Pos, old: &FlowSummary, new: &FlowSummary) {
        // A flow becomes a candidate once it is finished and passes the rest
        // of the filter: on finishing, or later (its client resolved, a pin).
        if let Some(deep) = &mut self.deep
            && is_finished(new)
            && self.filter.matches(new)
            && !(is_finished(old) && self.filter.matches(old))
        {
            deep.queue(new.id);
        }
        let was = get_bit(&self.members, pos);
        let is = self.accepts(new);
        match (was, is) {
            (true, true) => {
                if !self.is_arrival_order() && compare_key(self.sort, old, new) != Ordering::Equal {
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
        let deep_dirty = self.deep.as_mut().is_some_and(|d| std::mem::take(&mut d.dirty));
        if !self.structural && self.appended == 0 && self.changed.is_empty() && total == self.last_total && !deep_dirty
        {
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
            deep: self.deep_progress(),
        };
        Some(delta)
    }

    /// Arena positions of the view's rows, in display order.
    pub fn positions(&self) -> &[Pos] {
        &self.indices
    }

    pub fn rows(&self, flows: &[FlowSummary], start: u32, count: u32) -> Vec<FlowRow> {
        let start = (start as usize).min(self.indices.len());
        let end = start.saturating_add(count as usize).min(self.indices.len());
        self.indices[start..end]
            .iter()
            .map(|&pos| FlowRow::from(&flows[pos as usize]))
            .collect()
    }

    fn is_arrival_order(&self) -> bool {
        self.sort.key == SortKey::Id && self.sort.dir == SortDir::Asc
    }

    fn sort_indices(&mut self, flows: &[FlowSummary]) {
        self.needs_resort = false;
        let SortSpec { key, dir, extra } = self.sort;
        if key == SortKey::Id {
            // Positions are pushed in arrival order, which is id order.
            if dir == SortDir::Asc {
                self.indices.sort_unstable();
            } else {
                self.indices.sort_unstable_by(|a, b| b.cmp(a));
            }
            return;
        }
        let cmp = |a: &Pos, b: &Pos| {
            let (fa, fb) = (&flows[*a as usize], &flows[*b as usize]);
            let ord = compare_key(SortSpec { key, dir, extra }, fa, fb).then(a.cmp(b));
            if dir == SortDir::Desc { ord.reverse() } else { ord }
        };
        if self.indices.len() >= PARALLEL_THRESHOLD {
            self.indices.par_sort_unstable_by(cmp);
        } else {
            self.indices.sort_unstable_by(cmp);
        }
    }
}

fn compare_key(sort: SortSpec, a: &FlowSummary, b: &FlowSummary) -> Ordering {
    match sort.key {
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
        // Flows without the header sort first, like a missing status.
        SortKey::Extra => {
            let at = |f: &FlowSummary| f.extra.get(sort.extra as usize).cloned().flatten();
            at(a).cmp(&at(b))
        }
    }
}

fn set_bit(bits: &mut Vec<u64>, pos: Pos) {
    let word = pos as usize / 64;
    if word >= bits.len() {
        bits.resize(word + 1, 0);
    }
    bits[word] |= 1 << (pos % 64);
}

fn get_bit(bits: &[u64], pos: Pos) -> bool {
    bits.get(pos as usize / 64).is_some_and(|w| w & (1 << (pos % 64)) != 0)
}
