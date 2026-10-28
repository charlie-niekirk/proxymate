//! Flow stores: the summary arena plus the views over it, and the body and
//! detail storage beside it. The live capture and every opened document
//! (a saved session or an imported HAR) each have a [`Store`]; their
//! details and bodies come from a [`BlobBackend`].
//!
//! Flow ids are never reused and the arena stays in id order. A clear
//! empties it; the flow cap evicts the oldest flows but keeps pinned and
//! in-flight ones, so after an eviction a few old flows may sit before a
//! gap. Lookups by id count back from the newest flow, and fall back to a
//! binary search for those.

mod backend;
mod blob;
mod deep;
mod detail;
mod filter;
mod messages;
mod query;
mod segments;
mod sidebar;
mod sqlite;
mod view;

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::SyncSender;

use pm_model::{FilterSpec, FlowId, FlowState, FlowSummary, RowsPage, SortSpec, ViewDelta, ViewId, ViewInfo};

pub(crate) use backend::BlobBackend;
pub(crate) use blob::Capture;
pub use deep::DeepQuery;
pub(crate) use detail::{DetailStore, StoredDetail};
pub use filter::CompiledFilter;
pub(crate) use messages::{SseLog, WsLog, page_events, sse_event};
pub use query::FlowQuery;
pub(crate) use segments::SegmentStore;
pub use sidebar::{SidebarCounts, in_domain, registrable_domain};
pub(crate) use sqlite::{Opened, SavedFlow, SavedWs, SessionWriter, SqliteBackend};
pub use view::{DeepJob, View};

use crate::Error;
use crate::ingest::FlowEvent;

#[derive(Default)]
pub struct Store {
    /// The id the next appended flow gets.
    next: FlowId,
    flows: Vec<FlowSummary>,
    views: HashMap<ViewId, View>,
    next_view: ViewId,
    sidebar: SidebarCounts,
    /// Flows evicted by the limits so far.
    evicted: u64,
    pinned: u32,
    /// Body bytes seen on the wire so far (sent, received), for throughput.
    traffic: (u64, u64),
    /// An eviction that found nothing it may drop (everything old is
    /// pinned or in flight) isn't tried again below this many flows.
    evict_retry_at: u32,
    /// The header column set the flows' `extra` values belong to.
    extra_generation: u32,
}

/// Flows dropped by one eviction, for the engine to forget their details.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Eviction {
    pub evicted: Vec<FlowId>,
    /// Flows older than the newest evicted one that stay (pinned or in flight).
    pub kept: Vec<FlowId>,
    /// The newest evicted id.
    pub through: FlowId,
}

/// Evicting stops this far under the flow cap, so it runs once per
/// twentieth of the cap rather than on every new flow.
const EVICT_SLACK_DIVISOR: u32 = 20;

/// Views of a document's store have ids `store << VIEW_STORE_SHIFT | n`,
/// so a view id alone says which store it is over (the live store's are
/// below `1 << VIEW_STORE_SHIFT`).
pub const VIEW_STORE_SHIFT: u32 = 24;

impl Store {
    /// The store of an opened document, holding `flows` (ids `0..n`).
    pub fn document(store: pm_model::StoreId, flows: Vec<FlowSummary>) -> Store {
        let mut s = Store {
            next_view: store << VIEW_STORE_SHIFT,
            ..Default::default()
        };
        s.insert(flows);
        s
    }

    /// Every summary, in id order.
    pub fn flows(&self) -> &[FlowSummary] {
        &self.flows
    }

    pub fn total(&self) -> u32 {
        self.flows.len() as u32
    }

    /// The id of the oldest flow still in the store.
    pub fn first_id(&self) -> FlowId {
        self.flows.first().map_or(self.next, |f| f.id)
    }

    /// The id the next appended flow must carry.
    pub fn next_id(&self) -> FlowId {
        self.next
    }

    pub fn get(&self, id: FlowId) -> Option<&FlowSummary> {
        self.pos_of(id).map(|pos| &self.flows[pos])
    }

    /// The flows with ids in `low..high`, oldest first.
    pub fn flows_in(&self, low: FlowId, high: FlowId) -> &[FlowSummary] {
        let from = self.flows.partition_point(|f| f.id < low);
        let to = self.flows.partition_point(|f| f.id < high).max(from);
        &self.flows[from..to]
    }

    /// Flows evicted by the limits so far.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Pinned flows in the store.
    pub fn pinned(&self) -> u32 {
        self.pinned
    }

    /// Body bytes sent and received by every flow since the store began
    /// (clears and evictions don't take them back).
    pub fn traffic(&self) -> (u64, u64) {
        self.traffic
    }

    /// The arena position of flow `id`.
    fn pos_of(&self, id: FlowId) -> Option<usize> {
        let back = self.next.checked_sub(id)? as usize;
        let len = self.flows.len();
        // Every flow newer than the last eviction sits at its distance
        // from the end.
        if (1..=len).contains(&back) && self.flows[len - back].id == id {
            return Some(len - back);
        }
        self.flows.binary_search_by_key(&id, |f| f.id).ok()
    }

    /// Appends flows, assigning their ids, and updates every view.
    pub fn insert(&mut self, batch: impl IntoIterator<Item = FlowSummary>) -> Range<FlowId> {
        let first = self.next_id();
        let batch: Vec<FlowSummary> = batch
            .into_iter()
            .enumerate()
            .map(|(i, mut f)| {
                f.id = first + i as FlowId;
                f
            })
            .collect();
        self.apply([FlowEvent::Insert(batch)]);
        first..self.next_id()
    }

    /// Replaces existing flows by id and updates every view. Unknown ids are ignored.
    pub fn update(&mut self, batch: impl IntoIterator<Item = FlowSummary>) {
        self.apply([FlowEvent::Update(batch.into_iter().collect())]);
    }

    /// Applies one ingest batch. Views see flows appended in this batch once,
    /// in their final state, and changes to older flows as updates. Returns
    /// the flush acknowledgements to send once the write lock is released.
    pub(crate) fn apply(&mut self, events: impl IntoIterator<Item = FlowEvent>) -> Vec<SyncSender<()>> {
        let mut first_new = self.flows.len();
        let mut acks = Vec::new();
        for event in events {
            match event {
                FlowEvent::Started(flow) => self.push(flow),
                FlowEvent::Insert(batch) => batch.into_iter().for_each(|f| self.push(f)),
                FlowEvent::Update(batch) => {
                    for flow in batch {
                        let pinned = flow.pinned;
                        if let Some(old) = self.get(flow.id).map(|f| f.pinned)
                            && old != pinned
                        {
                            self.pinned = if pinned {
                                self.pinned + 1
                            } else {
                                self.pinned.saturating_sub(1)
                            };
                        }
                        self.modify_identity(flow.id, first_new, |f| *f = flow);
                    }
                }
                FlowEvent::ClientResolved { id, client } => {
                    self.modify_identity(id, first_new, |f| f.client = Some(client));
                }
                FlowEvent::Response {
                    id,
                    status,
                    content_kind,
                } => self.modify(id, first_new, |f| {
                    f.status = Some(status);
                    f.content_kind = content_kind;
                }),
                FlowEvent::Content {
                    id,
                    content_kind,
                    graphql_op,
                } => self.modify(id, first_new, |f| {
                    f.content_kind = content_kind;
                    f.graphql_op = graphql_op;
                }),
                FlowEvent::Progress {
                    id,
                    req_size,
                    resp_size,
                } => self.modify_sizes(id, first_new, |f| {
                    f.req_size = req_size;
                    f.resp_size = resp_size;
                }),
                FlowEvent::WsMsg { id, direction, len } => self.modify_sizes(id, first_new, |f| match direction {
                    pm_model::WsDirection::Sent => f.req_size += len,
                    pm_model::WsDirection::Received => f.resp_size += len,
                }),
                FlowEvent::Finished {
                    id,
                    state,
                    duration_ms,
                    req_size,
                    resp_size,
                } => self.modify_sizes(id, first_new, |f| {
                    f.state = state;
                    f.duration_ms = Some(duration_ms);
                    f.req_size = req_size;
                    f.resp_size = resp_size;
                }),
                FlowEvent::Grpc { id, status } => self.modify(id, first_new, |f| f.grpc_status = Some(status)),
                FlowEvent::Edited { id } => self.modify(id, first_new, |f| f.edited = true),
                FlowEvent::State { id, state } => self.modify(id, first_new, |f| {
                    // A late "continue" must not revive a finished flow.
                    if matches!(f.state, FlowState::Active | FlowState::Paused) {
                        f.state = state;
                    }
                }),
                FlowEvent::Pin { ids, pinned } => {
                    for id in ids {
                        if self.get(id).is_some_and(|f| f.pinned != pinned) {
                            self.modify(id, first_new, |f| f.pinned = pinned);
                            self.pinned = if pinned {
                                self.pinned + 1
                            } else {
                                self.pinned.saturating_sub(1)
                            };
                        }
                    }
                }
                FlowEvent::Clear => {
                    self.flows.clear();
                    self.pinned = 0;
                    self.evict_retry_at = 0;
                    self.sidebar.clear();
                    first_new = 0;
                    for view in self.views.values_mut() {
                        view.reset_deep();
                        view.rebuild(&[]);
                    }
                }
                FlowEvent::Label { ids, label } => {
                    for id in ids {
                        if self.get(id).is_some_and(|f| f.label != label) {
                            self.modify(id, first_new, |f| f.label = label);
                        }
                    }
                }
                FlowEvent::Comment { id, comment } => self.modify(id, first_new, |f| f.comment = comment),
                FlowEvent::Extra { generation, values } => {
                    if generation != self.extra_generation {
                        continue;
                    }
                    for (id, cells) in values {
                        let changed = self.get(id).is_some_and(|f| {
                            cells
                                .iter()
                                .any(|(i, v)| f.extra.get(*i as usize).cloned().flatten() != *v)
                        });
                        if changed {
                            self.modify(id, first_new, |f| {
                                for (i, value) in cells {
                                    let i = i as usize;
                                    if f.extra.len() <= i {
                                        f.extra.resize(i + 1, None);
                                    }
                                    f.extra[i] = value;
                                }
                            });
                        }
                    }
                }
                FlowEvent::Columns { generation } => {
                    self.extra_generation = generation;
                    for f in &mut self.flows {
                        f.extra = Vec::new();
                    }
                    for view in self.views.values_mut() {
                        view.rebuild(&self.flows);
                    }
                }
                FlowEvent::Flush(ack) => acks.push(ack),
            }
        }
        for view in self.views.values_mut() {
            if first_new < self.flows.len() {
                view.on_insert(&self.flows, first_new);
            }
            view.finish_batch(&self.flows);
        }
        acks
    }

    fn push(&mut self, flow: FlowSummary) {
        // Ids are allocated and queued under one lock, so they arrive in
        // order; a clear skips ahead.
        debug_assert!(flow.id >= self.next, "flows must arrive in id order");
        self.next = flow.id + 1;
        self.pinned += u32::from(flow.pinned);
        self.traffic.0 += flow.req_size;
        self.traffic.1 += flow.resp_size;
        self.sidebar.add(&flow);
        self.flows.push(flow);
    }

    /// Evicts the oldest flows: down to 95% of `max_flows` once the store
    /// holds more (0 is no cap), and every flow up to `through`. Pinned and
    /// in-flight flows stay. Views rebuild over the new positions.
    pub(crate) fn evict(&mut self, max_flows: u32, through: Option<FlowId>) -> Option<Eviction> {
        let len = self.flows.len() as u32;
        let mut budget = if max_flows > 0 && len > max_flows && len >= self.evict_retry_at {
            len - (max_flows - max_flows / EVICT_SLACK_DIVISOR)
        } else {
            0
        };
        if budget == 0 && through.is_none_or(|t| t < self.first_id()) {
            return None;
        }
        let by_count = budget > 0;
        let mut evicted = Vec::new();
        let sidebar = &mut self.sidebar;
        self.flows.retain(|f| {
            let kept = f.pinned || matches!(f.state, FlowState::Active | FlowState::Paused);
            let due = budget > 0 || through.is_some_and(|t| f.id <= t);
            if kept || !due {
                return true;
            }
            budget = budget.saturating_sub(1);
            sidebar.remove(f);
            evicted.push(f.id);
            false
        });
        // The disk cap asks to go through an id even when every flow up to
        // it is kept (a long WebSocket): segments only they don't use can
        // still go.
        if by_count {
            // Everything old is pinned or in flight: don't rescan every batch.
            self.evict_retry_at = if evicted.is_empty() {
                len + max_flows / EVICT_SLACK_DIVISOR
            } else {
                0
            };
        }
        let through = match (evicted.last(), through) {
            (Some(&last), Some(t)) => last.max(t),
            (Some(&last), None) => last,
            (None, Some(t)) => t,
            (None, None) => return None,
        };
        let kept = self
            .flows
            .iter()
            .map(|f| f.id)
            .take_while(|&id| id <= through)
            .collect();
        self.evicted += evicted.len() as u64;
        if !evicted.is_empty() {
            for view in self.views.values_mut() {
                view.forget_deep(&evicted);
                view.rebuild(&self.flows);
            }
        }
        Some(Eviction { evicted, kept, through })
    }

    /// Deletes flows (M17), pinned ones too. Flows still in flight stay:
    /// their recorders would write them back. Views rebuild over the new
    /// positions. Returns the ids removed, sorted.
    pub(crate) fn remove(&mut self, ids: &std::collections::HashSet<FlowId>) -> Vec<FlowId> {
        let mut removed = Vec::new();
        let sidebar = &mut self.sidebar;
        let mut unpinned = 0;
        self.flows.retain(|f| {
            if !ids.contains(&f.id) || matches!(f.state, FlowState::Active | FlowState::Paused) {
                return true;
            }
            sidebar.remove(f);
            unpinned += u32::from(f.pinned);
            removed.push(f.id);
            false
        });
        self.pinned -= unpinned;
        if !removed.is_empty() {
            for view in self.views.values_mut() {
                view.forget_deep(&removed);
                view.rebuild(&self.flows);
            }
        }
        removed
    }

    /// True if the store has a flow with an id in `low..=high`.
    pub(crate) fn has_flow_in(&self, low: FlowId, high: FlowId) -> bool {
        high >= low && !self.flows_in(low, high.saturating_add(1)).is_empty()
    }

    /// The header column set the flows' values belong to.
    pub(crate) fn extra_generation(&self) -> u32 {
        self.extra_generation
    }

    /// Changes a flow in place. Flows appended earlier in this batch are not
    /// reported to views yet, so only older ones produce view updates.
    fn modify(&mut self, id: FlowId, first_new: usize, change: impl FnOnce(&mut FlowSummary)) {
        let Some(pos) = self.pos_of(id) else {
            return;
        };
        let slot = &mut self.flows[pos];
        if pos >= first_new {
            change(slot);
            return;
        }
        let old = slot.clone();
        change(slot);
        let new = &self.flows[pos];
        for view in self.views.values_mut() {
            view.on_update(pos as u32, &old, new);
        }
    }

    /// Like [`Store::modify`], for changes to a flow's sizes: the traffic
    /// counters take the growth.
    fn modify_sizes(&mut self, id: FlowId, first_new: usize, change: impl FnOnce(&mut FlowSummary)) {
        let mut grew = (0, 0);
        self.modify(id, first_new, |f| {
            let before = (f.req_size, f.resp_size);
            change(f);
            grew = (
                f.req_size.saturating_sub(before.0),
                f.resp_size.saturating_sub(before.1),
            );
        });
        self.traffic.0 += grew.0;
        self.traffic.1 += grew.1;
    }

    /// Like [`Store::modify`], for changes that may move the flow to another
    /// host or client: the sidebar counts follow.
    fn modify_identity(&mut self, id: FlowId, first_new: usize, change: impl FnOnce(&mut FlowSummary)) {
        let Some(old) = self.get(id).map(|f| (f.host.clone(), f.client.clone())) else {
            return;
        };
        self.modify(id, first_new, change);
        if let Some(pos) = self.pos_of(id) {
            self.sidebar.change(&old.0, old.1.as_ref(), &self.flows[pos]);
        }
    }

    pub fn sidebar(&self) -> &SidebarCounts {
        &self.sidebar
    }

    pub fn sidebar_mut(&mut self) -> &mut SidebarCounts {
        &mut self.sidebar
    }

    pub fn create_view(&mut self, spec: FilterSpec, sort: SortSpec) -> ViewInfo {
        self.next_view += 1;
        let id = self.next_view;
        let (view, elapsed_us) = timed(|| View::new(id, spec, sort, &self.flows));
        self.views.insert(id, view);
        self.info(id, elapsed_us)
    }

    pub fn set_filter(&mut self, id: ViewId, spec: FilterSpec) -> Result<ViewInfo, Error> {
        let view = self.views.get_mut(&id).ok_or(Error::UnknownView(id))?;
        let ((), elapsed_us) = timed(|| view.set_filter(spec, &self.flows));
        Ok(self.info(id, elapsed_us))
    }

    pub fn set_sort(&mut self, id: ViewId, sort: SortSpec) -> Result<ViewInfo, Error> {
        let view = self.views.get_mut(&id).ok_or(Error::UnknownView(id))?;
        let ((), elapsed_us) = timed(|| view.set_sort(sort, &self.flows));
        Ok(self.info(id, elapsed_us))
    }

    pub fn drop_view(&mut self, id: ViewId) -> bool {
        self.views.remove(&id).is_some()
    }

    /// Hands the deep search job of view `id` its next candidates.
    pub(crate) fn take_deep_batch(&mut self, id: ViewId, cancel: &Arc<AtomicBool>, max: usize) -> Option<Vec<FlowId>> {
        self.views.get_mut(&id)?.take_deep_batch(cancel, max)
    }

    /// Records what the deep search job of view `id` found.
    pub(crate) fn deep_results(&mut self, id: ViewId, cancel: &Arc<AtomicBool>, checked: u32, hits: &[FlowId]) {
        let hits: Vec<(FlowId, Option<view::Pos>)> = hits
            .iter()
            .map(|&id| (id, self.pos_of(id).map(|p| p as view::Pos)))
            .collect();
        if let Some(view) = self.views.get_mut(&id) {
            view.deep_results(&self.flows, cancel, checked, &hits);
        }
    }

    pub fn view(&self, id: ViewId) -> Option<&View> {
        self.views.get(&id)
    }

    pub fn view_mut(&mut self, id: ViewId) -> Option<&mut View> {
        self.views.get_mut(&id)
    }

    pub fn rows(&self, id: ViewId, start: u32, count: u32) -> Result<RowsPage, Error> {
        let view = self.views.get(&id).ok_or(Error::UnknownView(id))?;
        Ok(RowsPage {
            view: id,
            start,
            len: view.len(),
            rows: view.rows(&self.flows, start, count),
        })
    }

    /// The summaries a view shows, in its order.
    pub fn view_flows(&self, id: ViewId) -> Result<Vec<FlowSummary>, Error> {
        let view = self.views.get(&id).ok_or(Error::UnknownView(id))?;
        Ok(view
            .positions()
            .iter()
            .map(|&pos| self.flows[pos as usize].clone())
            .collect())
    }

    pub fn take_delta(&mut self, id: ViewId) -> Option<ViewDelta> {
        let total = self.total();
        self.views.get_mut(&id)?.take_delta(total)
    }

    fn info(&self, id: ViewId, elapsed_us: u32) -> ViewInfo {
        let view = &self.views[&id];
        ViewInfo {
            view: id,
            total: self.total(),
            len: view.len(),
            elapsed_us,
            deep: view.deep_progress(),
        }
    }
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, u32) {
    let start = std::time::Instant::now();
    let out = f();
    (out, start.elapsed().as_micros().min(u32::MAX as u128) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Generator;
    use pm_model::{FlowState, ProtocolFilter, SortDir, SortKey};

    fn store_with(n: usize) -> Store {
        let mut store = Store::default();
        let mut generator = Generator::new(7);
        store.insert((0..n).map(|_| generator.completed_flow()));
        store
    }

    #[test]
    fn appends_flow_into_arrival_order_views_as_appended_rows() {
        let mut store = store_with(100);
        let view = store.create_view(FilterSpec::default(), SortSpec::default()).view;
        let first = store.take_delta(view).unwrap();
        assert!(first.structural);
        assert_eq!((first.len, first.total), (100, 100));
        assert_eq!(store.take_delta(view), None);

        let mut generator = Generator::new(8);
        store.insert((0..5).map(|_| generator.completed_flow()));
        let delta = store.take_delta(view).unwrap();
        assert!(!delta.structural);
        assert_eq!((delta.len, delta.appended, delta.total), (105, 5, 105));
    }

    #[test]
    fn refilter_matches_a_linear_scan() {
        let mut store = store_with(5_000);
        let spec = FilterSpec {
            protocol: ProtocolFilter::Https,
            text: "api".into(),
            ..Default::default()
        };
        let expected: Vec<FlowId> = {
            let filter = CompiledFilter::compile(&spec);
            store.flows.iter().filter(|f| filter.matches(f)).map(|f| f.id).collect()
        };
        assert!(!expected.is_empty() && expected.len() < 5_000);
        let view = store.create_view(FilterSpec::default(), SortSpec::default()).view;
        let info = store.set_filter(view, spec).unwrap();
        assert_eq!(info.len as usize, expected.len());
        let page = store.rows(view, 0, u32::MAX).unwrap();
        let ids: Vec<FlowId> = page.rows.iter().map(|r| r.0).collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn updates_report_changed_ids_or_restructure() {
        let mut store = store_with(10);
        let all = store.create_view(FilterSpec::default(), SortSpec::default()).view;
        let https = store
            .create_view(
                FilterSpec {
                    protocol: ProtocolFilter::Https,
                    ..Default::default()
                },
                SortSpec::default(),
            )
            .view;
        store.take_delta(all);
        store.take_delta(https);

        // A summary change that keeps membership is reported by id.
        let mut f = store.get(3).unwrap().clone();
        f.state = FlowState::Failed;
        store.update([f.clone()]);
        let delta = store.take_delta(all).unwrap();
        assert_eq!(delta.changed, vec![3]);
        assert!(!delta.structural);

        // Flipping the scheme moves the flow in or out of the HTTPS view.
        let was_https = f.scheme.is_tls();
        f.scheme = if was_https {
            pm_model::Scheme::Http
        } else {
            pm_model::Scheme::Https
        };
        let before = store.view(https).unwrap().len();
        store.update([f]);
        let delta = store.take_delta(https).unwrap();
        assert!(delta.structural);
        assert_eq!(delta.len, if was_https { before - 1 } else { before + 1 });
    }

    #[test]
    fn sorted_views_stay_sorted_after_inserts() {
        let mut store = store_with(1_000);
        let sort = SortSpec {
            key: SortKey::RespSize,
            dir: SortDir::Desc,
            ..Default::default()
        };
        let view = store.create_view(FilterSpec::default(), sort).view;
        let mut generator = Generator::new(9);
        store.insert((0..100).map(|_| generator.completed_flow()));
        let page = store.rows(view, 0, u32::MAX).unwrap();
        assert_eq!(page.rows.len(), 1_100);
        assert!(page.rows.windows(2).all(|w| w[0].14 >= w[1].14));
        assert!(store.take_delta(view).unwrap().structural);
    }

    #[test]
    fn clear_drops_flows_but_never_reuses_ids() {
        let mut store = store_with(10);
        let view = store.create_view(FilterSpec::default(), SortSpec::default()).view;
        store.take_delta(view);
        store.apply([FlowEvent::Clear]);
        let delta = store.take_delta(view).unwrap();
        assert!(delta.structural);
        assert_eq!((delta.len, delta.total), (0, 0));

        // A late event for a cleared flow is ignored.
        store.apply([FlowEvent::Finished {
            id: 3,
            state: FlowState::Completed,
            duration_ms: 1,
            req_size: 0,
            resp_size: 0,
        }]);
        assert_eq!(store.get(3), None);

        let mut generator = Generator::new(1);
        let ids = store.insert((0..2).map(|_| generator.completed_flow()));
        assert_eq!(ids, 10..12);
        assert_eq!(store.get(11).map(|f| f.id), Some(11));
        let page = store.rows(view, 0, 10).unwrap();
        assert_eq!(page.rows.iter().map(|r| r.0).collect::<Vec<_>>(), vec![10, 11]);
    }

    #[test]
    fn events_for_flows_started_in_the_same_batch_are_folded_in() {
        let mut store = Store::default();
        let view = store
            .create_view(
                FilterSpec {
                    content: vec![pm_model::ContentGroup::Json],
                    ..Default::default()
                },
                SortSpec::default(),
            )
            .view;
        store.take_delta(view);
        let mut flow = Generator::new(2).active_flow();
        flow.id = 0;
        flow.content_kind = pm_model::ContentKind::None;
        store.apply([
            FlowEvent::Started(flow),
            FlowEvent::Response {
                id: 0,
                status: 200,
                content_kind: pm_model::ContentKind::Json,
            },
        ]);
        // The flow only matches the JSON chip after its response arrived.
        let delta = store.take_delta(view).unwrap();
        assert_eq!((delta.len, delta.appended), (1, 1));
        assert_eq!(store.get(0).unwrap().status, Some(200));
    }

    #[test]
    fn eviction_keeps_pinned_and_active_flows_and_reports_disk_only_passes() {
        let mut store = store_with(10);
        let mut pinned = store.get(1).unwrap().clone();
        pinned.pinned = true;
        let mut active = store.get(2).unwrap().clone();
        active.state = FlowState::Active;
        store.update([pinned, active]);
        assert_eq!(store.pinned(), 1);

        // The disk cap asks for everything through id 2; only 0 can go.
        let eviction = store.evict(0, Some(2)).unwrap();
        assert_eq!(
            (eviction.evicted, eviction.kept, eviction.through),
            (vec![0], vec![1, 2], 2)
        );
        assert_eq!(store.first_id(), 1);
        assert_eq!(store.get(5).map(|f| f.id), Some(5));
        assert_eq!(
            store.flows_in(2, 5).iter().map(|f| f.id).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );

        // Nothing left to evict through 2: segments may still be freed.
        let again = store.evict(0, Some(2)).unwrap();
        assert!(again.evicted.is_empty());
        assert_eq!(again.kept, vec![1, 2]);
        assert_eq!(store.evict(0, None), None);
    }

    #[test]
    fn unknown_views_are_errors() {
        let store = Store::default();
        assert!(matches!(store.rows(42, 0, 10), Err(Error::UnknownView(42))));
    }
}
