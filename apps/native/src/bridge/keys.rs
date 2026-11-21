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

#[derive(Clone)]
pub struct Caches {
    pub inspected: Entity<InspectedCache>,
    pub operation: Entity<OperationCache>,
    pub body: Entity<BodyCache>,
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

// ---- Rules (G7) ----

/// A script a rule's "Run script" action can name (its source stays out).
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptChoice {
    pub id: String,
    pub name: String,
    pub kind: pm_model::ScriptKind,
    pub enabled: bool,
}

/// `ruleSettingsKey`: the breakpoint timeout.
pub type RuleSettingsCache = Resource<(), pm_model::RuleSettings>;
/// `scriptsKey`, as the rule form's script picker needs it.
pub type ScriptChoicesCache = Resource<(), Vec<ScriptChoice>>;
/// `mockSourcesKey`: the mock library ServeFrom answers from.
pub type MockSourcesCache = Resource<(), Vec<pm_model::MockSource>>;

#[derive(Clone)]
pub struct RulesCaches {
    pub settings: Entity<RuleSettingsCache>,
    pub scripts: Entity<ScriptChoicesCache>,
    pub mock_sources: Entity<MockSourcesCache>,
}

impl Global for RulesCaches {}

/// The Rules window's caches, made on first use.
pub fn rules_caches(cx: &mut App) -> RulesCaches {
    if let Some(caches) = cx.try_global::<RulesCaches>() {
        return caches.clone();
    }
    let engine = cx.try_global::<Bridge>().map(|b| b.engine.clone());
    let settings = engine
        .clone()
        .map(|engine| Arc::new(move |_: &()| Ok(engine.rule_settings())) as Fetcher<(), pm_model::RuleSettings>);
    let scripts = engine.clone().map(|engine| {
        Arc::new(move |_: &()| {
            Ok(engine
                .scripts()
                .into_iter()
                .map(|s| ScriptChoice {
                    id: s.id,
                    name: s.name,
                    kind: s.kind,
                    enabled: s.enabled,
                })
                .collect())
        }) as Fetcher<(), Vec<ScriptChoice>>
    });
    let mock_sources = engine
        .map(|engine| Arc::new(move |_: &()| Ok(engine.mock_sources())) as Fetcher<(), Vec<pm_model::MockSource>>);
    let caches = RulesCaches {
        settings: cx.new(|_| Resource::new(settings)),
        scripts: cx.new(|_| Resource::new(scripts)),
        mock_sources: cx.new(|_| Resource::new(mock_sources)),
    };
    cx.set_global(caches.clone());
    caches
}
