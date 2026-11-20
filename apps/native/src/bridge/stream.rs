//! Engine push streams into gpui entities. A sink (called on the engine's
//! publisher thread) only enqueues; a foreground task drains the channel,
//! keeps the newest value and notifies the entity's observers once per
//! drain, so a burst of pushes costs one render.

use gpui_kit::{App, AppContext as _, Entity};

/// The engine's sink shape: `false` unsubscribes.
pub type Sink<T> = Box<dyn FnMut(T) -> bool + Send>;

/// The latest value of a single-slot stream (`None` until the first push).
pub struct Latest<T> {
    pub value: Option<T>,
}

/// Subscribes through `subscribe` and returns the entity the stream feeds.
/// Once every handle to the entity is dropped, the next push ends the task
/// and the one after it unsubscribes the sink.
pub fn latest<T: Send + 'static>(cx: &mut App, subscribe: impl FnOnce(Sink<T>)) -> Entity<Latest<T>> {
    let entity = cx.new(|_| Latest { value: None });
    let (tx, rx) = async_channel::unbounded::<T>();
    subscribe(Box::new(move |value| tx.try_send(value).is_ok()));
    let weak = entity.downgrade();
    cx.spawn(async move |cx| {
        while let Ok(mut value) = rx.recv().await {
            while let Ok(newer) = rx.try_recv() {
                value = newer;
            }
            let updated = weak.update(cx, |latest, cx| {
                latest.value = Some(value);
                cx.notify();
            });
            if updated.is_err() {
                break;
            }
        }
    })
    .detach();
    entity
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gpui_kit::TestAppContext;

    use super::{Sink, latest};

    #[gpui_kit::test]
    fn pushes_reach_the_entity_and_bursts_coalesce(cx: &mut TestAppContext) {
        let slot: Arc<Mutex<Option<Sink<u32>>>> = Arc::default();
        let held = slot.clone();
        let entity = cx.update(|cx| latest(cx, move |sink| *held.lock().unwrap() = Some(sink)));
        let renders = Arc::new(Mutex::new(0));
        let counter = renders.clone();
        let _sub = cx.update(|cx| cx.observe(&entity, move |_, _| *counter.lock().unwrap() += 1));
        {
            let mut sink = slot.lock().unwrap();
            let sink = sink.as_mut().unwrap();
            for n in 1..=5 {
                assert!(sink(n));
            }
        }
        cx.run_until_parked();
        assert_eq!(cx.read(|cx| entity.read(cx).value), Some(5));
        assert_eq!(*renders.lock().unwrap(), 1, "five pushes, one notify");
    }

    #[gpui_kit::test]
    fn a_dropped_entity_unsubscribes_the_sink(cx: &mut TestAppContext) {
        let slot: Arc<Mutex<Option<Sink<u32>>>> = Arc::default();
        let held = slot.clone();
        let entity = cx.update(|cx| latest(cx, move |sink| *held.lock().unwrap() = Some(sink)));
        drop(entity);
        cx.run_until_parked();
        let push = |n| (slot.lock().unwrap().as_mut().unwrap())(n);
        // This push finds the entity gone and ends the task, closing the channel.
        push(1);
        cx.run_until_parked();
        assert!(!push(2));
    }
}
