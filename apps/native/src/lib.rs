//! Proxymate on gpui and gpui-kit (the Native track in `docs/PLAN.md`): the
//! app's modules, as a library so `tests/` can drive its views with injected
//! snapshots and no engine. `main.rs` is the binary.

pub mod app;
pub mod appearance;
pub mod bridge;
#[cfg(feature = "dev-harness")]
pub mod dev;
pub mod features;
pub mod menus;
pub mod prefs;
pub mod shell;
