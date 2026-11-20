//! The bridge between gpui and the engine. It owns the one multi-thread
//! tokio runtime and the `Arc<Engine>`. Blocking engine calls go through
//! `Engine::blocking` or `cx.background_spawn`, never on the foreground
//! thread, except getters known to be O(1).
//!
//! Engine sinks only enqueue (see [`stream`]); one foreground task per
//! stream drains its channel into a gpui entity and notifies. The bridge
//! subscribes once to each single-slot stream and every window observes the
//! same entity.

pub mod stream;

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{App, Entity, Global};
use pm_core::Engine;
use pm_model::{SidebarSnapshot, StorageStatus};

use stream::Latest;

pub struct Bridge {
    /// Kept for the app's lifetime: the proxy, MCP and engine tasks run on it.
    _runtime: tokio::runtime::Runtime,
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
        Bridge {
            _runtime: runtime,
            engine,
        }
    }

    pub fn global(cx: &App) -> &Bridge {
        cx.global::<Bridge>()
    }

    pub fn engine(cx: &App) -> Arc<Engine> {
        Bridge::global(cx).engine.clone()
    }
}

/// The single-slot streams every window shares, subscribed on first use.
#[derive(Default)]
pub struct Streams {
    storage: Option<Entity<Latest<StorageStatus>>>,
    sidebar: Option<Entity<Latest<SidebarSnapshot>>>,
}

impl Global for Streams {}

/// The storage status (flows, disk, throughput), pushed at most once per
/// publisher tick.
pub fn storage(cx: &mut App) -> Entity<Latest<StorageStatus>> {
    if let Some(entity) = cx.default_global::<Streams>().storage.clone() {
        return entity;
    }
    let engine = Bridge::engine(cx);
    let entity = stream::latest(cx, move |sink| engine.subscribe_storage(sink));
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
