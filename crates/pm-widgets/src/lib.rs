//! Reusable gpui widgets for the native app, drawn only with `pm-theme`
//! tokens: [`Pill`] (G0), [`FlowGrid`](flow_grid::FlowGrid) with its
//! coloured URL and app icons (G1), and the [`Assets`] they draw from.
//! Later phases add KvTable (G3), HexView, JsonTree and the
//! CodeView/CodeEditor wrappers (G4), LineDiff (G9) and Waterfall (G3).

mod assets;
pub mod flow_grid;
mod pill;

pub use assets::Assets;
pub use pill::Pill;
