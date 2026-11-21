//! The bridge between gpui and the engine. It owns the one multi-thread
//! tokio runtime and the `Arc<Engine>`. Blocking engine calls go through
//! `Engine::blocking` or `cx.background_spawn`, never on the foreground
//! thread, except getters known to be O(1).
//!
//! Engine sinks only enqueue (see [`stream`]); one foreground task per
//! stream drains its channel into a gpui entity and notifies. The bridge
//! subscribes once to each single-slot stream and every window observes the
//! same entity.
//!
//! Server state that views read on demand (a flow's detail, a body) is in
//! keyed [`resource`] caches ([`keys`]); writes go through [`mutations`],
//! which invalidate what they make stale.

pub mod keys;
pub mod mutations;
pub mod resource;
pub mod stream;

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{App, Entity, Global};
use pm_core::Engine;
use pm_model::{BreakpointInfo, DevicesSnapshot, McpStatus, RuleBook, SidebarSnapshot, StorageStatus};

use stream::Latest;

pub struct Bridge {
    /// Kept for the app's lifetime: the proxy, MCP and engine tasks run on it.
    runtime: tokio::runtime::Runtime,
    engine: Arc<Engine>,
}

impl Global for Bridge {}

impl Bridge {
    /// Builds the runtime and starts the engine, the proxy, the exit guards
    /// and MCP, before gpui starts (so a proxy error is known at first frame).
    pub fn start(data_dir: Option<PathBuf>) -> Bridge {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("pm-tokio")
            .build()
            .expect("tokio runtime");
        let engine = pm_app::start_engine(runtime.handle(), data_dir);
        pm_app::install_exit_guards(runtime.handle(), &engine);
        pm_app::spawn_mcp(runtime.handle(), engine.clone());
        Bridge { runtime, engine }
    }

    pub fn global(cx: &App) -> &Bridge {
        cx.global::<Bridge>()
    }

    pub fn engine(cx: &App) -> Arc<Engine> {
        Bridge::global(cx).engine.clone()
    }

    /// The tokio runtime, for async work that needs it (the updater's HTTP).
    pub fn runtime(cx: &App) -> tokio::runtime::Handle {
        Bridge::global(cx).runtime.handle().clone()
    }

    /// The runtime async engine writes run on (G12's settings): the
    /// bridge's, or the one a test set with [`set_runtime`].
    pub fn try_runtime(cx: &App) -> Option<tokio::runtime::Handle> {
        cx.try_global::<Bridge>()
            .map(|b| b.runtime.handle().clone())
            .or_else(|| cx.try_global::<TestRuntime>().map(|r| r.0.clone()))
    }
}

/// A runtime for async engine writes when there is no bridge (tests that
/// drive a window against their own engine).
struct TestRuntime(tokio::runtime::Handle);

impl Global for TestRuntime {}

/// Lets mutations reach async engine methods without a bridge (tests).
pub fn set_runtime(handle: tokio::runtime::Handle, cx: &mut App) {
    cx.set_global(TestRuntime(handle));
}

/// The single-slot streams every window shares, subscribed on first use.
#[derive(Default)]
pub struct Streams {
    storage: Option<Entity<Latest<StorageStatus>>>,
    sidebar: Option<Entity<Latest<SidebarSnapshot>>>,
    rules: Option<Entity<Latest<RuleBook>>>,
    breakpoints: Option<Entity<Latest<Vec<BreakpointInfo>>>>,
    devices: Option<Entity<Latest<DevicesSnapshot>>>,
    mcp: Option<Entity<Latest<McpStatus>>>,
}

impl Global for Streams {}

/// The storage status (flows, disk, throughput), pushed at most once per
/// publisher tick.
pub fn storage(cx: &mut App) -> Entity<Latest<StorageStatus>> {
    if let Some(entity) = cx.default_global::<Streams>().storage.clone() {
        return entity;
    }
    let entity = match cx.try_global::<Bridge>().map(|b| b.engine.clone()) {
        Some(engine) => stream::latest(cx, move |sink| engine.subscribe_storage(sink)),
        None => stream::latest(cx, |_| {}),
    };
    cx.global_mut::<Streams>().storage = Some(entity.clone());
    entity
}

/// The sidebar's Apps and Domains, counted over the tab shown, pushed when
/// they change (at most every 250 ms).
pub fn sidebar(cx: &mut App) -> Entity<Latest<SidebarSnapshot>> {
    if let Some(entity) = cx.default_global::<Streams>().sidebar.clone() {
        return entity;
    }
    let engine = Bridge::engine(cx);
    let entity = stream::latest(cx, move |sink| engine.subscribe_sidebar(sink));
    cx.global_mut::<Streams>().sidebar = Some(entity.clone());
    entity
}

/// The rules and groups (G7), pushed at first and whenever the UI or an
/// agent changes them. Without an engine (tests, offscreen renders) the
/// entity is never fed: tests set its value.
pub fn rule_book(cx: &mut App) -> Entity<Latest<RuleBook>> {
    if let Some(entity) = cx.default_global::<Streams>().rules.clone() {
        return entity;
    }
    let entity = match cx.try_global::<Bridge>().map(|b| b.engine.clone()) {
        Some(engine) => stream::latest(cx, move |sink| engine.subscribe_rules(sink)),
        None => stream::latest(cx, |_| {}),
    };
    cx.global_mut::<Streams>().rules = Some(entity.clone());
    entity
}

/// The flows paused at a breakpoint (the toolbar's badge; G8's editor),
/// pushed whenever the queue changes. Without an engine, as [`rule_book`].
pub fn breakpoints(cx: &mut App) -> Entity<Latest<Vec<BreakpointInfo>>> {
    if let Some(entity) = cx.default_global::<Streams>().breakpoints.clone() {
        return entity;
    }
    let entity = match cx.try_global::<Bridge>().map(|b| b.engine.clone()) {
        Some(engine) => stream::latest(cx, move |sink| engine.subscribe_breakpoints(sink)),
        None => stream::latest(cx, |_| {}),
    };
    cx.global_mut::<Streams>().breakpoints = Some(entity.clone());
    entity
}

/// LAN devices and their activity (Settings › Clients; G13's prompts),
/// pushed when one connects, is decided or forgotten. Without an engine,
/// as [`rule_book`].
pub fn devices(cx: &mut App) -> Entity<Latest<DevicesSnapshot>> {
    if let Some(entity) = cx.default_global::<Streams>().devices.clone() {
        return entity;
    }
    let entity = match cx.try_global::<Bridge>().map(|b| b.engine.clone()) {
        Some(engine) => stream::latest(cx, move |sink| engine.subscribe_devices(sink)),
        None => stream::latest(cx, |_| {}),
    };
    cx.global_mut::<Streams>().devices = Some(entity.clone());
    entity
}

/// The MCP server's state: its settings, where it serves or why not, and
/// the connected clients (Settings › MCP; G13's indicator). Pushed when
/// any of it changes. Without an engine, as [`rule_book`].
pub fn mcp(cx: &mut App) -> Entity<Latest<McpStatus>> {
    if let Some(entity) = cx.default_global::<Streams>().mcp.clone() {
        return entity;
    }
    let entity = match cx.try_global::<Bridge>().map(|b| b.engine.clone()) {
        Some(engine) => stream::latest(cx, move |sink| engine.subscribe_mcp(sink)),
        None => stream::latest(cx, |_| {}),
    };
    cx.global_mut::<Streams>().mcp = Some(entity.clone());
    entity
}
