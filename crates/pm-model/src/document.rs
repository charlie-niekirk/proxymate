//! Flow stores beside the live capture: saved sessions (`.proxymate`) and
//! imported HAR files, opened as documents in their own tabs.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::flow::FlowId;
use crate::view::ViewId;

/// Which flow store a flow id belongs to. Ids are only unique within a
/// store: [`LIVE_STORE`] is the capture, each opened document gets its own.
pub type StoreId = u32;

/// The live capture's store.
pub const LIVE_STORE: StoreId = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum DocumentKind {
    /// A saved `.proxymate` session.
    Session,
    /// An imported HAR file.
    Har,
}

/// An opened document, for its tab and the sidebar's Saved section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DocumentInfo {
    pub store: StoreId,
    pub kind: DocumentKind,
    /// The file name, without its directory.
    pub name: String,
    /// Where it was opened from.
    pub path: String,
    pub flows: u32,
}

/// Which flows of a store to save or export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FlowSelection {
    /// Every flow in the store.
    All,
    /// The rows of a view, in its order (the current filter and sort).
    View { view: ViewId },
    /// These flows, in this order.
    Ids { ids: Vec<FlowId> },
}

/// Progress of a background save (session or HAR), pushed as it goes. The
/// last update has `finished` set, with `error` if it failed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
pub struct SaveProgress {
    pub done: u32,
    pub total: u32,
    pub finished: bool,
    pub error: Option<String>,
    /// The file written.
    pub path: String,
}
