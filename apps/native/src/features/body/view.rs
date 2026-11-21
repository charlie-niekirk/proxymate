//! What a viewer shows, in two steps: [`prepare`] turns the body's bytes
//! into what the viewer needs (formatted text, a parsed tree, form fields,
//! parts, operations) on the background executor, then the pane builds the
//! widgets for it on the foreground ([`Surface`] for text: the gpui-kit
//! editor up to the 2 MB limit, the CodeView above it).

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, SharedString, Window};
use pm_view::body::graphql::{self, Operation};
use pm_view::body::highlight::Lang;
use pm_view::body::json_doc::{self, JNode};
use pm_view::body::multipart::{self, Part};
use pm_view::body::registry::ViewerId;
use pm_view::body::{FORMAT_MAX, HIGHLIGHT_LIMIT, form_fields, json_format, looks_minified, markup, text_of};
use pm_widgets::code_editor::{self, CodeEditor};
use pm_widgets::code_view::{CodeView, Highlight};

/// The message shown for a binary body in a text viewer.
pub const BINARY: &str = "Binary body: switch to Hex to see its bytes.";

/// A viewer's input, ready to show.
pub enum Prepared {
    /// Text to show, with a warning note above it (`Not valid JSON: …`).
    Text {
        text: SharedString,
        note: Option<SharedString>,
    },
    Binary,
    Tree(Arc<JNode>),
    /// JS or CSS, and whether it looks minified (it starts beautified then).
    Script { text: SharedString, minified: bool },
    Form {
        text: SharedString,
        fields: Vec<(String, String)>,
    },
    Multipart {
        boundary: Option<String>,
        parts: Vec<Part>,
    },
    /// The operations, or the text when the body isn't GraphQL.
    Graphql(Result<Vec<Operation>, SharedString>),
}

/// What `viewer` needs from `bytes` (run on the background executor).
pub fn prepare(viewer: ViewerId, bytes: &[u8], content_type: Option<&str>) -> Prepared {
    if viewer == ViewerId::Multipart {
        let boundary = multipart::boundary_of(content_type);
        let parts = boundary
            .as_deref()
            .map(|b| multipart::parse_multipart(bytes, b))
            .unwrap_or_default();
        return Prepared::Multipart { boundary, parts };
    }
    let Some(text) = text_of(bytes) else {
        return Prepared::Binary;
    };
    match viewer {
        ViewerId::Json => match json_format::format_json(text, "  ") {
            Some(pretty) => Prepared::Text {
                text: pretty.into(),
                note: None,
            },
            None => Prepared::Text {
                text: text.to_owned().into(),
                note: Some(match json_doc::parse(text) {
                    Err(e) => format!("Not valid JSON: {e}. Showing the text.").into(),
                    Ok(_) => "Not a JSON object or array. Showing the text.".into(),
                }),
            },
        },
        ViewerId::JsonTree => match json_doc::parse(text) {
            Ok(doc) => Prepared::Tree(Arc::new(doc)),
            Err(e) => Prepared::Text {
                text: text.to_owned().into(),
                note: Some(format!("Not valid JSON: {e}. Showing the text.").into()),
            },
        },
        ViewerId::Xml | ViewerId::Html => Prepared::Text {
            text: if text.len() <= FORMAT_MAX {
                markup::format_markup(text, viewer == ViewerId::Html, "  ").into()
            } else {
                text.to_owned().into()
            },
            note: None,
        },
        ViewerId::Javascript | ViewerId::Css => Prepared::Script {
            minified: looks_minified(text),
            text: text.to_owned().into(),
        },
        ViewerId::Form => Prepared::Form {
            fields: form_fields(text),
            text: text.to_owned().into(),
        },
        ViewerId::Graphql => Prepared::Graphql(graphql::parse_body(text, content_type).ok_or_else(|| text.to_owned().into())),
        _ => Prepared::Text {
            text: text.to_owned().into(),
            note: None,
        },
    }
}

/// The editor's language and the CodeView's lexer for a viewer.
pub fn languages(viewer: ViewerId) -> (&'static str, Lang) {
    let lang = match viewer {
        ViewerId::Json | ViewerId::JsonTree => Lang::Json,
        ViewerId::Xml | ViewerId::Html | ViewerId::HtmlPreview => Lang::Markup,
        ViewerId::Javascript => Lang::Javascript,
        ViewerId::Css => Lang::Css,
        ViewerId::Graphql => Lang::Graphql,
        _ => Lang::Plain,
    };
    (code_editor::language_for(viewer), lang)
}

/// The lexer for a body of `kind` (the Raw tab's body).
pub fn lang_for_kind(kind: pm_model::ContentKind) -> Lang {
    use pm_model::ContentKind as K;
    match kind {
        K::Json | K::GraphQl => Lang::Json,
        K::Xml | K::Html | K::Svg => Lang::Markup,
        K::Javascript => Lang::Javascript,
        K::Css => Lang::Css,
        _ => Lang::Plain,
    }
}

/// How a text is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceOpts {
    /// The gpui-kit editor (up to the 2 MB limit), else the CodeView.
    pub editor: bool,
    /// The CodeView's own highlighting ("Highlight anyway").
    pub highlight: bool,
    pub wrap: bool,
}

impl SurfaceOpts {
    /// Text of a body of `size` bytes, highlighted anyway or not.
    pub fn for_size(size: usize, highlight_anyway: bool, wrap: bool) -> SurfaceOpts {
        SurfaceOpts {
            editor: size <= HIGHLIGHT_LIMIT,
            highlight: highlight_anyway,
            wrap,
        }
    }
}

pub type CodeMenu = std::rc::Rc<dyn Fn(Option<String>, SharedString, gpui_kit::Point<gpui_kit::Pixels>, &mut Window, &mut App)>;

/// A text widget: the editor or the CodeView.
#[derive(Clone)]
pub enum Surface {
    Editor(Entity<CodeEditor>),
    View(Entity<CodeView>),
}

impl Surface {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: &str,
        label: &str,
        text: SharedString,
        viewer: ViewerId,
        opts: SurfaceOpts,
        menu: CodeMenu,
        window: &mut Window,
        cx: &mut App,
    ) -> Surface {
        let (language, lang) = languages(viewer);
        if opts.editor {
            let (id, label) = (id.to_owned(), label.to_owned());
            let editor = cx.new(|cx| {
                let mut e = CodeEditor::new(id, label, language, true, window, cx).on_menu(move |req, window, cx| {
                    menu(req.selection, req.text, req.position, window, cx)
                });
                e.set_wrap(opts.wrap, window, cx);
                e.set_text(text, window, cx);
                e
            });
            Surface::Editor(editor)
        } else {
            let (id, label) = (id.to_owned(), label.to_owned());
            let view = cx.new(|cx| {
                CodeView::new(id, label, text, cx)
                    .highlight(opts.highlight.then_some(Highlight::Lang(lang)))
                    .wrap(opts.wrap, cx)
                    .on_menu(move |req, window, cx| menu(req.selection, req.text, req.position, window, cx))
            });
            Surface::View(view)
        }
    }

    pub fn set_terms(&self, terms: &[String], cx: &mut App) {
        let terms = terms.to_vec();
        match self {
            Surface::Editor(e) => e.update(cx, |e, cx| e.set_terms(terms, cx)),
            Surface::View(v) => v.update(cx, |v, cx| v.set_terms(terms, cx)),
        }
    }

    pub fn set_text(&self, text: SharedString, window: &mut Window, cx: &mut App) {
        match self {
            Surface::Editor(e) => e.update(cx, |e, cx| e.set_text(text, window, cx)),
            Surface::View(v) => v.update(cx, |v, cx| v.set_text(text, cx)),
        }
    }

    pub fn set_wrap(&self, wrap: bool, window: &mut Window, cx: &mut App) {
        match self {
            Surface::Editor(e) => e.update(cx, |e, cx| e.set_wrap(wrap, window, cx)),
            Surface::View(v) => v.update(cx, |v, cx| v.set_wrap(wrap, cx)),
        }
    }

    pub fn select_all(&self, window: &mut Window, cx: &mut App) {
        match self {
            Surface::Editor(e) => e.update(cx, |e, cx| e.select_all(window, cx)),
            Surface::View(v) => v.update(cx, |v, cx| v.select_all(cx)),
        }
    }

    pub fn open_find(&self, window: &mut Window, cx: &mut App) {
        match self {
            Surface::Editor(e) => e.update(cx, |e, cx| e.open_find(window, cx)),
            Surface::View(v) => v.update(cx, |v, cx| v.open_find(window, cx)),
        }
    }

    pub fn text(&self, cx: &App) -> SharedString {
        match self {
            Surface::Editor(e) => e.read(cx).text().clone(),
            Surface::View(v) => v.read(cx).text().clone(),
        }
    }

    /// Matches of the body terms, and the one stepped to.
    pub fn matches(&self, cx: &App) -> (usize, Option<usize>) {
        match self {
            Surface::Editor(e) => (e.read(cx).match_count(), e.read(cx).current_match()),
            Surface::View(v) => (v.read(cx).match_count(), v.read(cx).current_match()),
        }
    }

    pub fn is_editor(&self) -> bool {
        matches!(self, Surface::Editor(_))
    }

    pub fn element(&self) -> AnyElement {
        match self {
            Surface::Editor(e) => e.clone().into_any_element(),
            Surface::View(v) => v.clone().into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(p: Prepared) -> (String, Option<String>) {
        match p {
            Prepared::Text { text, note } => (text.to_string(), note.map(|n| n.to_string())),
            _ => panic!("not text"),
        }
    }

    #[test]
    fn prepares_each_viewer() {
        assert_eq!(
            text(prepare(ViewerId::Json, br#"{"a":1.50}"#, None)),
            ("{\n  \"a\": 1.50\n}".into(), None)
        );
        let (shown, note) = text(prepare(ViewerId::Json, b"nope", None));
        assert_eq!(shown, "nope");
        assert!(note.unwrap().starts_with("Not valid JSON: "));
        let (_, note) = text(prepare(ViewerId::JsonTree, b"{\"a\":", None));
        assert!(note.unwrap().contains("Showing the text"));
        assert!(matches!(prepare(ViewerId::JsonTree, b"[1]", None), Prepared::Tree(_)));
        assert_eq!(text(prepare(ViewerId::Xml, b"<a><b/></a>", None)).0, "<a>\n  <b/>\n</a>");
        assert!(matches!(prepare(ViewerId::Text, b"a\0b", None), Prepared::Binary));
        assert!(matches!(
            prepare(ViewerId::Javascript, "x".repeat(1000).as_bytes(), None),
            Prepared::Script { minified: true, .. }
        ));
        match prepare(ViewerId::Form, b"a=1&b=2", None) {
            Prepared::Form { fields, .. } => assert_eq!(fields.len(), 2),
            _ => panic!("form"),
        }
        match prepare(ViewerId::Multipart, b"--x\r\n\r\nhi\r\n--x--", Some("multipart/form-data; boundary=x")) {
            Prepared::Multipart { boundary, parts } => {
                assert_eq!(boundary.as_deref(), Some("x"));
                assert_eq!(parts.len(), 1);
            }
            _ => panic!("multipart"),
        }
        assert!(matches!(prepare(ViewerId::Graphql, b"{\"q\":1}", None), Prepared::Graphql(Err(_))));
    }
}
