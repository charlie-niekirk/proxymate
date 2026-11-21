//! Every [`Resource`] key, one shape of value per key (the port of
//! `lib/query.ts`'s key factories), and the caches that hold them. A phase
//! that reads new engine state adds its key and cache here.
//!
//! The caches are app-wide ([`caches`]): their fetchers call the bridge's
//! engine, and without one (tests, offscreen renders) they hold only what
//! is [`Resource::set`].

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Entity, Global};
use pm_model::{
    BodyPart, FlowDetail, FlowId, FlowSummary, OperationMatch, SseEvent, SsePage, StoreId, ViewDelta, WsDirection,
    WsMessage, WsPage, WsQuery,
};
use pm_view::stream::LIST_PAGE;

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

/// A WebSocket flow's messages under a filter (`wsHeadKey`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WsListKey {
    pub flow: FlowKey,
    pub direction: Option<WsDirection>,
    pub text: String,
}

impl WsListKey {
    pub fn query(&self) -> WsQuery {
        WsQuery {
            direction: self.direction,
            text: self.text.clone(),
        }
    }
}

/// One page of a list ([`LIST_PAGE`] rows from `page * LIST_PAGE`), as of
/// `dropped` older rows gone (the per-connection limit shifts positions).
/// A full page never changes; a partial one goes stale with its flow.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PageKey<L> {
    pub list: L,
    pub page: u32,
    pub dropped: u32,
}

/// One WebSocket message's payload: as forwarded, or (`original`) as it
/// arrived before a rule edited it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WsPayloadKey {
    pub flow: FlowKey,
    pub index: u32,
    pub original: bool,
}

/// An event stream's events matching a search (`sseHeadKey`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SseListKey {
    pub flow: FlowKey,
    pub text: String,
}

/// `wsHeadKey`: the counts, open state and close of a filtered message list.
pub type WsHeadCache = Resource<WsListKey, Arc<WsPage>>;
/// `wsPagesKey`: a page of messages.
pub type WsRowsCache = Resource<PageKey<WsListKey>, Arc<Vec<WsMessage>>>;
/// `wsPayloadKey`: a payload, `None` for an original that isn't there (the
/// message wasn't edited).
pub type WsPayloadCache = Resource<WsPayloadKey, Option<Arc<Vec<u8>>>>;
/// `sseHeadKey`: the counts and open state of an event list.
pub type SseHeadCache = Resource<SseListKey, Arc<SsePage>>;
/// `ssePagesKey`: a page of events.
pub type SseRowsCache = Resource<PageKey<SseListKey>, Arc<Vec<SseEvent>>>;

#[derive(Clone)]
pub struct Caches {
    pub inspected: Entity<InspectedCache>,
    pub operation: Entity<OperationCache>,
    pub body: Entity<BodyCache>,
    pub decoded: Entity<DecodedCache>,
    pub ws_head: Entity<WsHeadCache>,
    pub ws_rows: Entity<WsRowsCache>,
    pub ws_payload: Entity<WsPayloadCache>,
    pub sse_head: Entity<SseHeadCache>,
    pub sse_rows: Entity<SseRowsCache>,
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
    let body: Option<Fetcher<BodyKey, Arc<Vec<u8>>>> = engine.clone().map(|engine| {
        Arc::new(move |k: &BodyKey| {
            engine
                .body_head_in(k.flow.store, k.flow.id, k.part, false, None)
                .map(|b| Arc::new(b.bytes))
                .map_err(|e| e.to_string())
        }) as Fetcher<_, _>
    });
    let ws_head: Option<Fetcher<WsListKey, Arc<WsPage>>> = engine.clone().map(|engine| {
        Arc::new(move |k: &WsListKey| {
            engine
                .websocket_messages_in(k.flow.store, k.flow.id, &k.query(), 0, 0)
                .map(Arc::new)
                .map_err(|e| e.to_string())
        }) as Fetcher<_, _>
    });
    let ws_rows: Option<Fetcher<PageKey<WsListKey>, Arc<Vec<WsMessage>>>> = engine.clone().map(|engine| {
        Arc::new(move |k: &PageKey<WsListKey>| {
            let f = k.list.flow;
            engine
                .websocket_messages_in(f.store, f.id, &k.list.query(), k.page * LIST_PAGE, LIST_PAGE)
                .map(|p| Arc::new(p.messages))
                .map_err(|e| e.to_string())
        }) as Fetcher<_, _>
    });
    let ws_payload: Option<Fetcher<WsPayloadKey, Option<Arc<Vec<u8>>>>> = engine.clone().map(|engine| {
        Arc::new(move |k: &WsPayloadKey| {
            let f = k.flow;
            let bytes = match k.original {
                true => engine.websocket_original_in(f.store, f.id, k.index),
                false => engine.websocket_payload_in(f.store, f.id, k.index).map(Some),
            };
            bytes.map(|b| b.map(Arc::new)).map_err(|e| e.to_string())
        }) as Fetcher<_, _>
    });
    let sse_head: Option<Fetcher<SseListKey, Arc<SsePage>>> = engine.clone().map(|engine| {
        Arc::new(move |k: &SseListKey| {
            engine
                .sse_events_in(k.flow.store, k.flow.id, &k.text, 0, 0)
                .map(Arc::new)
                .map_err(|e| e.to_string())
        }) as Fetcher<_, _>
    });
    let sse_rows: Option<Fetcher<PageKey<SseListKey>, Arc<Vec<SseEvent>>>> = engine.map(|engine| {
        Arc::new(move |k: &PageKey<SseListKey>| {
            let f = k.list.flow;
            engine
                .sse_events_in(f.store, f.id, &k.list.text, k.page * LIST_PAGE, LIST_PAGE)
                .map(|p| Arc::new(p.events))
                .map_err(|e| e.to_string())
        }) as Fetcher<_, _>
    });
    let caches = Caches {
        inspected: cx.new(|_| Resource::new(inspected)),
        operation: cx.new(|_| Resource::new(operation)),
        body: cx.new(|_| Resource::new(body)),
        decoded: cx.new(|_| Resource::new(decoded)),
        ws_head: cx.new(|_| Resource::new(ws_head)),
        ws_rows: cx.new(|_| Resource::new(ws_rows)),
        ws_payload: cx.new(|_| Resource::new(ws_payload)),
        sse_head: cx.new(|_| Resource::new(sse_head)),
        sse_rows: cx.new(|_| Resource::new(sse_rows)),
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
    streams_changed(store, |id| everything || delta.changed.contains(&id), everything, cx);
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

/// New messages or events may have arrived for the flows `changed` picks
/// (a delta names a flow whose message counts or sizes moved): their list
/// heads and partial pages go stale, so the panels refetch only the tail.
/// `everything` (a clear) drops full pages and payloads too.
pub fn streams_changed(store: StoreId, changed: impl Fn(FlowId) -> bool, everything: bool, cx: &mut App) {
    let caches = caches(cx);
    let hit = |k: &FlowKey| k.store == store && changed(k.id);
    let partial = |len: usize| len < LIST_PAGE as usize;
    caches.ws_head.update(cx, |c, cx| c.invalidate(|k, _| hit(&k.flow), cx));
    caches
        .sse_head
        .update(cx, |c, cx| c.invalidate(|k, _| hit(&k.flow), cx));
    caches.ws_rows.update(cx, |c, cx| {
        c.invalidate(
            |k, v| hit(&k.list.flow) && (everything || v.is_none_or(|v| partial(v.len()))),
            cx,
        )
    });
    caches.sse_rows.update(cx, |c, cx| {
        c.invalidate(
            |k, v| hit(&k.list.flow) && (everything || v.is_none_or(|v| partial(v.len()))),
            cx,
        )
    });
    if everything {
        caches
            .ws_payload
            .update(cx, |c, cx| c.invalidate(|k, _| hit(&k.flow), cx));
    }
}

/// A flow's WebSocket changed by our own hand (a send or a close): its
/// lists go stale at once, before the delta that follows.
pub fn websocket_changed(flow: FlowKey, cx: &mut App) {
    streams_changed(flow.store, |id| id == flow.id, false, cx);
}
