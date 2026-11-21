//! The Raw tab (`Inspector/RawView.tsx`): the message rebuilt from its
//! parsed parts (not the exact wire bytes), its body fetched only when it
//! is plain, finished and at most 1 MB (`pm_view::inspector::raw_body`).
//!
//! Since G4 the text is a [`CodeView`]: wrapped, the head highlighted as
//! HTTP and the body by its kind, with Select all, Find (⌘F) and the body
//! terms marked; its menu is the code menu with the start line and headers
//! (`body::menus::raw`).

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Entity, SharedString};
use pm_model::{BodyPart, ContentKind, Header};
use pm_widgets::code_view::{CodeView, Highlight};

use crate::features::body;

/// One rebuilt message and its view.
pub struct RawDoc {
    pub start_line: String,
    pub headers: Vec<Header>,
    pub text: SharedString,
    pub view: Entity<CodeView>,
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
        kind: ContentKind,
        build: impl FnOnce() -> (String, Vec<Header>, String),
        cx: &mut App,
    ) -> &RawDoc {
        let slot = match part {
            BodyPart::Request => &mut self.request,
            BodyPart::Response => &mut self.response,
        };
        if slot.as_ref().is_none_or(|d| d.source != source) {
            let (start_line, headers, text) = build();
            let text: SharedString = text.into();
            let highlight = Highlight::Http {
                // The start line, the headers and the blank line.
                head_lines: headers.len() + 2,
                body: body::lang_for_kind(kind),
            };
            let (id, label) = match part {
                BodyPart::Request => ("raw-request", "Raw request"),
                BodyPart::Response => ("raw-response", "Raw response"),
            };
            let (line, menu_headers) = (start_line.clone(), headers.clone());
            // The same view across flows keeps its find bar and scroll handle.
            let view = match slot.take() {
                Some(doc) => {
                    doc.view.update(cx, |v, cx| {
                        v.set_text(text.clone(), cx);
                        v.set_highlight(Some(highlight), cx);
                    });
                    doc.view
                }
                None => cx.new(|cx| {
                    CodeView::new(id, label, text.clone(), cx)
                        .highlight(Some(highlight))
                        .wrap(true, cx)
                }),
            };
            view.update(cx, |v, _| {
                v.set_on_menu(move |req, window, cx| {
                    body::menus::raw(part, &line, &menu_headers, req.text, req.selection).show(req.position, window, cx)
                });
            });
            *slot = Some(RawDoc {
                start_line,
                headers,
                text,
                view,
                source,
            });
        }
        slot.as_ref().expect("just set")
    }
}

/// A shared value's address: what a [`RawDoc`] was built from.
pub fn address<T>(value: &Arc<T>) -> usize {
    Arc::as_ptr(value) as usize
}
