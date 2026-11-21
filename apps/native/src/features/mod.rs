//! Feature modules, one per area (flow table, inspector, rules, …). Each
//! exposes `register(cx)`, which adds its actions, key bindings, palette
//! commands and menu items, so this list is the only central file a phase
//! touches. The command registry binds every command's key last, once all
//! of them are registered.

use gpui_kit::App;

pub mod body;
pub mod breakpoints;
pub mod capture;
pub mod chrome;
pub mod clipboard;
pub mod commands;
pub mod compose;
pub mod context_menu;
pub mod documents;
pub mod editing;
pub mod flow_menu;
pub mod flow_table;
pub mod inspector;
pub mod navigate;
pub mod replay;
pub mod rules;

pub fn register(cx: &mut App) {
    clipboard::register(cx);
    commands::register(cx);
    body::register(cx);
    capture::register(cx);
    chrome::register(cx);
    documents::register(cx);
    flow_table::register(cx);
    inspector::register(cx);
    navigate::register(cx);
    rules::register(cx);
    editing::register(cx);
    breakpoints::register(cx);
    compose::register(cx);
    replay::register(cx);
    commands::bind_keys(cx);
}
