//! The live flow store: the summary arena plus the views over it.
//!
//! Bodies, `FlowDetail` segments and eviction arrive in M1; for now a flow's
//! id is its index in the arena.

mod filter;
mod view;

use std::collections::HashMap;
use std::ops::Range;

use pm_model::{FilterSpec, FlowId, FlowSummary, RowsPage, SortSpec, ViewDelta, ViewId, ViewInfo};

pub use filter::CompiledFilter;
pub use view::View;

use crate::Error;

#[derive(Default)]
pub struct Store {
    flows: Vec<FlowSummary>,
    views: HashMap<ViewId, View>,
    next_view: ViewId,
}

impl Store {
    pub fn total(&self) -> u32 {
        self.flows.len() as u32
    }

    pub fn get(&self, id: FlowId) -> Option<&FlowSummary> {
        self.flows.get(id as usize)
    }

    /// Appends flows, assigning their ids, and updates every view.
    pub fn insert(&mut self, batch: impl IntoIterator<Item = FlowSummary>) -> Range<FlowId> {
        let first = self.flows.len();
        for mut flow in batch {
            flow.id = self.flows.len() as FlowId;
            self.flows.push(flow);
        }
        for view in self.views.values_mut() {
            view.on_insert(&self.flows, first);
            view.finish_batch(&self.flows);
        }
        first as FlowId..self.flows.len() as FlowId
    }

    /// Replaces existing flows by id and updates every view. Unknown ids are ignored.
    pub fn update(&mut self, batch: impl IntoIterator<Item = FlowSummary>) {
        for flow in batch {
            let Some(slot) = self.flows.get_mut(flow.id as usize) else {
                continue;
            };
            let old = std::mem::replace(slot, flow);
            let new = &self.flows[old.id as usize];
            for view in self.views.values_mut() {
                view.on_update(&old, new);
            }
        }
        for view in self.views.values_mut() {
            view.finish_batch(&self.flows);
        }
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
    fn unknown_views_are_errors() {
        let store = Store::default();
        assert!(matches!(store.rows(42, 0, 10), Err(Error::UnknownView(42))));
    }
}
