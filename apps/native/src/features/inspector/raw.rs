//! The Raw tab (`Inspector/RawView.tsx`): the message rebuilt from its
//! parsed parts (not the exact wire bytes), its body fetched only when it
//! is plain, finished and at most 1 MB (`pm_view::inspector::raw_body`).
//!
//! The text is drawn as a virtualised list of wrapped, selectable chunks
//! (lines, long ones cut every 2 KB) that share one selection document, so
//! a 1 MB body costs only the rows on screen. G4's CodeView takes it over,
//! with highlighting.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::{SelectableText, TextSelectionHandle};
use gpui_kit::prelude::*;
use gpui_kit::{App, ElementId, ListAlignment, ListState, SharedString, div, list, px};
use pm_model::{BodyPart, Header};

/// A chunk of a long line is at most this many bytes.
const CHUNK: usize = 2048;

/// One rebuilt message and its list.
pub struct RawDoc {
    pub start_line: String,
    pub headers: Vec<Header>,
    pub text: SharedString,
    chunks: Rc<Vec<Range<usize>>>,
    list: ListState,
    handle: TextSelectionHandle,
    /// What it was built from: the inspected flow and the body, by address.
    source: (usize, usize),
}

#[derive(Default)]
pub struct RawDocs {
    request: Option<RawDoc>,
    response: Option<RawDoc>,
}

impl RawDocs {
    pub fn get(&self, part: BodyPart) -> Option<&RawDoc> {
        match part {
            BodyPart::Request => self.request.as_ref(),
            BodyPart::Response => self.response.as_ref(),
        }
    }

    /// The doc for `part`, rebuilt when its sources changed.
    pub fn update(
        &mut self,
        part: BodyPart,
        source: (usize, usize),
        build: impl FnOnce() -> (String, Vec<Header>, String),
        cx: &mut App,
    ) -> &RawDoc {
        let slot = match part {
            BodyPart::Request => &mut self.request,
            BodyPart::Response => &mut self.response,
        };
        if slot.as_ref().is_none_or(|d| d.source != source) {
            let (start_line, headers, text) = build();
            let chunks = Rc::new(chunks(&text));
            let handle = TextSelectionHandle::new(text.clone(), cx);
            *slot = Some(RawDoc {
                start_line,
                headers,
                list: ListState::new(chunks.len(), ListAlignment::Top, px(400.)),
                chunks,
                text: text.into(),
                handle,
                source,
            });
        }
        slot.as_ref().expect("just set")
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

impl RawDoc {
    /// The list of chunks, each a run of the shared selection document.
    pub fn element(&self, id: &'static str) -> impl IntoElement + use<> {
        let text = self.text.clone();
        let chunks = self.chunks.clone();
        let handle = self.handle.clone();
        list(self.list.clone(), move |i, _, _| {
            let range = chunks.get(i).cloned().unwrap_or(0..0);
            // Drop the line's own newline: the next row starts the next line.
            let piece = text[range].trim_end_matches('\n').to_owned();
            div()
                .px(px(8.))
                // An empty line keeps its height.
                .min_h(px(17.))
                .child(
                    SelectableText::with_handle(ElementId::Name(format!("{id}-{i}").into()), handle.clone(), piece)
                        .document_order(i as u64),
                )
                .into_any_element()
        })
        .size_full()
    }
}

/// A shared value's address: what a [`RawDoc`] was built from.
pub fn address<T>(value: &Arc<T>) -> usize {
    Arc::as_ptr(value) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_are_cut_on_character_boundaries() {
        let text = format!("GET / HTTP/1.1\nHost: a\n\n{}", "é".repeat(CHUNK));
        let parts = chunks(&text);
        assert_eq!(&text[parts[0].clone()], "GET / HTTP/1.1\n");
        assert_eq!(&text[parts[2].clone()], "\n");
        let joined: String = parts.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(joined, text);
        assert!(parts.iter().all(|r| r.len() <= CHUNK));
        assert_eq!(parts.len(), 5, "the body's 4 KB line is two pieces");
        assert_eq!(chunks(""), vec![0..0usize; 1]);
    }
}
