//! The command registry (`lib/commands.ts` and `hooks/useShortcuts.ts`):
//! `pm_view::commands` lists every command; feature modules register a
//! handler per id in their `register` (with when it applies). ⌘K opens
//! the palette with the commands that have a handler and apply now,
//! grouped as the Tauri palette groups them, with their shortcuts; a
//! command's key runs it from anywhere in the window.
//!
//! Commands that later phases build have no handler yet, so they are
//! neither listed nor bound.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::Focusable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::prelude::*;
use gpui_kit::{App, Global, KeyBinding, SharedString, Window, div, px};
use pm_theme::kit::TokensExt as _;
use pm_view::commands::{self as list, Group};

type Run = Rc<dyn Fn(&mut Window, &mut App)>;
type Enabled = Rc<dyn Fn(&App) -> bool>;

struct Handler {
    enabled: Enabled,
    run: Run,
}

#[derive(Default)]
pub struct Registry {
    handlers: HashMap<String, Handler>,
    bound: bool,
}

impl Global for Registry {}

/// Runs the command with this id (the palette; menu items).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct RunCommand {
    pub id: SharedString,
}

/// A command's key: runs whichever command with that key applies (⌘E
/// starts or stops recording).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct RunShortcut {
    pub keystroke: SharedString,
}

gpui_kit::actions!(proxymate, [TogglePalette]);

pub const MAC: bool = cfg!(target_os = "macos");

/// Commands whose key a focused text field keeps: ⌘⌫ deletes to the start
/// of the line there, not every flow.
const TEXT_FIELDS_KEEP: [&str; 1] = ["clear"];

/// Commands whose key the body viewers keep (G4): ⌘F searches the body in
/// a CodeView or code editor, not the flows.
const CODE_VIEWS_KEEP: [&str; 1] = ["search"];
/// Where [`CODE_VIEWS_KEEP`] keys step aside: a focused CodeView (or its
/// find bar) and the code editor's input.
const CODE_VIEWS: &str = "!CodeView && !CodeViewFind && !(Input && mode == editor)";

pub fn register(cx: &mut App) {
    cx.default_global::<Registry>();
    cx.on_action(|a: &RunCommand, cx| {
        let id = a.id.clone();
        in_active_window(cx, move |window, cx| {
            run(&id, window, cx);
        });
    });
    cx.on_action(|a: &RunShortcut, cx| {
        let keystroke = a.keystroke.clone();
        in_active_window(cx, move |window, cx| run_shortcut(&keystroke, window, cx));
    });
    cx.on_action(|_: &TogglePalette, cx| in_active_window(cx, toggle_palette));
}

/// Runs `f` on the active window once the current dispatch is over (an
/// action is handled while its window is still dispatching it).
fn in_active_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    if let Some(handle) = cx.active_window().or_else(|| cx.windows().first().copied()) {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| f(window, cx));
        });
    }
}

/// Adds the handler for command `id` (an id of `pm_view::commands`).
pub fn add(
    id: &str,
    enabled: impl Fn(&App) -> bool + 'static,
    run: impl Fn(&mut Window, &mut App) + 'static,
    cx: &mut App,
) {
    debug_assert!(list::get(id).is_some(), "unknown command {id}");
    cx.default_global::<Registry>().handlers.insert(
        id.to_owned(),
        Handler {
            enabled: Rc::new(enabled),
            run: Rc::new(run),
        },
    );
}

/// Always applies.
pub fn always(_: &App) -> bool {
    true
}

/// Binds the keys of every registered command, and ⌘K. Call once, after
/// every feature has registered. Bare and table-only keys belong to the
/// flow table's own bindings (`FlowGrid`).
pub fn bind_keys(cx: &mut App) {
    let registry = cx.default_global::<Registry>();
    if registry.bound {
        return;
    }
    registry.bound = true;
    let ids: Vec<String> = registry.handlers.keys().cloned().collect();
    // Keystroke → the context it yields to text fields or code views in.
    let mut strokes: Vec<(String, Option<&'static str>)> = list::all()
        .into_iter()
        .filter(|c| ids.iter().any(|id| *id == c.id) && !c.bare && !c.in_table)
        .filter_map(|c| {
            let id = c.id.as_ref();
            let context = if TEXT_FIELDS_KEEP.contains(&id) {
                Some("!Input")
            } else if CODE_VIEWS_KEEP.contains(&id) {
                Some(CODE_VIEWS)
            } else {
                None
            };
            Some((c.keystroke(MAC)?, context))
        })
        .collect();
    strokes.sort();
    strokes.dedup_by(|a, b| a.0 == b.0);
    let primary = if MAC { "cmd" } else { "ctrl" };
    // Key bindings take `'static` keystrokes; these are made once, at start.
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let mut bindings = vec![KeyBinding::new(leak(format!("{primary}-k")), TogglePalette, None)];
    // A binding with no context ranks as deep as the focus, so it would
    // beat a text field's own; `!Input` steps aside for them.
    bindings.extend(strokes.into_iter().map(|(stroke, context)| {
        KeyBinding::new(
            leak(stroke.clone()),
            RunShortcut {
                keystroke: stroke.into(),
            },
            context,
        )
    }));
    cx.bind_keys(bindings);
}

fn handler(id: &str, cx: &App) -> Option<(Enabled, Run)> {
    let h = cx.try_global::<Registry>()?.handlers.get(id)?;
    Some((h.enabled.clone(), h.run.clone()))
}

/// Whether `id` has a handler and applies now.
pub fn is_available(id: &str, cx: &App) -> bool {
    handler(id, cx).is_some_and(|(enabled, _)| enabled(cx))
}

/// The commands the palette lists now, in the list's order.
pub fn available(cx: &App) -> Vec<list::Command> {
    list::all().into_iter().filter(|c| is_available(&c.id, cx)).collect()
}

/// Runs `id` if it applies; false otherwise.
pub fn run(id: &str, window: &mut Window, cx: &mut App) -> bool {
    let Some((enabled, run)) = handler(id, cx) else {
        return false;
    };
    if !enabled(cx) {
        return false;
    }
    run(window, cx);
    true
}

fn run_shortcut(keystroke: &str, window: &mut Window, cx: &mut App) {
    let found = list::all()
        .into_iter()
        .find(|c| c.keystroke(MAC).as_deref() == Some(keystroke) && is_available(&c.id, cx));
    if let Some(command) = found {
        run(&command.id, window, cx);
    }
}

/// Whether the palette is open in `window`.
#[derive(Default)]
struct PaletteOpen(bool);

impl Global for PaletteOpen {}

/// ⌘K: opens the palette, or closes it.
pub fn toggle_palette(window: &mut Window, cx: &mut App) {
    if cx.default_global::<PaletteOpen>().0 && window.has_active_dialog(cx) {
        window.close_dialog(cx);
        cx.default_global::<PaletteOpen>().0 = false;
        return;
    }
    open_palette(window, cx);
}

pub fn is_palette_open(cx: &App) -> bool {
    cx.try_global::<PaletteOpen>().is_some_and(|p| p.0)
}

/// The palette's groups as listed now: each group's commands (ids).
pub fn palette_groups(cx: &App) -> Vec<(Group, Vec<list::Command>)> {
    let commands = available(cx);
    Group::ALL
        .iter()
        .map(|g| {
            (
                *g,
                commands.iter().filter(|c| c.group == *g).cloned().collect::<Vec<_>>(),
            )
        })
        .filter(|(_, items)| !items.is_empty())
        .collect()
}

pub fn open_palette(window: &mut Window, cx: &mut App) {
    // Which commands apply depends on the selection and the tab, as of opening.
    let groups = Rc::new(palette_groups(cx));
    let state = cx.new(|cx| CommandState::new(window, cx));
    let first = Cell::new(true);
    cx.default_global::<PaletteOpen>().0 = true;
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            state.read(cx).focus_handle(cx).focus(window, cx);
        }
        let muted = cx.tokens().muted_foreground.hsla();
        let mut palette = Command::new(&state)
            .placeholder("Type a command…")
            .empty(|_, _, _| div().p(px(12.)).text_size(px(13.)).child("No matching command"));
        for (group, commands) in groups.iter() {
            let items = commands.iter().map(|c| {
                let title: SharedString = c.title.clone().into_owned().into();
                let shortcut = c.shortcut(MAC);
                let label = title.clone();
                CommandItem::new()
                    .label(title)
                    .keywords(c.keywords.iter().copied())
                    .child(move |_, _| {
                        div()
                            .flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .child(label.clone())
                            .children(
                                shortcut
                                    .clone()
                                    .map(|s| div().text_size(px(11.)).text_color(muted).child(s)),
                            )
                    })
            });
            palette = palette.group(CommandGroup::new().label(group.label()).items(items));
        }
        let groups = groups.clone();
        palette = palette
            .on_confirm(move |path, window, cx| {
                let id = groups
                    .get(path.section)
                    .and_then(|(_, items)| items.get(path.row))
                    .map(|c| c.id.to_string());
                close_palette(window, cx);
                if let Some(id) = id {
                    // After the dialog has closed and handed focus back.
                    let handle = window.window_handle();
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| run(&id, window, cx));
                    });
                }
            })
            .on_cancel(close_palette);
        dialog
            // A palette is there at once, as the Tauri one is.
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .w(px(520.))
            .close_button(false)
            .margin_top(px(80.))
            .on_close(|_, _, cx| cx.default_global::<PaletteOpen>().0 = false)
            .child(palette)
    });
}

fn close_palette(window: &mut Window, cx: &mut App) {
    cx.default_global::<PaletteOpen>().0 = false;
    window.close_dialog(cx);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::TestAppContext;

    use super::*;

    struct Blank;

    impl gpui_kit::Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui_kit::test]
    fn only_registered_commands_that_apply_are_listed_and_keys_pick_the_one_that_applies(cx: &mut TestAppContext) {
        let ran = Rc::new(RefCell::new(Vec::new()));
        let recording = Rc::new(Cell::new(true));
        cx.update(|cx| {
            gpui_kit::init(cx);
            register(cx);
            let (r, rec) = (ran.clone(), recording.clone());
            add(
                "record-stop",
                move |_| rec.get(),
                move |_, _| r.borrow_mut().push("stop"),
                cx,
            );
            let (r, rec) = (ran.clone(), recording.clone());
            add(
                "record-start",
                move |_| !rec.get(),
                move |_, _| r.borrow_mut().push("start"),
                cx,
            );
            let r = ran.clone();
            add("open", always, move |_, _| r.borrow_mut().push("open"), cx);
        });
        let ids = |cx: &mut TestAppContext| -> Vec<String> {
            cx.update(|cx| available(cx).into_iter().map(|c| c.id.into_owned()).collect())
        };
        assert_eq!(ids(cx), ["record-stop", "open"]);
        let window = cx.add_window(|_, _| Blank);
        let cmd_e = if MAC { "cmd-e" } else { "ctrl-e" };
        let _ = window.update(cx, |_, window, cx| run_shortcut(cmd_e, window, cx));
        recording.set(false);
        assert_eq!(ids(cx), ["record-start", "open"]);
        let _ = window.update(cx, |_, window, cx| run_shortcut(cmd_e, window, cx));
        let _ = window.update(cx, |_, window, cx| assert!(!run("record-stop", window, cx)));
        assert_eq!(*ran.borrow(), ["stop", "start"]);
        let groups = cx.update(|cx| palette_groups(cx));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, Group::Capture);
    }
}
