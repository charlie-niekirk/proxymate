//! The live flow store: the summary arena plus the views over it, and the
//! body and detail storage beside it.
//!
//! Flow ids are never reused. `flows[0]` holds id `base`; a clear empties the
//! arena and moves `base` up to the next id.

mod blob;
mod detail;
mod filter;
mod segments;
mod sidebar;
mod view;

use std::collections::HashMap;
use std::ops::Range;
use std::sync::mpsc::SyncSender;

use pm_model::{FilterSpec, FlowId, FlowSummary, RowsPage, SortSpec, ViewDelta, ViewId, ViewInfo};

pub(crate) use blob::Capture;
pub(crate) use detail::{DetailStore, StoredDetail};
pub use filter::CompiledFilter;
pub(crate) use segments::SegmentStore;
pub use sidebar::{SidebarCounts, in_domain, registrable_domain};
pub use view::View;

use crate::Error;
use crate::ingest::FlowEvent;

#[derive(Default)]
pub struct Store {
    base: FlowId,
    flows: Vec<FlowSummary>,
    views: HashMap<ViewId, View>,
    next_view: ViewId,
    sidebar: SidebarCounts,
}

impl Store {
    pub fn total(&self) -> u32 {
        self.flows.len() as u32
    }

    /// The id the next appended flow must carry.
    pub fn next_id(&self) -> FlowId {
        self.base + self.flows.len() as FlowId
    }

    pub fn get(&self, id: FlowId) -> Option<&FlowSummary> {
        self.flows.get(id.checked_sub(self.base)? as usize)
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
                FlowEvent::Progress {
                    id,
                    req_size,
                    resp_size,
                } => self.modify(id, first_new, |f| {
                    f.req_size = req_size;
                    f.resp_size = resp_size;
                }),
                FlowEvent::Finished {
                    id,
                    state,
                    duration_ms,
                    req_size,
                    resp_size,
                } => self.modify(id, first_new, |f| {
                    f.state = state;
                    f.duration_ms = Some(duration_ms);
                    f.req_size = req_size;
                    f.resp_size = resp_size;
                }),
                FlowEvent::Clear => {
                    self.base = self.next_id();
                    self.flows.clear();
                    self.sidebar.clear();
                    first_new = 0;
                    for view in self.views.values_mut() {
                        view.rebuild(&[]);
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
        // Ids are allocated and queued under one lock, so they arrive in order.
        debug_assert_eq!(flow.id, self.next_id(), "flows must arrive in id order");
        self.sidebar.add(&flow);
        self.flows.push(flow);
    }

    /// Changes a flow in place. Flows appended earlier in this batch are not
    /// reported to views yet, so only older ones produce view updates.
    fn modify(&mut self, id: FlowId, first_new: usize, change: impl FnOnce(&mut FlowSummary)) {
        let Some(pos) = id.checked_sub(self.base).map(|p| p as usize) else {
            return;
        };
        let Some(slot) = self.flows.get_mut(pos) else {
            return;
        };
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

    /// Like [`Store::modify`], for changes that may move the flow to another
    /// host or client: the sidebar counts follow.
    fn modify_identity(&mut self, id: FlowId, first_new: usize, change: impl FnOnce(&mut FlowSummary)) {
        let Some(old) = self.get(id).map(|f| (f.host.clone(), f.client.clone())) else {
            return;
        };
        self.modify(id, first_new, change);
        if let Some(pos) = id.checked_sub(self.base).map(|p| p as usize)
            && let Some(new) = self.flows.get(pos)
        {
            self.sidebar.change(&old.0, old.1.as_ref(), new);
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

    pub fn take_delta(&mut self, id: ViewId) -> Option<ViewDelta> {
        let total = self.total();
        self.views.get_mut(&id)?.take_delta(total)
    }

    fn info(&self, id: ViewId, elapsed_us: u32) -> ViewInfo {
        ViewInfo {
            view: id,
            total: self.total(),
            len: self.views[&id].len(),
            elapsed_us,
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
    fn unknown_views_are_errors() {
        let store = Store::default();
        assert!(matches!(store.rows(42, 0, 10), Err(Error::UnknownView(42))));
    }
}
