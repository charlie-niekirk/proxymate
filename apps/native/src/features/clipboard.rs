//! The one way the app writes to the clipboard, and its toasts (ported
//! from `lib/clipboard.ts` and `reportError` in `lib/actions.ts`). Menus,
//! buttons and commands all copy through here, so every copy confirms
//! itself ("Copied header value", 1.2 s, one toast replacing the last) or
//! explains a failure the same way. Errors from actions are toasts too:
//! what failed as the title, the reason as the description.
//!
//! Menus copy with the [`CopyText`] action, handled app-wide, so a native
//! menu item needs no view to land on.

use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::{App, AppContext as _, ClipboardItem, Global, Image, ImageFormat, SharedString, Window};

/// Where toasts stack: bottom right, as the Tauri app's do, clear of the
/// toolbar and the search box.
const PLACEMENT: gpui_kit::Anchor = gpui_kit::Anchor::BottomRight;

/// How long "Copied …" stays.
pub const COPIED_FOR: Duration = Duration::from_millis(1200);

/// Copies `text`; `what` names it in the toast. A menu item's action.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct CopyText {
    pub text: SharedString,
    pub what: SharedString,
}

impl CopyText {
    pub fn new(text: impl Into<SharedString>, what: impl Into<SharedString>) -> CopyText {
        CopyText {
            text: text.into(),
            what: what.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
}

/// A toast as shown, kept for tests and the dev remote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub kind: ToastKind,
    pub title: SharedString,
    pub description: Option<SharedString>,
    /// The label of its button, if it has one.
    pub action: Option<SharedString>,
}

/// The toasts shown so far (the last few), and the copy toast's sequence
/// number, so an old toast's timer doesn't close a newer one.
#[derive(Default)]
pub struct Toasts {
    pub shown: Vec<Toast>,
    copied_seq: u64,
}

impl Global for Toasts {}

impl Toasts {
    pub fn last(cx: &App) -> Option<Toast> {
        cx.try_global::<Toasts>()?.shown.last().cloned()
    }
}

/// The copy toast's notification id: one at a time.
struct CopiedToast;

pub fn register(cx: &mut App) {
    cx.on_action(|a: &CopyText, cx| copy(a.text.as_ref(), Some(a.what.as_ref()), cx));
}

/// Copies `text`; `what` names it in the toast ("Copied header value").
pub fn copy(text: &str, what: Option<&str>, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
    copied(what, cx);
}

/// Copies `text` without a toast: for buttons that show their own tick.
pub fn copy_silent(text: &str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
}

/// Copies text that has to be fetched first (a body, a cURL command), on
/// the background executor. A failure to fetch toasts like a failure to
/// copy; `Ok(None)` copies nothing.
pub fn copy_later(
    load: impl FnOnce() -> Result<Option<String>, String> + Send + 'static,
    what: impl Into<SharedString>,
    cx: &mut App,
) {
    let what = what.into();
    let task = cx.background_spawn(async move { load() });
    cx.spawn(async move |cx| {
        let loaded = task.await;
        cx.update(|cx| match loaded {
            Ok(Some(text)) => copy(&text, Some(&what), cx),
            Ok(None) => {}
            Err(err) => couldnt_copy(&err, cx),
        });
    })
    .detach();
}

/// Copies a PNG image (the clipboard takes PNG for images).
pub fn copy_image(png: Vec<u8>, what: &str, cx: &mut App) {
    let image = Image::from_bytes(ImageFormat::Png, png);
    cx.write_to_clipboard(ClipboardItem::new_image(&image));
    copied(Some(what), cx);
}

fn copied(what: Option<&str>, cx: &mut App) {
    let title: SharedString = match what {
        Some(what) if !what.is_empty() => format!("Copied {what}").into(),
        _ => "Copied".into(),
    };
    let toasts = cx.default_global::<Toasts>();
    toasts.copied_seq += 1;
    let seq = toasts.copied_seq;
    record(
        Toast {
            kind: ToastKind::Success,
            title: title.clone(),
            description: None,
            action: None,
        },
        cx,
    );
    in_window(cx, |window, cx| {
        let note = Notification::success(title)
            .placement(PLACEMENT)
            .id::<CopiedToast>()
            .autohide(false);
        window.push_notification(note, cx);
    });
    cx.spawn(async move |cx| {
        cx.background_executor().timer(COPIED_FOR).await;
        cx.update(|cx| {
            if cx.default_global::<Toasts>().copied_seq == seq {
                in_window(cx, |window, cx| window.remove_notification::<CopiedToast>(cx));
            }
        });
    })
    .detach();
}

fn couldnt_copy(err: &str, cx: &mut App) {
    toast(ToastKind::Error, "Couldn't copy", Some(err.to_owned()), cx);
}

/// A failed action: what was being done as the title, then why.
pub fn report_error(what: &str, err: impl std::fmt::Display, cx: &mut App) {
    toast(ToastKind::Error, what, Some(err.to_string()), cx);
}

/// A success toast (an intercepted launch: "Started Google Chrome").
pub fn success(title: &str, description: Option<String>, cx: &mut App) {
    toast(ToastKind::Success, title, description, cx);
}

/// An informational toast (a stub's "arrives in G…" note, say).
pub fn info(title: &str, description: Option<String>, cx: &mut App) {
    toast(ToastKind::Info, title, description, cx);
}

fn toast(kind: ToastKind, title: &str, description: Option<String>, cx: &mut App) {
    if kind == ToastKind::Error {
        eprintln!("{title}: {}", description.as_deref().unwrap_or(""));
    }
    let toast = Toast {
        kind,
        title: title.to_owned().into(),
        description: description.map(Into::into),
        action: None,
    };
    record(toast.clone(), cx);
    in_window(cx, move |window, cx| {
        let kind = match toast.kind {
            ToastKind::Success => NotificationType::Success,
            ToastKind::Error => NotificationType::Error,
            ToastKind::Info => NotificationType::Info,
        };
        let mut note = Notification::new()
            .placement(PLACEMENT)
            .with_type(kind)
            .title(toast.title.clone());
        if let Some(description) = toast.description.clone() {
            note = note.message(description);
        }
        window.push_notification(note, cx);
    });
}

/// A success toast with a button (`Not recording x` › Settings): it stays
/// until dismissed or its button is clicked, which dispatches `action`.
pub fn success_with_action(
    title: &str,
    description: &str,
    label: &str,
    action: Box<dyn gpui_kit::Action>,
    cx: &mut App,
) {
    let toast = Toast {
        kind: ToastKind::Success,
        title: title.to_owned().into(),
        description: Some(description.to_owned().into()),
        action: Some(label.to_owned().into()),
    };
    record(toast.clone(), cx);
    in_window(cx, move |window, cx| {
        let label = toast.action.clone().unwrap_or_default();
        let note = Notification::new()
            .placement(PLACEMENT)
            .with_type(NotificationType::Success)
            .title(toast.title.clone())
            .message(toast.description.clone().unwrap_or_default())
            .action(move |_, _, _| {
                let action = action.boxed_clone();
                gpui_kit::component::button::Button::new("toast-action")
                    .label(label.clone())
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            });
        window.push_notification(note, cx);
    });
}

fn record(toast: Toast, cx: &mut App) {
    let shown = &mut cx.default_global::<Toasts>().shown;
    shown.push(toast);
    if shown.len() > 20 {
        shown.remove(0);
    }
}

/// Runs `f` on the active window (or the first) if it has gpui-kit's root,
/// which draws the toasts; deferred, since a copy usually runs while its
/// window is still dispatching the menu's action.
fn in_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    let Some(handle) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        return;
    };
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            if window.root::<gpui_kit::base::Root>().flatten().is_some() {
                f(window, cx);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::*;

    #[gpui_kit::test]
    fn copies_toast_and_menus_copy_through_the_action(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            register(cx);
            copy("hello", Some("greeting"), cx);
        });
        assert_eq!(
            cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
            Some("hello")
        );
        let last = cx.update(|cx| Toasts::last(cx)).unwrap();
        assert_eq!(last.title.as_ref(), "Copied greeting");
        cx.update(|cx| cx.dispatch_action(&CopyText::new("127.0.0.1:9090", "proxy address")));
        assert_eq!(
            cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
            Some("127.0.0.1:9090")
        );
        assert_eq!(
            cx.update(|cx| Toasts::last(cx)).unwrap().title.as_ref(),
            "Copied proxy address"
        );
    }

    #[gpui_kit::test]
    fn copy_later_copies_what_loads_and_reports_failures(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            copy_later(|| Ok(Some("fetched".into())), "body", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
            Some("fetched")
        );
        cx.update(|cx| copy_later(|| Err("gone".into()), "body", cx));
        cx.run_until_parked();
        let last = cx.update(|cx| Toasts::last(cx)).unwrap();
        assert_eq!((last.kind, last.title.as_ref()), (ToastKind::Error, "Couldn't copy"));
        assert_eq!(last.description.as_deref(), Some("gone"));
        assert_eq!(
            cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
            Some("fetched")
        );
    }
}
