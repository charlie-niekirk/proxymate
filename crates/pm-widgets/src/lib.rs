//! Reusable gpui widgets for the native app, drawn only with `pm-theme`
//! tokens: [`Pill`] (G0), [`FlowGrid`](flow_grid::FlowGrid) with its
//! coloured URL and app icons (G1), [`KvTable`] (G3), and the [`Assets`]
//! they draw from. Later phases add HexView, JsonTree and the
//! CodeView/CodeEditor wrappers (G4), JsonEditor and KvEditor (G8) and
//! LineDiff (G9).

mod assets;
pub mod code_editor;
pub mod code_view;
pub mod flow_grid;
pub mod hex_view;
pub mod json_editor;
pub mod json_tree;
pub mod kv_editor;
pub mod kv_table;
mod pill;

pub use assets::Assets;
pub use kv_table::KvTable;
pub use pill::Pill;

/// One of the bundled icons (see [`Assets`]), `size` px, in `color`.
pub fn icon(name: gpui_kit::assets::IconName, color: gpui_kit::Hsla, size: f32) -> gpui_kit::Svg {
    use gpui_kit::{Styled as _, px, svg};
    svg().path(name.path()).flex_none().size(px(size)).text_color(color)
}
