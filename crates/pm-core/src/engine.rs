//! The `Engine` facade. Tauri commands, MCP tools and the CLI call only this,
//! so the UI and agents always see the same behaviour.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use pm_model::{FilterSpec, FlowId, FlowSummary, RowsPage, SortSpec, ViewDelta, ViewId, ViewInfo};

use crate::Error;
use crate::store::Store;

/// Deltas are published at 20 Hz.
pub const PUBLISH_INTERVAL: Duration = Duration::from_millis(50);

/// Receives a view's deltas. Returning `false` unsubscribes (e.g. the
/// webview's channel was closed).
pub type DeltaSink = Box<dyn FnMut(ViewDelta) -> bool + Send>;

pub struct Engine {
    store: RwLock<Store>,
    sinks: Mutex<HashMap<ViewId, DeltaSink>>,
}

impl Engine {
    /// Creates an engine and starts its 20 Hz delta publisher thread, which
    /// stops when the last `Arc<Engine>` is dropped.
    pub fn new() -> Arc<Engine> {
        let engine = Arc::new(Engine {
            store: RwLock::new(Store::default()),
            sinks: Mutex::new(HashMap::new()),
        });
        let weak = Arc::downgrade(&engine);
        std::thread::Builder::new()
            .name("pm-publisher".into())
            .spawn(move || publisher_loop(weak))
            .expect("spawn publisher thread");
        engine
    }

    /// Number of flows in the live store.
    pub fn total(&self) -> u32 {
        self.store.read().total()
    }

    /// Appends new flows under a single write lock. Ids are assigned here;
    /// whatever `id` the summaries carry is ignored.
    pub fn insert_flows(&self, batch: Vec<FlowSummary>) -> Range<FlowId> {
        self.store.write().insert(batch)
    }

    /// Replaces existing flows (matched by id) under a single write lock.
    pub fn update_flows(&self, batch: Vec<FlowSummary>) {
        self.store.write().update(batch);
    }

    pub fn get_flow(&self, id: FlowId) -> Option<FlowSummary> {
        self.store.read().get(id).cloned()
    }

    pub fn create_view(&self, filter: FilterSpec, sort: SortSpec) -> ViewInfo {
        self.store.write().create_view(filter, sort)
    }

    /// Refilters synchronously and returns the new length, so callers can
    /// fetch rows straight away instead of waiting for the next delta.
    pub fn set_filter(&self, view: ViewId, filter: FilterSpec) -> Result<ViewInfo, Error> {
        self.store.write().set_filter(view, filter)
    }

    pub fn set_sort(&self, view: ViewId, sort: SortSpec) -> Result<ViewInfo, Error> {
        self.store.write().set_sort(view, sort)
    }

    pub fn drop_view(&self, view: ViewId) {
        self.sinks.lock().remove(&view);
        self.store.write().drop_view(view);
    }

    pub fn get_rows(&self, view: ViewId, start: u32, count: u32) -> Result<RowsPage, Error> {
        self.store.read().rows(view, start, count)
    }

    /// Routes a view's deltas to `sink`, replacing any previous subscriber.
    /// The first delta is structural and goes out on the next tick.
    pub fn subscribe(&self, view: ViewId, sink: DeltaSink) -> Result<(), Error> {
        self.store
            .write()
            .view_mut(view)
            .ok_or(Error::UnknownView(view))?
            .mark_structural();
        self.sinks.lock().insert(view, sink);
        Ok(())
    }

    /// Publishes one tick: drains each subscribed view's pending change and
    /// hands it to its sink. Called by the publisher thread; public for tests
    /// and headless tools.
    pub fn publish_tick(&self) {
        let mut sinks = self.sinks.lock();
        if sinks.is_empty() {
            return;
        }
        let deltas: Vec<ViewDelta> = {
            let mut store = self.store.write();
            sinks.keys().filter_map(|&view| store.take_delta(view)).collect()
        };
        for delta in deltas {
            let view = delta.view;
            if let Some(sink) = sinks.get_mut(&view)
                && !sink(delta)
            {
                sinks.remove(&view);
            }
        }
    }
}

fn publisher_loop(engine: Weak<Engine>) {
    loop {
        std::thread::sleep(PUBLISH_INTERVAL);
        let Some(engine) = engine.upgrade() else {
            return;
        };
        engine.publish_tick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Generator;
    use std::sync::mpsc;

    #[test]
    fn subscribers_receive_structural_then_incremental_deltas() {
        let engine = Engine::new();
        let mut generator = Generator::new(1);
        engine.insert_flows((0..10).map(|_| generator.completed_flow()).collect());
        let view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;

        let (tx, rx) = mpsc::channel();
        engine.subscribe(view, Box::new(move |d| tx.send(d).is_ok())).unwrap();
        engine.publish_tick();
        let first = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(first.structural);
        assert_eq!(first.len, 10);

        engine.insert_flows((0..3).map(|_| generator.completed_flow()).collect());
        engine.publish_tick();
        // The background publisher may have raced us to this tick.
        let delta = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!((delta.appended, delta.len), (3, 13));
    }

    #[test]
    fn closed_sinks_are_dropped() {
        let engine = Engine::new();
        let view = engine.create_view(FilterSpec::default(), SortSpec::default()).view;
        engine.subscribe(view, Box::new(|_| false)).unwrap();
        engine.publish_tick();
        assert!(engine.sinks.lock().is_empty());
    }
}
