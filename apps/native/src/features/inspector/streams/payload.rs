//! A WebSocket payload or an event's data (`PayloadView.tsx`): a bar with
//! the viewer picker and "type · size", then the bytes as selectable text
//! (a virtualised list of chunks sharing one selection, as the Raw tab
//! draws it) or as a hex dump (a `uniform_list` of 16-byte rows). G4's
//! viewers (JSON, XML, …) can replace both later; until then Text and Hex
//! are the choices, binary frames starting on Hex.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::{SelectableText, TextSelectionHandle};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ElementId, ListAlignment, ListState, Role, SharedString, Toggled, div, list, px, uniform_list,
};
use pm_theme::Tokens;
use pm_view::copy_formats::{HEX_WIDTH, hex_row};
use pm_view::flow::format_size;

/// A chunk of a long line is at most this many bytes.
const CHUNK: usize = 2048;

/// How the bytes are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Viewer {
    Text,
    Hex,
}

impl Viewer {
    pub fn label(self) -> &'static str {
        match self {
            Viewer::Text => "Text",
            Viewer::Hex => "Hex",
        }
    }

    pub fn parse(s: &str) -> Option<Viewer> {
        [Viewer::Text, Viewer::Hex]
            .into_iter()
            .find(|v| v.label().eq_ignore_ascii_case(s))
    }
}

/// What the bytes look like: a MIME type for the bar, and the viewer they
/// open in. Binary frames open as hex unless they hold JSON.
pub fn sniff(bytes: &[u8], binary: bool) -> (&'static str, Viewer) {
    match pm_view::http::body_text(bytes) {
        Some(text) if matches!(text.trim_start().chars().next(), Some('{' | '[')) => ("application/json", Viewer::Text),
        Some(_) if !binary => ("text/plain", Viewer::Text),
        Some(_) => ("text/plain", Viewer::Hex),
        None => ("application/octet-stream", Viewer::Hex),
    }
}

/// The shown payload and its list states, rebuilt when the bytes change.
pub struct PayloadDoc {
    bytes: Arc<Vec<u8>>,
    pub mime: &'static str,
    pub viewer: Viewer,
    text: SharedString,
    chunks: Rc<Vec<Range<usize>>>,
    list: ListState,
    handle: TextSelectionHandle,
    hex: gpui_kit::UniformListScrollHandle,
}

impl PayloadDoc {
    pub fn new(bytes: Arc<Vec<u8>>, binary: bool, cx: &mut App) -> PayloadDoc {
        let (mime, viewer) = sniff(&bytes, binary);
        let text: SharedString = String::from_utf8_lossy(&bytes).into_owned().into();
        let chunks = Rc::new(chunks(&text));
        PayloadDoc {
            mime,
            viewer,
            list: ListState::new(chunks.len(), ListAlignment::Top, px(400.)),
            chunks,
            handle: TextSelectionHandle::new(text.clone(), cx),
            text,
            hex: gpui_kit::UniformListScrollHandle::new(),
            bytes,
        }
    }

    /// True if this doc shows `bytes` (the same allocation).
    pub fn shows(&self, bytes: &Arc<Vec<u8>>) -> bool {
        Arc::ptr_eq(&self.bytes, bytes)
    }

    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.bytes
    }

    /// The bar and the viewer. `pick` switches the viewer.
    pub fn element(
        &self,
        id: &'static str,
        t: &Tokens,
        pick: impl Fn(Viewer, &mut gpui_kit::Window, &mut App) + Clone + 'static,
    ) -> AnyElement {
        if self.bytes.is_empty() {
            return centered("Empty payload", t);
        }
        let chips = [Viewer::Text, Viewer::Hex].map(|v| {
            let pick = pick.clone();
            chip(
                SharedString::from(format!("{id}-viewer-{}", v.label().to_lowercase())),
                v.label(),
                self.viewer == v,
                None,
                t,
                move |_, window, cx| pick(v, window, cx),
            )
        });
        let bar = div()
            .flex_none()
            .h(px(26.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(t.border.hsla())
            .bg(t.pane_toolbar.hsla())
            .text_size(px(11.))
            .child(
                div()
                    .id(SharedString::from(format!("{id}-viewers")))
                    .role(Role::RadioGroup)
                    .aria_label("Viewer")
                    .flex()
                    .gap(px(2.))
                    .children(chips),
            )
            .child(
                div()
                    .truncate()
                    .font_family("Menlo")
                    .text_color(t.muted_foreground.hsla())
                    .child(format!("{} · {}", self.mime, format_size(self.bytes.len() as u64))),
            );
        let body = match self.viewer {
            Viewer::Text => self.text_element(id).into_any_element(),
            Viewer::Hex => self.hex_element(id, t).into_any_element(),
        };
        div()
            .id(SharedString::from(format!("{id}-payload")))
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(bar)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    fn text_element(&self, id: &'static str) -> impl IntoElement + use<> {
        let text = self.text.clone();
        let chunks = self.chunks.clone();
        let handle = self.handle.clone();
        list(self.list.clone(), move |i, _, _| {
            let range = chunks.get(i).cloned().unwrap_or(0..0);
            let piece = text[range].trim_end_matches('\n').to_owned();
            div()
                .px(px(8.))
                .min_h(px(17.))
                .font_family("Menlo")
                .text_size(px(12.))
                .child(
                    SelectableText::with_handle(
                        ElementId::Name(format!("{id}-text-{i}").into()),
                        handle.clone(),
                        piece,
                    )
                    .document_order(i as u64),
                )
                .into_any_element()
        })
        .size_full()
    }

    fn hex_element(&self, id: &'static str, t: &Tokens) -> impl IntoElement + use<> {
        let bytes = self.bytes.clone();
        let rows = bytes.len().div_ceil(HEX_WIDTH);
        let (muted, fg) = (t.muted_foreground.hsla(), t.foreground.hsla());
        uniform_list(SharedString::from(format!("{id}-hex")), rows, move |range, _, _| {
            range
                .map(|r| {
                    let row = hex_row(&bytes, r);
                    div()
                        .id(("hex-row", r))
                        .h(px(18.))
                        .px(px(8.))
                        .flex()
                        .gap(px(16.))
                        .font_family("Menlo")
                        .text_size(px(12.))
                        .child(div().flex_none().text_color(muted).child(row.offset))
                        .child(div().flex_none().w(px(7.3 * 49.)).text_color(fg).child(row.hex))
                        .child(div().flex_none().text_color(muted).child(row.ascii))
                })
                .collect()
        })
        .track_scroll(&self.hex)
        .size_full()
    }
}

/// The text cut into lines, and long lines into pieces of at most
/// [`CHUNK`] bytes (on character boundaries).
fn chunks(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.len();
        let mut piece = start;
        while end - piece > CHUNK {
            let mut cut = piece + CHUNK;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            out.push(piece..cut);
            piece = cut;
        }
        out.push(piece..end);
        start = end;
    }
    if out.is_empty() {
        out.push(0..0);
    }
    out
}

/// A centred line of muted text filling the pane ("Select a message").
pub fn centered(text: impl Into<SharedString>, t: &Tokens) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(px(16.))
        .text_size(px(12.))
        .text_color(t.muted_foreground.hsla())
        .child(text.into())
        .into_any_element()
}

/// A toggle chip (the filter bar's look): a button that says whether it
/// is on, with an icon before its label.
pub fn chip(
    id: SharedString,
    label: &'static str,
    on: bool,
    icon: Option<gpui_kit::Svg>,
    t: &Tokens,
    click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static,
) -> AnyElement {
    use gpui_kit::base::TestSupportExt as _;
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .h(px(20.))
        .px(px(8.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .rounded(px(3.))
        .text_size(px(12.))
        .map(|d| {
            if on {
                d.bg(t.primary_chip_bg.hsla()).text_color(t.primary_chip_fg.hsla())
            } else {
                d.text_color(t.muted_foreground.hsla()).hover(|s| s.bg(t.muted.hsla()))
            }
        })
        .children(icon)
        .child(label)
        .on_click(click)
        .test_support()
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_frames_open_as_hex_unless_they_hold_json() {
        assert_eq!(sniff(b"hello", false), ("text/plain", Viewer::Text));
        assert_eq!(sniff(b"hello", true), ("text/plain", Viewer::Hex));
        assert_eq!(sniff(b" {\"a\":1}", true), ("application/json", Viewer::Text));
        assert_eq!(sniff(&[0, 1, 2], false), ("application/octet-stream", Viewer::Hex));
        assert_eq!(Viewer::parse("hex"), Some(Viewer::Hex));
    }

    #[test]
    fn long_lines_are_cut_on_character_boundaries() {
        let text = format!("a\n{}", "é".repeat(CHUNK));
        let parts = chunks(&text);
        let joined: String = parts.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(joined, text);
        assert!(parts.iter().all(|r| r.len() <= CHUNK));
        assert_eq!(chunks(""), vec![0..0usize; 1]);
    }
}
