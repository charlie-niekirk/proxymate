//! Navigate and Settings commands (`lib/commands.ts`): Search flows (⌘F,
//! focuses and selects the search box), Filter apps and domains (⌘L), the
//! pinned, labelled and commented filters, the body search, and
//! Appearance: System, Light or Dark.

use gpui_kit::App;

use crate::features::commands::{self, always};
use crate::features::documents::with_workspace;
use crate::prefs::Appearance;
use crate::shell;

pub fn register(cx: &mut App) {
    commands::add("search", always, shell::focus_search, cx);
    commands::add("filter", always, shell::focus_sidebar_filter, cx);
    for (id, text) in [
        ("pinned", "is:pinned"),
        ("labelled", "is:labelled"),
        ("commented", "is:commented"),
    ] {
        commands::add(
            id,
            always,
            move |_, cx| {
                with_workspace(cx, |w, cx| w.set_text(text, cx));
            },
            cx,
        );
    }
    commands::add(
        "deep-search",
        always,
        |window, cx| {
            with_workspace(cx, |w, cx| {
                let text = pm_view::commands::with_body_term(&w.filter.text);
                w.set_text(text, cx);
            });
            shell::focus_search_at_end(window, cx);
        },
        cx,
    );
    for (id, appearance) in [
        ("appearance-system", Appearance::System),
        ("appearance-light", Appearance::Light),
        ("appearance-dark", Appearance::Dark),
    ] {
        commands::add(id, always, move |_, cx| crate::appearance::set(appearance, cx), cx);
    }
}
