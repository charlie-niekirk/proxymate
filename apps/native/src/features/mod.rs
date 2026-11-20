//! Feature modules, one per area (flow table, inspector, rules, …). Each
//! exposes `register(cx)`, which adds its actions, key bindings, palette
//! commands and menu items, so this list is the only central file a phase
//! touches.

use gpui_kit::App;

pub mod flow_table;

pub fn register(cx: &mut App) {
    flow_table::register(cx);
}
