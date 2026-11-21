//! Feature modules, one per area (flow table, inspector, rules, …). Each
//! exposes `register(cx)`, which adds its actions, key bindings, palette
//! commands and menu items, so this list is the only central file a phase
//! touches. The command registry binds every command's key last, once all
//! of them are registered.

use gpui_kit::App;

pub mod capture;
pub mod chrome;
pub mod clipboard;
pub mod commands;
pub mod context_menu;
pub mod documents;
pub mod flow_table;
pub mod navigate;

pub fn register(cx: &mut App) {
    clipboard::register(cx);
    commands::register(cx);
    capture::register(cx);
    chrome::register(cx);
    documents::register(cx);
    flow_table::register(cx);
    navigate::register(cx);
    commands::bind_keys(cx);
}
