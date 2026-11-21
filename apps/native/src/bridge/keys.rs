//! Every [`Resource`] key, one shape of value per key (the port of
//! `lib/query.ts`'s key factories), and the caches that hold them. A phase
//! that reads new engine state adds its key and cache here.
//!
//! The caches are app-wide ([`caches`]): their fetchers call the bridge's
//! engine, and without one (tests, offscreen renders) they hold only what
//! is [`Resource::set`].

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Entity, Global};
use pm_model::{BodyPart, FlowDetail, FlowId, FlowSummary, OperationMatch, StoreId, ViewDelta};

use super::Bridge;
use super::resource::{Fetcher, Resource};

/// One flow of one store (the live capture or a document).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FlowKey {
    pub store: StoreId,
    pub id: FlowId,
}

impl FlowKey {
    pub fn new(store: StoreId, id: FlowId) -> FlowKey {
        FlowKey { store, id }
    }
}

/// One body of one flow, as captured (not decoded).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BodyKey {
    pub flow: FlowKey,
    pub part: BodyPart,
}

/// One body as the Body tab reads it: decoded, and only the first `max`
/// bytes until "Load all".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DecodedKey {
    pub flow: FlowKey,
    pub part: BodyPart,
    pub max: Option<u64>,
}

/// A decoded body (`useBody`): its bytes, the encoding undone, or why it
/// couldn't be (the raw bytes are shown then).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DecodedBody {
    pub bytes: Arc<Vec<u8>>,
    pub decoded_from: Option<String>,
    pub decode_error: Option<String>,
    /// Only the first `max` bytes were read and there are more.
    pub cut: bool,
}

/// What the inspector shows of a flow: its summary and, unless it is
/// synthetic, its detail.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspected {
    pub flow: FlowSummary,
    pub detail: Option<FlowDetail>,
}

/// `flowKey`: the inspected flow, `None` once the store has no such flow
/// (cleared, deleted or evicted).
pub type InspectedCache = Resource<FlowKey, Option<Arc<Inspected>>>;
/// `operationKey`: the imported OpenAPI operation a flow matches (M20).
pub type OperationCache = Resource<FlowKey, Option<OperationMatch>>;
/// `bodyKey`: a body's captured bytes (the Raw tab). Not refetched when a
/// live flow changes: the Raw tab reads a body only once it is finished.
pub type BodyCache = Resource<BodyKey, Arc<Vec<u8>>>;
/// `bodyKey` with `decoded`: the Body tab's bodies (G4). Read once a body
/// is finished, so not refetched as a live flow changes.
pub type DecodedCache = Resource<DecodedKey, Arc<DecodedBody>>;

#[derive(Clone)]
pub struct Caches {
    pub inspected: Entity<InspectedCache>,
    pub operation: Entity<OperationCache>,
    pub body: Entity<BodyCache>,
    pub decoded: Entity<DecodedCache>,
}

impl Global for Caches {}

/// The app's caches, made on first use.
pub fn caches(cx: &mut App) -> Caches {
    if let Some(caches) = cx.try_global::<Caches>() {
        return caches.clone();
    }
    let engine = cx.try_global::<Bridge>().map(|b| b.engine.clone());
    let inspected: Option<Fetcher<FlowKey, Option<Arc<Inspected>>>> = engine.clone().map(|engine| {
        Arc::new(move |k: &FlowKey| {
            let Some(flow) = engine.get_flow_in(k.store, k.id) else {
                return Ok(None);
            };
            let detail = engine.flow_detail_in(k.store, k.id).map_err(|e| e.to_string())?;
            Ok(Some(Arc::new(Inspected { flow, detail })))
        }) as Fetcher<_, _>
    });
    let operation: Option<Fetcher<FlowKey, Option<OperationMatch>>> = engine.clone().map(|engine| {
        Arc::new(move |k: &FlowKey| engine.flow_operation_in(k.store, k.id).map_err(|e| e.to_string())) as Fetcher<_, _>
    });
    let decoded: Option<Fetcher<DecodedKey, Arc<DecodedBody>>> = engine.clone().map(|engine| {
        Arc::new(move |k: &DecodedKey| {
            let max = k.max.map(|m| usize::try_from(m).unwrap_or(usize::MAX));
            let (store, id) = (k.flow.store, k.flow.id);
            match engine.body_head_in(store, id, k.part, true, max) {
                Ok(b) => Ok(Arc::new(DecodedBody {
                    bytes: Arc::new(b.bytes),
                    decoded_from: b.decoded_from,
                    decode_error: None,
                    cut: b.cut,
                })),
                // It didn't decode: the raw bytes, with why.
                Err(err) => {
                    let raw = engine
                        .body_head_in(store, id, k.part, false, max)
                        .map_err(|e| e.to_string())?;
                    Ok(Arc::new(DecodedBody {
                        bytes: Arc::new(raw.bytes),
                        decoded_from: None,
                        decode_error: Some(err.to_string()),
                        cut: raw.cut,
                    }))
                }
            }
        }) as Fetcher<_, _>
    });
    let body: Option<Fetcher<BodyKey, Arc<Vec<u8>>>> = engine.map(|engine| {
        Arc::new(move |k: &BodyKey| {
            engine
                .body_head_in(k.flow.store, k.flow.id, k.part, false, None)
                .map(|b| Arc::new(b.bytes))
                .map_err(|e| e.to_string())
        }) as Fetcher<_, _>
    });
    let caches = Caches {
        inspected: cx.new(|_| Resource::new(inspected)),
        operation: cx.new(|_| Resource::new(operation)),
        body: cx.new(|_| Resource::new(body)),
        decoded: cx.new(|_| Resource::new(decoded)),
    };
    cx.set_global(caches.clone());
    caches
}

/// A view's delta may have changed flows of `store`: the inspected flows it
/// names go stale; a structural one (a clear, an eviction) makes every
/// cached flow of the store stale, bodies too, since ids can come back.
/// A flow asked for before it reached the store is asked again as rows arrive.
pub fn flows_changed(store: StoreId, delta: &ViewDelta, cx: &mut App) {
    let caches = caches(cx);
    let everything = delta.structural || delta.total == 0;
    caches.inspected.update(cx, |c, cx| {
        c.invalidate(
            |k, v| {
                k.store == store
                    && (everything
                        || delta.changed.contains(&k.id)
                        || (delta.appended > 0 && v.is_some_and(Option::is_none)))
            },
            cx,
        );
    });
    if everything {
        caches
            .body
            .update(cx, |c, cx| c.invalidate(|k, _| k.flow.store == store, cx));
        caches
            .decoded
            .update(cx, |c, cx| c.invalidate(|k, _| k.flow.store == store, cx));
        caches
            .operation
            .update(cx, |c, cx| c.invalidate(|k, _| k.store == store, cx));
    }
}

/// Flows of `store` changed in a way no delta reports (pins and labels of
/// a document, which aren't in a view's `changed`).
pub fn store_changed(store: StoreId, cx: &mut App) {
    let caches = caches(cx);
    caches
        .inspected
        .update(cx, |c, cx| c.invalidate(|k, _| k.store == store, cx));
}
