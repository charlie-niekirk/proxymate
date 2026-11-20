//! Reusable gpui widgets for the native app, drawn only with `pm-theme`
//! tokens. G0 starts the crate with [`Pill`]; later phases add FlowGrid
//! (G1), ColouredUrl (G1), KvTable (G3), HexView, JsonTree and the
//! CodeView/CodeEditor wrappers (G4), LineDiff (G9) and Waterfall (G3).

mod pill;

pub use pill::Pill;
