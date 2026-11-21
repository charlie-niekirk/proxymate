//! The Body tab (G4; `BodyViewer/BodyViewer.tsx`): picks the best viewer
//! for the body's kind (the dropdown overrides it, and the choice sticks
//! per kind for the session), loads the decoded body through the
//! `decoded` cache, and keeps huge bodies cheap: only the first 20 MB load
//! until "Load all", and over 2 MB text goes to the CodeView, highlighted
//! only on "Highlight anyway".
//!
//! Text viewers: JSON (re-indented without parsing values), JSON Tree,
//! XML and HTML (re-indented), JavaScript and CSS (Beautify), Text (Wrap),
//! Form, Multipart (a sub-viewer per part), GraphQL and Hex. Media and
//! decoded viewers are G5's; until then their kinds offer Hex and the
//! others in the dropdown.
//!
//! Formatting, parsing and beautifying run on the background executor;
//! the filter's body terms are marked in whichever viewer shows. Every
//! viewer's menu ends with the body items ([`menus::body_items`]).

pub mod menus;
mod view;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::{SelectableText, TestSupportExt as _};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Selectable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, FontWeight, Global, MouseButton, MouseDownEvent, Role,
    SharedString, Task, Window, div, px,
};
use pm_model::{BodyInfo, BodyPart, ContentKind, FlowId, Header, StoreId};
use pm_theme::Tokens;
use pm_theme::kit::TokensExt as _;
use pm_view::body::content::{essence, fallback_kind, kind_from_content_type, sniff};
use pm_view::body::graphql::{self, Operation};
use pm_view::body::multipart::Part;
use pm_view::body::registry::{Source, ViewerId, pick_viewer, viewers_for};
use pm_view::body::{HIGHLIGHT_LIMIT, LOAD_LIMIT, beautify, looks_binary, suggested_file_name, text_of};
use pm_view::flow::format_size;
use pm_view::http::{display_header_name, header_value};
use pm_view::inspector::KvKind;
use pm_widgets::hex_view::HexView;
use pm_widgets::json_tree::JsonTree;
use pm_widgets::{KvTable, code_editor, code_view};

use crate::bridge::keys::{self, Caches, DecodedBody, DecodedKey, FlowKey};
use crate::bridge::resource::Load;
use crate::features::clipboard::{copy_later, report_error};
use crate::features::context_menu::Menu;
use crate::features::inspector::actions::FlowRef;
pub use menus::BodyRef;
use view::{BINARY, Prepared, SurfaceOpts, prepare};
pub use view::{Surface, lang_for_kind};

/// Copies the shown body as text (or Base64).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct CopyBody {
    pub flow: FlowRef,
    pub part: BodyPart,
    pub base64: bool,
}

/// Asks where to save a body, decoded, and writes it there.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SaveBody {
    pub flow: FlowRef,
    pub part: BodyPart,
    pub file_name: SharedString,
}

/// Picks a viewer for bodies of a kind.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ChooseViewer {
    pub kind: ContentKind,
    pub viewer: ViewerId,
}

gpui_kit::actions!(
    proxymate,
    [
        /// The code menu's "Select all" (⌘A in the viewer).
        SelectAllInBody,
        /// The code menu's "Find…" (⌘F in the viewer).
        FindInBody
    ]
);

/// The viewer chosen per content kind (the Tauri UI's `viewerByKind`, for
/// the session).
#[derive(Default)]
pub struct Choices {
    by_kind: HashMap<ContentKind, ViewerId>,
    /// "Couldn't save: …" per body.
    save_errors: HashMap<(StoreId, FlowId, BodyPart), SharedString>,
}

impl Global for Choices {}

pub fn choice(kind: ContentKind, cx: &App) -> Option<ViewerId> {
    cx.try_global::<Choices>().and_then(|c| c.by_kind.get(&kind).copied())
}

/// Picks `viewer` for bodies of `kind` (the dropdown, the dev remote).
pub fn choose(kind: ContentKind, viewer: ViewerId, cx: &mut App) {
    cx.default_global::<Choices>().by_kind.insert(kind, viewer);
    cx.refresh_windows();
}

pub fn register(cx: &mut App) {
    code_view::init(cx);
    code_editor::init(cx);
    cx.default_global::<Choices>();
    cx.on_action(|a: &ChooseViewer, cx| choose(a.kind, a.viewer, cx));
    cx.on_action(|a: &CopyBody, cx| copy_body(a.flow, a.part, a.base64, cx));
    cx.on_action(|a: &SaveBody, cx| save_body(a.flow, a.part, a.file_name.to_string(), cx));
}

/// The bytes the Body tab has for a body, if any are loaded.
fn loaded(flow: FlowRef, part: BodyPart, cx: &mut App) -> Option<Arc<Vec<u8>>> {
    let caches = keys::caches(cx);
    let key = FlowKey::new(flow.store, flow.id);
    let decoded = caches.decoded.read(cx);
    [None, Some(LOAD_LIMIT)].into_iter().find_map(|max| {
        decoded
            .peek(&DecodedKey { flow: key, part, max })
            .map(|b| b.bytes.clone())
    })
}

fn engine(cx: &App) -> Option<Arc<pm_core::Engine>> {
    crate::workspace::Workspace::global(cx).and_then(|w| w.read(cx).engine().cloned())
}

fn copy_body(flow: FlowRef, part: BodyPart, base64: bool, cx: &mut App) {
    let bytes = loaded(flow, part, cx);
    let engine = engine(cx);
    let what = if base64 { "body as Base64" } else { "body" };
    copy_later(
        move || {
            let bytes = match bytes {
                Some(b) => b,
                None => {
                    let engine = engine.ok_or("No body loaded")?;
                    Arc::new(
                        engine
                            .body_head_in(flow.store, flow.id, part, true, None)
                            .map_err(|e| e.to_string())?
                            .bytes,
                    )
                }
            };
            if base64 {
                return Ok(Some(pm_view::copy_formats::base64(&bytes)));
            }
            match text_of(&bytes) {
                Some(text) => Ok(Some(text.to_owned())),
                None => Err("The body isn't UTF-8 text: copy it as Base64 instead".into()),
            }
        },
        what,
        cx,
    );
}

fn save_body(flow: FlowRef, part: BodyPart, file_name: String, cx: &mut App) {
    let Some(engine) = engine(cx) else {
        return;
    };
    cx.default_global::<Choices>()
        .save_errors
        .remove(&(flow.store, flow.id, part));
    let directory = std::env::var_os("HOME").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let picked = cx.prompt_for_new_path(&directory, Some(&file_name));
    cx.spawn(async move |cx| {
        let path = match picked.await {
            Ok(Ok(Some(path))) => path,
            Ok(Err(err)) => {
                cx.update(|cx| report_error("Couldn't show the save panel", err, cx));
                return;
            }
            _ => return,
        };
        let saved = cx
            .background_spawn(async move { engine.save_body_in(flow.store, flow.id, part, true, &path) })
            .await;
        if let Err(err) = saved {
            cx.update(|cx| {
                cx.default_global::<Choices>()
                    .save_errors
                    .insert((flow.store, flow.id, part), err.to_string().into());
                report_error("Couldn't save the body", &err, cx);
                cx.refresh_windows();
            });
        }
    })
    .detach();
}

/// What the pane shows a body of.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyInput {
    pub key: FlowKey,
    pub part: BodyPart,
    pub info: BodyInfo,
    pub finished: bool,
    pub headers: Vec<Header>,
    /// The request URL: file names, and GET GraphQL operations.
    pub url: String,
    /// The filter's body terms for this side.
    pub terms: Vec<String>,
}

impl BodyInput {
    pub fn content_type(&self) -> Option<&str> {
        header_value(&self.headers, "content-type")
    }

    /// The body's kind: the engine's, else the header's for a body recorded
    /// before detection or still streaming (a trailers-only gRPC response
    /// has a status to show without a body).
    pub fn kind(&self) -> ContentKind {
        let ct = self.content_type();
        let grpc = kind_from_content_type(ct) == ContentKind::Grpc && self.part == BodyPart::Response;
        match self.info.kind {
            ContentKind::None if self.info.size > 0 || grpc => fallback_kind(ct),
            kind => kind,
        }
    }

    pub fn mime(&self) -> String {
        self.info
            .mime
            .clone()
            .unwrap_or_else(|| match essence(self.content_type()) {
                m if m.is_empty() => "application/octet-stream".into(),
                m => m,
            })
    }

    fn flow(&self) -> FlowRef {
        FlowRef {
            store: self.key.store,
            id: self.key.id,
        }
    }
}

enum CodeBar {
    None,
    Wrap(bool),
    Beautify {
        on: bool,
        original: SharedString,
        state: Beautified,
    },
}

enum Beautified {
    Idle,
    Running(#[allow(dead_code)] Task<()>),
    Done(SharedString),
    Failed(SharedString),
}

struct PartView {
    part: Part,
    mime: String,
    headers: Rc<Vec<(SharedString, SharedString)>>,
    body: PartBody,
}

enum PartBody {
    None,
    Inline(SharedString),
    Text(Surface),
    Hex(Entity<HexView>),
    Message(SharedString),
}

enum Shown {
    Message(SharedString),
    Code {
        surface: Surface,
        bar: CodeBar,
        note: Option<SharedString>,
    },
    Tree(Entity<JsonTree>),
    Hex(Entity<HexView>),
    Form {
        fields: Rc<Vec<(String, String)>>,
        raw: Surface,
        show_raw: bool,
    },
    Multipart {
        boundary: Option<String>,
        parts: Vec<PartView>,
        /// Without a boundary: the bytes.
        hex: Option<Entity<HexView>>,
    },
    Graphql {
        ops: Rc<Vec<Operation>>,
        index: usize,
        query: Option<Surface>,
        variables: Option<Surface>,
        extensions: Option<Surface>,
    },
}

/// What a [`Shown`] was built from: the body (by address), the viewer, and
/// whether a big text is highlighted anyway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Source_ {
    body: usize,
    viewer: ViewerId,
    highlight_anyway: bool,
}

pub struct BodyPane {
    part: BodyPart,
    caches: Caches,
    input: Option<BodyInput>,
    load_all: bool,
    highlight_anyway: bool,
    source: Option<Source_>,
    shown: Option<Shown>,
    preparing: Option<Task<()>>,
}

impl BodyPane {
    pub fn new(part: BodyPart, cx: &mut Context<Self>) -> Self {
        let caches = keys::caches(cx);
        cx.observe(&caches.decoded, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Choices>(|_, cx| cx.notify()).detach();
        BodyPane {
            part,
            caches,
            input: None,
            load_all: false,
            highlight_anyway: false,
            source: None,
            shown: None,
            preparing: None,
        }
    }

    /// The body to show; a new flow starts with the 20 MB cap and no
    /// highlighting of big texts.
    pub fn set_input(&mut self, input: BodyInput, cx: &mut Context<Self>) {
        if self.input.as_ref().is_some_and(|i| i == &input) {
            return;
        }
        if self.input.as_ref().is_none_or(|i| i.key != input.key) {
            self.load_all = false;
            self.highlight_anyway = false;
        }
        self.input = Some(input);
        cx.notify();
    }

    /// The viewer showing, for the body's kind.
    pub fn viewer(&self, cx: &App) -> Option<(ContentKind, ViewerId)> {
        let input = self.input.as_ref()?;
        let kind = input.kind();
        Some((kind, pick_viewer(kind, choice(kind, cx))))
    }

    pub fn load_all(&mut self, cx: &mut Context<Self>) {
        self.load_all = true;
        cx.notify();
    }

    pub fn highlight_anyway(&mut self, cx: &mut Context<Self>) {
        self.highlight_anyway = true;
        cx.notify();
    }

    /// The text surface showing (code viewers, Form's raw text), if any.
    pub fn surface(&self) -> Option<&Surface> {
        match self.shown.as_ref()? {
            Shown::Code { surface, .. } => Some(surface),
            Shown::Form {
                raw, show_raw: true, ..
            } => Some(raw),
            Shown::Graphql { query, .. } => query.as_ref(),
            _ => None,
        }
    }

    pub fn tree(&self) -> Option<&Entity<JsonTree>> {
        match self.shown.as_ref()? {
            Shown::Tree(t) => Some(t),
            _ => None,
        }
    }

    pub fn hex(&self) -> Option<&Entity<HexView>> {
        match self.shown.as_ref()? {
            Shown::Hex(h) => Some(h),
            _ => None,
        }
    }

    /// Whether a viewer is being prepared (formatting, parsing).
    pub fn is_preparing(&self) -> bool {
        self.preparing.is_some()
    }

    /// What the pane shows, as JSON (the dev remote's `body`).
    pub fn describe(&self, cx: &App) -> serde_json::Value {
        let Some(input) = &self.input else {
            return serde_json::Value::Null;
        };
        let kind = input.kind();
        let viewer = pick_viewer(kind, choice(kind, cx));
        let (shown, detail) = match &self.shown {
            None => ("none", serde_json::Value::Null),
            Some(Shown::Message(m)) => ("message", serde_json::json!(m.to_string())),
            Some(Shown::Code { surface, note, .. }) => {
                let (matches, current) = surface.matches(cx);
                (
                    if surface.is_editor() {
                        "code-editor"
                    } else {
                        "code-view"
                    },
                    serde_json::json!({
                        "chars": surface.text(cx).len(),
                        "note": note.as_ref().map(|n| n.to_string()),
                        "matches": matches,
                        "current": current,
                    }),
                )
            }
            Some(Shown::Tree(tree)) => {
                let t = tree.read(cx);
                (
                    "json-tree",
                    serde_json::json!({ "rows": t.rows().len(), "hits": t.hit_count(), "current": t.current_hit() }),
                )
            }
            Some(Shown::Hex(hex)) => ("hex", serde_json::json!({ "rows": hex.read(cx).row_count() })),
            Some(Shown::Form { fields, show_raw, .. }) => {
                ("form", serde_json::json!({ "fields": fields.len(), "raw": show_raw }))
            }
            Some(Shown::Multipart { parts, boundary, .. }) => (
                "multipart",
                serde_json::json!({ "parts": parts.len(), "boundary": boundary }),
            ),
            Some(Shown::Graphql { ops, index, .. }) => (
                "graphql",
                serde_json::json!({
                    "operations": ops.iter().map(|o| o.operation_name.clone()).collect::<Vec<_>>(),
                    "index": index,
                }),
            ),
        };
        serde_json::json!({
            "kind": kind,
            "viewer": viewer.id(),
            "viewers": viewers_for(kind).iter().map(|v| v.id()).collect::<Vec<_>>(),
            "size": input.info.size,
            "finished": input.finished,
            "loadAll": self.load_all,
            "highlightAnyway": self.highlight_anyway,
            "preparing": self.preparing.is_some(),
            "shown": shown,
            "detail": detail,
        })
    }

    /// The menu a right-click on the viewer opens (the dev remote's
    /// `contextMenu: "body:<part>"`): the code menu for text, else the body
    /// menu.
    pub fn menu(&self, selected: Option<String>, cx: &App) -> Menu {
        let bytes = self.form_bytes(cx);
        let body = self.body_ref(bytes.as_deref().map(|b| b.as_slice()));
        match self.surface() {
            Some(surface) => menus::code(selected, surface.text(cx), Menu::new(), body.as_ref()),
            None => menus::fallback(selected, body.as_ref()),
        }
    }

    /// What the pane's menus end with.
    fn body_ref(&self, bytes: Option<&[u8]>) -> Option<BodyRef> {
        let input = self.input.as_ref()?;
        let bytes = bytes?;
        Some(BodyRef {
            flow: input.flow(),
            part: input.part,
            binary: looks_binary(bytes),
            empty: bytes.is_empty(),
            file_name: suggested_file_name(&input.url, &input.mime(), input.part).into(),
        })
    }

    /// The body loaded (the first 20 MB unless "Load all").
    fn load(&mut self, input: &BodyInput, cx: &mut Context<Self>) -> (Load<Arc<DecodedBody>>, bool) {
        let capped = input.info.captured > LOAD_LIMIT && !self.load_all;
        let key = DecodedKey {
            flow: input.key,
            part: input.part,
            max: capped.then_some(LOAD_LIMIT),
        };
        (self.caches.decoded.update(cx, |c, cx| c.get(&key, cx)), capped)
    }

    /// Builds the viewer for `body` unless it is already built.
    fn ensure_shown(
        &mut self,
        body: &Arc<DecodedBody>,
        viewer: ViewerId,
        input: &BodyInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = Source_ {
            body: Arc::as_ptr(body) as usize,
            viewer,
            highlight_anyway: self.highlight_anyway,
        };
        if self.source == Some(source) {
            return;
        }
        self.source = Some(source);
        self.shown = None;
        let bytes = body.bytes.clone();
        let body_ref = self.body_ref(Some(&bytes));
        if viewer == ViewerId::Hex {
            self.shown = Some(Shown::Hex(hex_view(self.part, bytes, body_ref, cx)));
            self.preparing = None;
            return;
        }
        let content_type = input.content_type().map(str::to_owned);
        let task = cx.background_spawn(async move { prepare(viewer, &bytes, content_type.as_deref()) });
        let size = body.bytes.len();
        self.preparing = Some(cx.spawn_in(window, async move |this, cx| {
            let prepared = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.source != Some(source) {
                    return;
                }
                this.preparing = None;
                let shown = this.build(prepared, viewer, size, window, cx);
                this.shown = Some(shown);
                cx.notify();
            });
        }));
    }

    fn code_menu(&self, bytes: Option<&[u8]>) -> view::CodeMenu {
        let body = self.body_ref(bytes);
        Rc::new(move |selection, text, position, window, cx| {
            menus::code(selection, text, Menu::new(), body.as_ref()).show(position, window, cx)
        })
    }

    fn id(&self, what: &str) -> String {
        let part = match self.part {
            BodyPart::Request => "request",
            BodyPart::Response => "response",
        };
        format!("{part}-body-{what}")
    }

    fn label(&self) -> &'static str {
        match self.part {
            BodyPart::Request => "Request body",
            BodyPart::Response => "Response body",
        }
    }

    fn build(
        &mut self,
        prepared: Prepared,
        viewer: ViewerId,
        size: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Shown {
        let bytes = self.current_bytes(cx);
        let menu = self.code_menu(bytes.as_deref().map(|b| b.as_slice()));
        let opts = SurfaceOpts::for_size(size, self.highlight_anyway, false);
        let (id, label) = (self.id("code"), self.label());
        match prepared {
            Prepared::Binary => Shown::Message(BINARY.into()),
            Prepared::Text { text, note } => {
                let wrap = viewer == ViewerId::Text;
                let opts = SurfaceOpts { wrap, ..opts };
                let shown_viewer = if note.is_some() { ViewerId::Text } else { viewer };
                Shown::Code {
                    surface: Surface::new(&id, label, text, shown_viewer, opts, menu, window, cx),
                    bar: if wrap { CodeBar::Wrap(true) } else { CodeBar::None },
                    note,
                }
            }
            Prepared::Script { text, minified } => {
                let surface = Surface::new(
                    &id,
                    label,
                    text.clone(),
                    viewer,
                    SurfaceOpts { wrap: true, ..opts },
                    menu,
                    window,
                    cx,
                );
                let mut shown = Shown::Code {
                    surface,
                    bar: CodeBar::Beautify {
                        on: false,
                        original: text,
                        state: Beautified::Idle,
                    },
                    note: None,
                };
                if minified {
                    self.toggle_beautify_in(&mut shown, viewer, window, cx);
                }
                shown
            }
            Prepared::Tree(doc) => {
                let body = self.body_ref(bytes.as_deref().map(|b| b.as_slice()));
                let tree = cx.new(|cx| {
                    JsonTree::new(self.id("tree"), doc, window, cx).on_menu(move |req, window, cx| {
                        menus::tree(req.row.as_ref(), req.value, req.selection, body.as_ref()).show(
                            req.position,
                            window,
                            cx,
                        )
                    })
                });
                Shown::Tree(tree)
            }
            Prepared::Form { text, fields } => Shown::Form {
                fields: Rc::new(fields),
                raw: Surface::new(
                    &id,
                    label,
                    text,
                    ViewerId::Text,
                    SurfaceOpts { wrap: true, ..opts },
                    menu,
                    window,
                    cx,
                ),
                show_raw: false,
            },
            Prepared::Multipart { boundary, parts } => {
                let bytes = bytes.unwrap_or_default();
                if boundary.is_none() {
                    return Shown::Multipart {
                        boundary,
                        parts: Vec::new(),
                        hex: Some(hex_view(self.part, bytes.clone(), self.body_ref(Some(&bytes)), cx)),
                    };
                }
                let parts = parts
                    .into_iter()
                    .enumerate()
                    .map(|(i, part)| self.part_view(i, part, &bytes, window, cx))
                    .collect();
                Shown::Multipart {
                    boundary,
                    parts,
                    hex: None,
                }
            }
            Prepared::Graphql(Ok(ops)) => self.graphql(Rc::new(ops), 0, window, cx),
            Prepared::Graphql(Err(text)) => Shown::Code {
                surface: Surface::new(
                    &id,
                    label,
                    text,
                    ViewerId::Text,
                    SurfaceOpts { wrap: true, ..opts },
                    menu,
                    window,
                    cx,
                ),
                bar: CodeBar::Wrap(true),
                note: Some("Not a GraphQL request body. Showing the text.".into()),
            },
        }
    }

    fn current_bytes(&mut self, cx: &mut Context<Self>) -> Option<Arc<Vec<u8>>> {
        let input = self.input.clone()?;
        self.load(&input, cx).0.ready().map(|b| b.bytes.clone())
    }

    fn part_view(
        &self,
        i: usize,
        part: Part,
        bytes: &Arc<Vec<u8>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PartView {
        let body = &bytes[part.body.clone()];
        let sniffed = sniff(body, part.content_type.as_deref());
        let headers = Rc::new(
            part.headers
                .iter()
                .map(|(k, v)| {
                    (
                        SharedString::from(display_header_name(k)),
                        SharedString::from(v.clone()),
                    )
                })
                .collect(),
        );
        let viewer = viewers_for(sniffed.kind)[0];
        let short = sniffed.kind == ContentKind::Text && body.len() <= 200 && !body.contains(&b'\n');
        let id = format!("{}-{i}", self.id("part"));
        let body_view = if body.is_empty() {
            PartBody::None
        } else if short {
            PartBody::Inline(String::from_utf8_lossy(body).into_owned().into())
        } else if viewer == ViewerId::Hex || viewer.def().source != Source::Bytes {
            if viewer.def().source == Source::Bytes {
                PartBody::Hex(hex_view(self.part, Arc::new(body.to_vec()), None, cx))
            } else {
                PartBody::Message(
                    format!(
                        "{} preview: G5 adds media viewers. Copy or save the part from its menu.",
                        viewer.label()
                    )
                    .into(),
                )
            }
        } else {
            match prepare(viewer, body, part.content_type.as_deref()) {
                Prepared::Text { text, .. } | Prepared::Script { text, .. } | Prepared::Form { text, .. } => {
                    let text_bytes = bytes.clone();
                    let menu = {
                        let part = part.clone();
                        let mime = sniffed.mime.clone();
                        Rc::new(
                            move |selection: Option<String>,
                                  text: SharedString,
                                  position,
                                  window: &mut Window,
                                  cx: &mut App| {
                                let extra = menus::part(&part, &mime, &text_bytes[..]);
                                menus::code(selection, text, extra, None).show(position, window, cx)
                            },
                        ) as view::CodeMenu
                    };
                    let opts = SurfaceOpts::for_size(body.len(), false, true);
                    PartBody::Text(Surface::new(&id, "Part body", text, viewer, opts, menu, window, cx))
                }
                Prepared::Binary => PartBody::Hex(hex_view(self.part, Arc::new(body.to_vec()), None, cx)),
                _ => PartBody::Inline(String::from_utf8_lossy(body).into_owned().into()),
            }
        };
        PartView {
            mime: sniffed.mime,
            headers,
            body: body_view,
            part,
        }
    }

    fn graphql(&self, ops: Rc<Vec<Operation>>, index: usize, window: &mut Window, cx: &mut Context<Self>) -> Shown {
        let op = &ops[index.min(ops.len() - 1)];
        let surface = |what: &str, text: String, viewer: ViewerId, window: &mut Window, cx: &mut Context<Self>| {
            let menu: view::CodeMenu = Rc::new(move |selection, text, position, window, cx| {
                menus::code(selection, text, Menu::new(), None).show(position, window, cx)
            });
            let opts = SurfaceOpts::for_size(text.len(), false, false);
            Surface::new(&self.id(what), what, text.into(), viewer, opts, menu, window, cx)
        };
        Shown::Graphql {
            query: (!op.query.is_empty()).then(|| surface("query", op.query.clone(), ViewerId::Graphql, window, cx)),
            variables: op
                .variables_text()
                .map(|v| surface("variables", v, ViewerId::Json, window, cx)),
            extensions: op
                .extensions_text()
                .map(|v| surface("extensions", v, ViewerId::Json, window, cx)),
            ops,
            index,
        }
    }

    fn pick_operation(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Shown::Graphql { ops, .. }) = &self.shown {
            let ops = ops.clone();
            self.shown = Some(self.graphql(ops, index, window, cx));
            cx.notify();
        }
    }

    fn toggle_beautify_in(&mut self, shown: &mut Shown, viewer: ViewerId, window: &mut Window, cx: &mut Context<Self>) {
        let Shown::Code {
            surface,
            bar: CodeBar::Beautify { on, original, state },
            ..
        } = shown
        else {
            return;
        };
        *on = !*on;
        if !*on {
            surface.set_text(original.clone(), window, cx);
            surface.set_wrap(true, window, cx);
            return;
        }
        surface.set_wrap(false, window, cx);
        if let Beautified::Done(text) = state {
            surface.set_text(text.clone(), window, cx);
            return;
        }
        let language = if viewer == ViewerId::Css {
            beautify::Language::Css
        } else {
            beautify::Language::Javascript
        };
        let text = original.clone();
        let task = cx.background_spawn(async move { beautify::beautify(&text, language) });
        *state = Beautified::Running(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(Shown::Code {
                    surface,
                    bar: CodeBar::Beautify { on, state, .. },
                    ..
                }) = &mut this.shown
                {
                    match result {
                        Ok(text) => {
                            let text: SharedString = text.into();
                            if *on {
                                surface.set_text(text.clone(), window, cx);
                            }
                            *state = Beautified::Done(text);
                        }
                        Err(e) => *state = Beautified::Failed(e.into()),
                    }
                    cx.notify();
                }
            });
        }));
    }

    fn toggle_beautify(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let viewer = self.source.map_or(ViewerId::Javascript, |s| s.viewer);
        if let Some(mut shown) = self.shown.take() {
            self.toggle_beautify_in(&mut shown, viewer, window, cx);
            self.shown = Some(shown);
        }
        cx.notify();
    }

    fn toggle_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Shown::Code {
            surface,
            bar: CodeBar::Wrap(wrap),
            ..
        }) = &mut self.shown
        {
            *wrap = !*wrap;
            surface.set_wrap(*wrap, window, cx);
        }
        cx.notify();
    }

    fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(surface) = self.surface().cloned() {
            surface.select_all(window, cx);
        }
    }

    fn find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(surface) = self.surface().cloned() {
            surface.open_find(window, cx);
        }
    }

    /// Passes the body terms to whichever viewer shows.
    fn mark_terms(&self, terms: &[String], cx: &mut App) {
        match &self.shown {
            Some(Shown::Code { surface, .. }) => surface.set_terms(terms, cx),
            Some(Shown::Form { raw, .. }) => raw.set_terms(terms, cx),
            Some(Shown::Tree(tree)) => {
                let terms = terms.to_vec();
                tree.update(cx, |t, cx| t.set_terms(terms, cx));
            }
            Some(Shown::Graphql { query, variables, .. }) => {
                for s in [query, variables].into_iter().flatten() {
                    s.set_terms(terms, cx);
                }
            }
            _ => {}
        }
    }
}

fn hex_view(part: BodyPart, bytes: Arc<Vec<u8>>, body: Option<BodyRef>, cx: &mut App) -> Entity<HexView> {
    let id = match part {
        BodyPart::Request => "request-body-hex",
        BodyPart::Response => "response-body-hex",
    };
    let menu_bytes = bytes.clone();
    cx.new(|cx| {
        HexView::new(id, bytes, cx).on_menu(move |req, window, cx| {
            menus::hex(&menu_bytes, req.row, req.offset, req.selection, body.as_ref()).show(req.position, window, cx)
        })
    })
}

fn centered(text: impl Into<SharedString>, t: &Tokens) -> AnyElement {
    div()
        .id("body-message")
        .size_full()
        .p(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .text_color(t.muted_foreground.hsla())
        .child(text.into())
        .test_support()
        .into_any_element()
}

/// A one-line note above a body (decoding, truncation, limits).
fn note(id: &'static str, warn: bool, t: &Tokens) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .role(if warn { Role::Alert } else { Role::Status })
        .flex_none()
        .px(px(8.))
        .py(px(3.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(t.border.hsla())
        .text_size(px(11.))
        .when(warn, |d| d.bg(t.s_4xx_bg.hsla()).text_color(t.foreground.hsla()))
        .when(!warn, |d| d.text_color(t.muted_foreground.hsla()))
}

fn bar(t: &Tokens) -> gpui_kit::Div {
    div()
        .flex_none()
        .h(px(28.))
        .px(px(6.))
        .flex()
        .items_center()
        .gap(px(4.))
        .border_b_1()
        .border_color(t.border.hsla())
        .text_size(px(11.))
}

impl BodyPane {
    fn render_bar(
        &self,
        kind: ContentKind,
        viewer: ViewerId,
        input: &BodyInput,
        bytes: Option<&[u8]>,
        mono: SharedString,
        t: &Tokens,
    ) -> AnyElement {
        let part = self.part;
        let options: Vec<ViewerId> = viewers_for(kind).to_vec();
        let picker = Button::new(match part {
            BodyPart::Request => "request-viewer-select",
            BodyPart::Response => "response-viewer-select",
        })
        .outline()
        .xsmall()
        .label(SharedString::from(format!("{} ▾", viewer.label())))
        .tooltip("Viewer")
        .on_click(move |e: &ClickEvent, window, cx| {
            let menu = options.iter().fold(Menu::new(), |m, v| {
                m.check(v.label(), *v == viewer, ChooseViewer { kind, viewer: *v })
            });
            menu.show(e.position(), window, cx);
        });
        let flow = input.flow();
        let can_copy = bytes.is_some_and(|b| !b.is_empty());
        let file_name: SharedString = suggested_file_name(&input.url, &input.mime(), part).into();
        bar(t)
            .child(picker)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(mono)
                    .text_color(t.muted_foreground.hsla())
                    .child(SharedString::from(format!(
                        "{} · {}",
                        input.mime(),
                        format_size(input.info.size)
                    ))),
            )
            .child(
                Button::new(match part {
                    BodyPart::Request => "request-copy-body",
                    BodyPart::Response => "response-copy-body",
                })
                .ghost()
                .xsmall()
                .icon(gpui_kit::assets::IconName::Copy)
                .label("Copy")
                .disabled(!can_copy)
                .tooltip(if bytes.is_some() {
                    "Copy the body as text (Base64 from the right-click menu)"
                } else {
                    "The viewer loads the body itself: save it instead"
                })
                .on_click(move |_, window, cx| {
                    window.dispatch_action(
                        Box::new(CopyBody {
                            flow,
                            part,
                            base64: false,
                        }),
                        cx,
                    )
                }),
            )
            .child(
                Button::new(match part {
                    BodyPart::Request => "request-save-body",
                    BodyPart::Response => "response-save-body",
                })
                .ghost()
                .xsmall()
                .icon(gpui_kit::assets::IconName::Download)
                .label("Save…")
                .on_click(move |_, window, cx| {
                    window.dispatch_action(
                        Box::new(SaveBody {
                            flow,
                            part,
                            file_name: file_name.clone(),
                        }),
                        cx,
                    )
                }),
            )
            .into_any_element()
    }
}

impl BodyPane {
    fn render_shown(&self, t: &Tokens, mono: &SharedString, cx: &mut Context<Self>) -> AnyElement {
        let Some(shown) = &self.shown else {
            return centered(
                if self.preparing.is_some() {
                    "Formatting…"
                } else {
                    "Loading…"
                },
                t,
            );
        };
        match shown {
            Shown::Message(text) => centered(text.clone(), t),
            Shown::Tree(tree) => tree.clone().into_any_element(),
            Shown::Hex(hex) => hex.clone().into_any_element(),
            Shown::Code {
                surface,
                bar: code_bar,
                note: n,
            } => {
                let tools = match code_bar {
                    CodeBar::None => None,
                    CodeBar::Wrap(wrap) => Some(
                        bar(t)
                            .child(
                                Button::new(self.id("wrap"))
                                    .ghost()
                                    .xsmall()
                                    .icon(gpui_kit::assets::IconName::TextWrap)
                                    .label("Wrap")
                                    .selected(*wrap)
                                    .tooltip("Wrap lines")
                                    .on_click(cx.listener(|this, _, window, cx| this.toggle_wrap(window, cx))),
                            )
                            .into_any_element(),
                    ),
                    CodeBar::Beautify { on, state, .. } => Some(
                        bar(t)
                            .child(
                                Button::new(self.id("beautify"))
                                    .ghost()
                                    .xsmall()
                                    .icon(gpui_kit::assets::IconName::WandSparkles)
                                    .label("Beautify")
                                    .selected(*on)
                                    .tooltip("Beautify")
                                    .on_click(cx.listener(|this, _, window, cx| this.toggle_beautify(window, cx))),
                            )
                            .children(match state {
                                Beautified::Running(_) if *on => Some(
                                    div()
                                        .text_color(t.muted_foreground.hsla())
                                        .child("Formatting…")
                                        .into_any_element(),
                                ),
                                Beautified::Failed(e) if *on => Some(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(t.s_failed.hsla())
                                        .child(SharedString::from(format!("Couldn't beautify: {e}")))
                                        .into_any_element(),
                                ),
                                _ => None,
                            })
                            .into_any_element(),
                    ),
                };
                let terms = self.input.as_ref().map(|i| i.terms.clone()).unwrap_or_default();
                let search = match surface {
                    Surface::Editor(editor) if !terms.is_empty() => Some(self.match_bar(editor, &terms, t, cx)),
                    _ => None,
                };
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .children(tools)
                    .children(search)
                    .children(n.clone().map(|n| note("body-invalid-note", true, t).child(n)))
                    .child(div().flex_1().min_h_0().child(surface.element()))
                    .into_any_element()
            }
            Shown::Form { fields, raw, show_raw } => {
                let count = fields.len();
                let toggle = |id: &'static str, label: &'static str, on: bool, raw: bool| {
                    Button::new(id)
                        .ghost()
                        .xsmall()
                        .label(label)
                        .selected(on)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(Shown::Form { show_raw, .. }) = &mut this.shown {
                                *show_raw = raw;
                                cx.notify();
                            }
                        }))
                };
                let content = if *show_raw {
                    raw.element()
                } else {
                    let rows = fields.clone();
                    let shared = Rc::new(
                        fields
                            .iter()
                            .map(|(k, v)| (SharedString::from(k.clone()), SharedString::from(v.clone())))
                            .collect(),
                    );
                    let body = self.form_body_ref(cx);
                    div()
                        .id(self.id("form-scroll"))
                        .size_full()
                        .overflow_y_scroll()
                        .child(KvTable::new(self.id("form"), "Form fields", shared).on_menu(
                            move |index, position, window, cx| {
                                let selected = crate::features::context_menu::selected_text(window, cx);
                                let menu =
                                    crate::features::inspector::menus::kv(&rows, KvKind::Form, None, index, selected);
                                menus::body_items(menu, body.as_ref()).show(position, window, cx);
                            },
                        ))
                        .into_any_element()
                };
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        bar(t)
                            .child(toggle("form-fields", "Fields", !*show_raw, false))
                            .child(toggle("form-raw", "Raw", *show_raw, true))
                            .child(
                                div()
                                    .ml_auto()
                                    .text_color(t.muted_foreground.hsla())
                                    .child(SharedString::from(format!(
                                        "{count} field{}",
                                        if count == 1 { "" } else { "s" }
                                    ))),
                            ),
                    )
                    .child(div().flex_1().min_h_0().child(content))
                    .into_any_element()
            }
            Shown::Multipart { boundary, parts, hex } => {
                if let Some(hex) = hex {
                    return div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(
                            note("multipart-no-boundary", true, t)
                                .child("The Content-Type has no boundary, so the parts can't be split."),
                        )
                        .child(div().flex_1().min_h_0().child(hex.clone()))
                        .into_any_element();
                }
                let boundary = boundary.clone().unwrap_or_default();
                if parts.is_empty() {
                    return centered(format!("No parts found for boundary {boundary}"), t);
                }
                let bytes = self.form_bytes(cx);
                div()
                    .id(self.id("multipart"))
                    .role(Role::List)
                    .aria_label("Multipart parts")
                    .size_full()
                    .overflow_y_scroll()
                    .p(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.muted_foreground.hsla())
                            .child(SharedString::from(format!(
                                "{} part{} · boundary {boundary}",
                                parts.len(),
                                if parts.len() == 1 { "" } else { "s" }
                            ))),
                    )
                    .children(
                        parts
                            .iter()
                            .enumerate()
                            .map(|(i, p)| self.part_card(i, p, bytes.clone(), t, mono)),
                    )
                    .into_any_element()
            }
            Shown::Graphql {
                ops,
                index,
                query,
                variables,
                extensions,
            } => {
                let op = &ops[(*index).min(ops.len() - 1)];
                let menu_op = op.clone();
                let picker = (ops.len() > 1).then(|| {
                    div().flex().gap(px(2.)).children(ops.iter().enumerate().map(|(i, o)| {
                        Button::new(("graphql-op", i))
                            .ghost()
                            .xsmall()
                            .label(SharedString::from(
                                o.operation_name.clone().unwrap_or_else(|| format!("#{}", i + 1)),
                            ))
                            .selected(i == *index)
                            .on_click(cx.listener(move |this, _, window, cx| this.pick_operation(i, window, cx)))
                    }))
                });
                let pane = |title: &'static str, content: AnyElement| {
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex_none()
                                .h(px(20.))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .border_b_1()
                                .border_color(t.border.hsla())
                                .bg(t.pane_header.hsla())
                                .text_size(px(10.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.muted_foreground.hsla())
                                .child(title),
                        )
                        .child(div().flex_1().min_h_0().child(content))
                };
                let empty = |text: &'static str| {
                    div()
                        .p(px(12.))
                        .text_size(px(12.))
                        .text_color(t.muted_foreground.hsla())
                        .child(text)
                        .into_any_element()
                };
                div()
                    .id(self.id("graphql"))
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        bar(t)
                            .children(picker)
                            .children(op.kind.clone().map(|k| {
                                div()
                                    .font_family(mono.clone())
                                    .text_color(t.syntax_keyword.hsla())
                                    .child(k)
                            }))
                            .child(
                                div()
                                    .id(self.id("graphql-name"))
                                    .font_family(mono.clone())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        menus::graphql(&menu_op).show(e.position, window, cx);
                                    })
                                    .child(SharedString::from(
                                        op.operation_name.clone().unwrap_or_else(|| "anonymous".into()),
                                    )),
                            )
                            .when(ops.len() > 1, |d| {
                                d.child(
                                    div()
                                        .ml_auto()
                                        .text_color(t.muted_foreground.hsla())
                                        .child(SharedString::from(format!("batch of {}", ops.len()))),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_row()
                            .child(
                                div().w(gpui_kit::relative(0.58)).h_full().flex().flex_col().child(pane(
                                    "QUERY",
                                    query
                                        .as_ref()
                                        .map(Surface::element)
                                        .unwrap_or_else(|| empty("Persisted query: no query text was sent")),
                                )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .h_full()
                                    .border_l_1()
                                    .border_color(t.border.hsla())
                                    .flex()
                                    .flex_col()
                                    .child(pane(
                                        "VARIABLES",
                                        variables
                                            .as_ref()
                                            .map(Surface::element)
                                            .unwrap_or_else(|| empty("No variables")),
                                    ))
                                    .children(extensions.as_ref().map(|e| pane("EXTENSIONS", e.element()))),
                            ),
                    )
                    .into_any_element()
            }
        }
    }

    /// "Body search: … · 3 of 12 ⌃ ⌄" over the editor (the CodeView draws
    /// its own).
    fn match_bar(
        &self,
        editor: &Entity<pm_widgets::code_editor::CodeEditor>,
        terms: &[String],
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (count, current) = (editor.read(cx).match_count(), editor.read(cx).current_match());
        let label = pm_view::body::search::match_label(count, current);
        let step = |id: &'static str, icon: gpui_kit::assets::IconName, forward: bool, tip: &'static str| {
            let editor = editor.clone();
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip(tip)
                .disabled(count == 0)
                .on_click(cx.listener(move |_, _, window, cx| {
                    editor.update(cx, |e, cx| e.step(forward, window, cx));
                    cx.notify();
                }))
        };
        bar(t)
            .bg(t.pane_header.hsla())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(t.muted_foreground.hsla())
                    .child(SharedString::from(format!("Body search: {}", terms.join(", ")))),
            )
            .child(
                div()
                    .id(self.id("match-count"))
                    .role(Role::Status)
                    .aria_label(SharedString::from(label.clone()))
                    .mr(px(4.))
                    .text_color(t.muted_foreground.hsla())
                    .child(label),
            )
            .child(step(
                "body-match-prev",
                gpui_kit::assets::IconName::ChevronUp,
                false,
                "Previous match",
            ))
            .child(step(
                "body-match-next",
                gpui_kit::assets::IconName::ChevronDown,
                true,
                "Next match",
            ))
            .into_any_element()
    }

    fn form_body_ref(&self, cx: &App) -> Option<BodyRef> {
        let bytes = self.form_bytes(cx);
        self.body_ref(bytes.as_deref().map(|b| b.as_slice()))
    }

    /// The loaded body's bytes, without fetching.
    fn form_bytes(&self, cx: &App) -> Option<Arc<Vec<u8>>> {
        let input = self.input.as_ref()?;
        let capped = input.info.captured > LOAD_LIMIT && !self.load_all;
        let key = DecodedKey {
            flow: input.key,
            part: input.part,
            max: capped.then_some(LOAD_LIMIT),
        };
        self.caches.decoded.read(cx).peek(&key).map(|b| b.bytes.clone())
    }

    fn part_card(
        &self,
        i: usize,
        p: &PartView,
        bytes: Option<Arc<Vec<u8>>>,
        t: &Tokens,
        mono: &SharedString,
    ) -> AnyElement {
        let part = p.part.clone();
        let mime = p.mime.clone();
        let size = p.part.body.len() as u64;
        let header_menu = {
            let part = part.clone();
            let mime = mime.clone();
            let bytes = bytes.clone().unwrap_or_default();
            move |e: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                cx.stop_propagation();
                menus::part(&part, &mime, &bytes).show(e.position, window, cx);
            }
        };
        let rows: Vec<(String, String)> = p.headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let body = match &p.body {
            PartBody::None => None,
            PartBody::Inline(text) => Some(
                div()
                    .border_t_1()
                    .border_color(t.border.hsla())
                    .px(px(8.))
                    .py(px(4.))
                    .font_family(mono.clone())
                    .text_size(px(12.))
                    .child(SelectableText::new(
                        ElementId::Name(format!("part-{i}-text").into()),
                        text.clone(),
                    ))
                    .into_any_element(),
            ),
            PartBody::Text(surface) => Some(self.part_box(surface.element(), t)),
            PartBody::Hex(hex) => Some(self.part_box(hex.clone().into_any_element(), t)),
            PartBody::Message(m) => Some(
                div()
                    .border_t_1()
                    .border_color(t.border.hsla())
                    .p(px(8.))
                    .text_size(px(11.))
                    .text_color(t.muted_foreground.hsla())
                    .child(m.clone())
                    .into_any_element(),
            ),
        };
        div()
            .id(("multipart-part", i))
            .role(Role::ListItem)
            .aria_label(SharedString::from(format!(
                "Part {}",
                p.part.name.clone().unwrap_or_else(|| (i + 1).to_string())
            )))
            .flex_none()
            .rounded(px(6.))
            .border_1()
            .border_color(t.border.hsla())
            .bg(t.card.hsla())
            .overflow_hidden()
            .child(
                div()
                    .id(("multipart-part-head", i))
                    .px(px(8.))
                    .py(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(t.border.hsla())
                    .bg(t.pane_header.hsla())
                    .text_size(px(11.))
                    .on_mouse_down(MouseButton::Right, header_menu)
                    .child(
                        div()
                            .font_family(mono.clone())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.syntax_key.hsla())
                            .child(SharedString::from(
                                p.part.name.clone().unwrap_or_else(|| "(unnamed)".into()),
                            )),
                    )
                    .children(
                        p.part
                            .filename
                            .clone()
                            .map(|f| div().font_family(mono.clone()).child(SharedString::from(f))),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .text_color(t.muted_foreground.hsla())
                            .child(SharedString::from(format!(
                                "{} · {}",
                                p.part.content_type.clone().unwrap_or_else(|| mime.clone()),
                                if size == 0 { "0 B".into() } else { format_size(size) }
                            ))),
                    ),
            )
            .child(
                KvTable::new(format!("part-{i}-headers"), "Part headers", p.headers.clone()).on_menu(
                    move |index, position, window, cx| {
                        let selected = crate::features::context_menu::selected_text(window, cx);
                        crate::features::inspector::menus::kv(&rows, KvKind::Header, None, index, selected)
                            .show(position, window, cx);
                    },
                ),
            )
            .children(body)
            .into_any_element()
    }

    fn part_box(&self, content: AnyElement, t: &Tokens) -> AnyElement {
        div()
            .h(px(224.))
            .border_t_1()
            .border_color(t.border.hsla())
            .child(content)
            .into_any_element()
    }
}

impl Render for BodyPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        let Some(input) = self.input.clone() else {
            return div().into_any_element();
        };
        let part = self.part;
        let kind = input.kind();
        let viewer = pick_viewer(kind, choice(kind, cx));
        let def = viewer.def();
        let info = &input.info;
        let root = |content: AnyElement| {
            div()
                .id(match part {
                    BodyPart::Request => "request-body-viewer",
                    BodyPart::Response => "response-body-viewer",
                })
                .role(Role::Region)
                .aria_label(match part {
                    BodyPart::Request => "Request body",
                    BodyPart::Response => "Response body",
                })
                .size_full()
                .flex()
                .flex_col()
                .child(content)
                .test_support()
                .into_any_element()
        };
        // A GET GraphQL request: its operation from the query string.
        if info.size == 0
            && kind == ContentKind::GraphQl
            && part == BodyPart::Request
            && let Some(op) = graphql::parse_query_string(&input.url)
        {
            let source = Source_ {
                body: 1,
                viewer: ViewerId::Graphql,
                highlight_anyway: false,
            };
            if self.source != Some(source) {
                self.source = Some(source);
                self.shown = Some(self.graphql(Rc::new(vec![op]), 0, window, cx));
            }
            return root(self.render_shown(&t, &mono, cx));
        }
        if info.size == 0 && !(kind == ContentKind::Grpc && part == BodyPart::Response) {
            self.source = None;
            self.shown = None;
            return root(centered(
                if input.finished {
                    "No body"
                } else {
                    "Waiting for body…"
                },
                &t,
            ));
        }
        if !input.finished && def.source != Source::Decoded {
            self.source = None;
            self.shown = None;
            return root(centered(
                format!("Body streaming · {} so far", format_size(info.size)),
                &t,
            ));
        }
        let (load, capped) = if def.source == Source::Bytes {
            self.load(&input, cx)
        } else {
            (Load::Loading, false)
        };
        let body = load.ready().cloned();
        let bytes = body.as_ref().map(|b| b.bytes.clone());
        let mut notes: Vec<AnyElement> = Vec::new();
        if let Some(b) = &body {
            if let Some(from) = &b.decoded_from {
                let shown = if capped {
                    format!("{}+", format_size(b.bytes.len() as u64))
                } else {
                    format_size(b.bytes.len() as u64)
                };
                notes.push(
                    note("body-decoded-note", false, &t)
                        .child(SharedString::from(format!(
                            "Decoded from {from} · {} → {shown}",
                            format_size(info.captured)
                        )))
                        .test_support()
                        .into_any_element(),
                );
            }
            if let Some(err) = &b.decode_error {
                notes.push(
                    note("body-decode-error", true, &t)
                        .child(SharedString::from(format!(
                            "Couldn't decode: {err}. Showing the raw bytes."
                        )))
                        .test_support()
                        .into_any_element(),
                );
            }
        }
        if info.truncated {
            notes.push(
                note("body-truncated-note", false, &t)
                    .child(SharedString::from(format!(
                        "Recorded the first {} of {}",
                        format_size(info.captured),
                        format_size(info.size)
                    )))
                    .test_support()
                    .into_any_element(),
            );
        }
        if capped {
            let flow = input.flow();
            let file_name: SharedString = suggested_file_name(&input.url, &input.mime(), part).into();
            notes.push(
                note("large-body-note", true, &t)
                    .child(SharedString::from(format!(
                        "Showing the first {} of {}.",
                        format_size(LOAD_LIMIT),
                        format_size(info.captured)
                    )))
                    .child(
                        Button::new("body-load-all")
                            .outline()
                            .xsmall()
                            .label("Load all")
                            .on_click(cx.listener(|this, _, _, cx| this.load_all(cx))),
                    )
                    .child(
                        Button::new("body-save-large")
                            .outline()
                            .xsmall()
                            .label("Save body to file…")
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(SaveBody {
                                        flow,
                                        part,
                                        file_name: file_name.clone(),
                                    }),
                                    cx,
                                )
                            }),
                    )
                    .test_support()
                    .into_any_element(),
            );
        }
        let too_big = bytes.as_ref().is_some_and(|b| b.len() > HIGHLIGHT_LIMIT);
        if def.code && too_big && !self.highlight_anyway {
            notes.push(
                note("highlight-note", false, &t)
                    .child(SharedString::from(format!(
                        "Highlighting is off for bodies over {}.",
                        format_size(HIGHLIGHT_LIMIT as u64)
                    )))
                    .child(
                        Button::new("body-highlight-anyway")
                            .outline()
                            .xsmall()
                            .label("Highlight anyway")
                            .on_click(cx.listener(|this, _, _, cx| this.highlight_anyway(cx))),
                    )
                    .test_support()
                    .into_any_element(),
            );
        }
        if let Some(err) = cx
            .try_global::<Choices>()
            .and_then(|c| c.save_errors.get(&(input.key.store, input.key.id, part)))
        {
            notes.push(
                note("body-save-error", true, &t)
                    .child(SharedString::from(format!("Couldn't save: {err}")))
                    .test_support()
                    .into_any_element(),
            );
        }
        let content = match (def.source, load) {
            (Source::Url | Source::Decoded, _) => {
                self.source = None;
                self.shown = None;
                centered(
                    format!(
                        "The {} viewer arrives in G5. Hex shows the bytes; Save… writes the file.",
                        viewer.label()
                    ),
                    &t,
                )
            }
            (_, Load::Loading) => centered("Loading…", &t),
            (_, Load::Failed(err)) => centered(format!("Couldn't load body: {err}"), &t),
            (_, Load::Ready(body)) => {
                self.ensure_shown(&body, viewer, &input, window, cx);
                self.mark_terms(&input.terms, cx);
                self.render_shown(&t, &mono, cx)
            }
        };
        let fallback = self.body_ref(bytes.as_deref().map(|b| b.as_slice()));
        root(
            div()
                .size_full()
                .flex()
                .flex_col()
                .key_context("BodyViewer")
                .on_action(cx.listener(|this, _: &SelectAllInBody, window, cx| this.select_all(window, cx)))
                .on_action(cx.listener(|this, _: &FindInBody, window, cx| this.find(window, cx)))
                .child(self.render_bar(
                    kind,
                    viewer,
                    &input,
                    bytes.as_deref().map(|b| b.as_slice()),
                    mono.clone(),
                    &t,
                ))
                .children(notes)
                .child(
                    div()
                        .id(match part {
                            BodyPart::Request => "request-body-content",
                            BodyPart::Response => "response-body-content",
                        })
                        .flex_1()
                        .min_h_0()
                        .on_mouse_down(MouseButton::Right, move |e: &MouseDownEvent, window, cx| {
                            let selected = crate::features::context_menu::selected_text(window, cx);
                            menus::fallback(selected, fallback.as_ref()).show(e.position, window, cx);
                        })
                        .child(content),
                )
                .into_any_element(),
        )
    }
}
