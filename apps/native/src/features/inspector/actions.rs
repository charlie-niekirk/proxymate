//! The inspector's and the flow row menu's actions (`lib/actions.ts`): what
//! a menu item or button dispatches. Copies of things the engine builds (a
//! URL, a cURL command, headers, a body, the raw message) load on the
//! background executor through `clipboard::copy_later`; writes go through
//! `bridge::mutations`.

use std::sync::Arc;

use gpui_kit::{App, SharedString};
use pm_core::Engine;
use pm_model::{BodyPart, FlowId, FlowSelection, StoreId};
use pm_view::copy_formats::{headers_block, with_term};
use pm_view::http::{header_rows, raw_message};

use crate::bridge::mutations;
use crate::features::clipboard::copy_later;
use crate::features::documents::{self, with_workspace};
use crate::workspace::{SaveKind, Workspace};

/// Narrows the search box by a DSL term (Filter by ▸, Filter flows with
/// this header).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct AddFilterTerm {
    pub term: SharedString,
}

/// One flow of one store, for the actions below.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FlowRef {
    pub store: StoreId,
    pub id: FlowId,
}

macro_rules! flow_action {
    ($(#[$doc:meta])* $name:ident { $($field:ident: $ty:ty),* }) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
        #[action(namespace = proxymate, no_json)]
        pub struct $name {
            pub flow: FlowRef,
            $(pub $field: $ty,)*
        }
    };
}

flow_action!(
    /// The request's URL as recorded.
    CopyFlowUrl {}
);
flow_action!(
    /// A shell command (built by the engine, bodies included) that sends
    /// the request again.
    CopyFlowCurl {}
);
flow_action!(
    /// A message's headers as `Name: value` lines.
    CopyFlowHeaders { part: BodyPart }
);
flow_action!(
    /// A body, decoded, as text (binary bodies explain themselves).
    CopyFlowBody { part: BodyPart }
);
flow_action!(
    /// The message as the Raw tab rebuilds it.
    CopyFlowRaw { part: BodyPart }
);
flow_action!(
    /// Pin or Unpin.
    PinFlow { pinned: bool }
);
flow_action!(
    /// Export HAR… of this flow alone.
    ExportFlowHar {}
);
flow_action!(
    /// Delete ⌫ (this flow alone).
    DeleteFlow {}
);

/// Never record <host>.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct NeverRecord {
    pub host: SharedString,
}

/// A rule badge's "Disable rule".
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct DisableRule {
    pub id: SharedString,
}

gpui_kit::actions!(proxymate, [FocusTable, FocusInspector]);

pub fn register(cx: &mut App) {
    cx.on_action(|a: &AddFilterTerm, cx| add_filter_term(&a.term, cx));
    cx.on_action(|a: &CopyFlowUrl, cx| copy_url(a.flow, cx));
    cx.on_action(|a: &CopyFlowCurl, cx| copy_curl(a.flow, cx));
    cx.on_action(|a: &CopyFlowHeaders, cx| {
        let (flow, part) = (a.flow, a.part);
        with_engine(cx, |engine, cx| {
            copy_later(
                move || headers_of(&engine, flow, part).map(Some),
                what(part, "headers"),
                cx,
            )
        });
    });
    cx.on_action(|a: &CopyFlowBody, cx| {
        let (flow, part) = (a.flow, a.part);
        with_engine(cx, |engine, cx| {
            copy_later(move || body_of(&engine, flow, part).map(Some), what(part, "body"), cx)
        });
    });
    cx.on_action(|a: &CopyFlowRaw, cx| {
        let (flow, part) = (a.flow, a.part);
        let label = match part {
            BodyPart::Request => "request",
            BodyPart::Response => "response",
        };
        with_engine(cx, |engine, cx| {
            copy_later(move || raw_of(&engine, flow, part).map(Some), label, cx)
        });
    });
    cx.on_action(|a: &PinFlow, cx| {
        let ids = vec![a.flow.id];
        mutations::pin_flows(a.flow.store, FlowSelection::Ids { ids }, a.pinned, cx);
    });
    cx.on_action(|a: &ExportFlowHar, cx| {
        let flow = a.flow;
        with_workspace(cx, |w, cx| w.set_store(flow.store, cx));
        documents::save(SaveKind::Har, Some(FlowSelection::Ids { ids: vec![flow.id] }), cx);
    });
    cx.on_action(|a: &DeleteFlow, cx| {
        let ids = vec![a.flow.id];
        mutations::remove_flows(a.flow.store, FlowSelection::Ids { ids }, |_, _| {}, cx);
    });
    cx.on_action(|a: &NeverRecord, cx| mutations::never_record(a.host.to_string(), cx));
    cx.on_action(|a: &DisableRule, cx| mutations::set_rule_enabled(a.id.to_string(), false, cx));
}

/// `text` narrowed by `term` in the search box.
pub fn add_filter_term(term: &str, cx: &mut App) {
    with_workspace(cx, |w, cx| {
        let text = with_term(&w.filter.text, term);
        w.set_text(text, cx);
    });
}

fn with_engine(cx: &mut App, f: impl FnOnce(Arc<Engine>, &mut App)) {
    if let Some(engine) = Workspace::global(cx).and_then(|w| w.read(cx).engine().cloned()) {
        f(engine, cx);
    }
}

fn what(part: BodyPart, thing: &str) -> String {
    match part {
        BodyPart::Request => format!("request {thing}"),
        BodyPart::Response => format!("response {thing}"),
    }
}

/// The recorded URL (the summary's for a synthetic flow).
pub fn copy_url(flow: FlowRef, cx: &mut App) {
    with_engine(cx, |engine, cx| {
        copy_later(
            move || {
                let detail = engine.flow_detail_in(flow.store, flow.id).map_err(|e| e.to_string())?;
                if let Some(detail) = detail {
                    return Ok(Some(detail.request.url));
                }
                Ok(engine
                    .get_flow_in(flow.store, flow.id)
                    .map(|f| pm_view::flow::flow_url(f.scheme, &f.host, f.port, &f.path, &f.query)))
            },
            "URL",
            cx,
        )
    });
}

/// Every URL of a selection, a line each.
pub fn copy_urls(store: StoreId, selection: FlowSelection, cx: &mut App) {
    with_engine(cx, |engine, cx| {
        copy_later(
            move || {
                let urls = engine.flow_urls(store, &selection).map_err(|e| e.to_string())?;
                Ok(Some(urls.join("\n")))
            },
            "URLs",
            cx,
        )
    });
}

pub fn copy_curl(flow: FlowRef, cx: &mut App) {
    with_engine(cx, |engine, cx| {
        copy_later(
            move || {
                engine
                    .curl_command(flow.store, flow.id)
                    .map(Some)
                    .map_err(|e| e.to_string())
            },
            "cURL command",
            cx,
        )
    });
}

fn detail_of(engine: &Engine, flow: FlowRef) -> Result<pm_model::FlowDetail, String> {
    engine
        .flow_detail_in(flow.store, flow.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "This flow has no recorded messages".to_owned())
}

/// A message's headers as `Name: value` lines.
pub fn headers_of(engine: &Engine, flow: FlowRef, part: BodyPart) -> Result<String, String> {
    let detail = detail_of(engine, flow)?;
    let headers = match part {
        BodyPart::Request => &detail.request.headers,
        BodyPart::Response => &detail.response.as_ref().ok_or("No response yet")?.headers,
    };
    Ok(headers_block(&header_rows(headers)))
}

/// A body, decoded, as text: a binary one says to copy it as Base64 from
/// the Body tab.
pub fn body_of(engine: &Engine, flow: FlowRef, part: BodyPart) -> Result<String, String> {
    let body = engine
        .body_head_in(flow.store, flow.id, part, true, None)
        .map_err(|e| e.to_string())?;
    pm_view::inspector::copy_body_text(&body.bytes, part)
}

/// The message as the Raw tab rebuilds it (decoded body, or a note for a
/// binary one).
pub fn raw_of(engine: &Engine, flow: FlowRef, part: BodyPart) -> Result<String, String> {
    let detail = detail_of(engine, flow)?;
    let body = engine
        .body_head_in(flow.store, flow.id, part, true, None)
        .map_err(|e| e.to_string())?;
    let text =
        (!body.bytes.is_empty()).then(|| pm_view::inspector::raw_body_text(&body.bytes, body.bytes.len() as u64));
    match part {
        BodyPart::Request => Ok(raw_message(
            &pm_view::inspector::request_line(&detail),
            &detail.request.headers,
            text.as_deref(),
        )),
        BodyPart::Response => {
            let line = pm_view::inspector::status_line(&detail).ok_or("No response yet")?;
            let headers = &detail.response.as_ref().ok_or("No response yet")?.headers;
            Ok(raw_message(&line, headers, text.as_deref()))
        }
    }
}
