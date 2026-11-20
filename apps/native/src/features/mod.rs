//! Feature modules, one per area (flow table, inspector, rules, …). Each
//! exposes `register(cx)`, which adds its actions, key bindings, palette
//! commands and menu items, so this list is the only central file a phase
//! touches. G0 has none yet; G1 adds the flow table.

use gpui_kit::App;

pub fn register(_cx: &mut App) {}
