//! What the inspector and the flow row menu show and copy, with no gpui:
//! the KvTable menus (`Inspector/KvTable.tsx`), the URL bar's status and
//! filters (`DetailPane.tsx`, `menus/filter.tsx`), the Raw tab's body rules
//! (`Inspector/RawView.tsx`), which message panel a flow gets
//! (`lib/stream.ts`) and "Copy <column>" (`FlowContextMenu.tsx` `cellText`).

use pm_model::{BodyInfo, BodyPart, FlowDetail, FlowRow, FlowSummary, Scheme};

use crate::columns::{Builtin, Column, ColumnId};
use crate::copy_formats::{
    Cookie, auth_token, dsl_term, form_pair, header_line, headers_block, parse_cookie, parse_set_cookie, plain_block,
    query_string,
};
use crate::flow::{format_duration, format_size, format_time, state_name, version_label};
use crate::http::{body_text, header_value};

/// What a name/value table's rows are, for what its menu copies: header
/// lines, URL-encoded query or form pairs, or plain `name: value` rows (TLS,
/// certificates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KvKind {
    Header,
    Query,
    Form,
    Plain,
}

impl KvKind {
    /// "Copy all <headers>".
    pub fn all_label(self) -> &'static str {
        match self {
            KvKind::Header => "headers",
            KvKind::Query => "parameters",
            KvKind::Form => "fields",
            KvKind::Plain => "rows",
        }
    }
}

/// The whole table as text: header lines, a query string, or `name: value` lines.
pub fn all_text<S: AsRef<str>>(rows: &[(S, S)], kind: KvKind) -> String {
    match kind {
        KvKind::Header => headers_block(rows),
        KvKind::Query | KvKind::Form => query_string(rows),
        KvKind::Plain => plain_block(rows),
    }
}

/// One copy item of a menu: its label, what it copies, and what the toast
/// calls it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyItem {
    pub label: String,
    pub text: String,
    pub what: String,
}

impl CopyItem {
    pub fn new(label: impl Into<String>, text: impl Into<String>, what: impl Into<String>) -> CopyItem {
        CopyItem {
            label: label.into(),
            text: text.into(),
            what: what.into(),
        }
    }
}

/// The copy items for one row of a table (`RowItems`): its name, value and
/// line, then a header's token and cookies. Several cookies are a submenu
/// ([`row_cookies`]); one is a plain item here.
pub fn row_copies(name: &str, value: &str, kind: KvKind) -> Vec<CopyItem> {
    let header = kind == KvKind::Header;
    let mut items = vec![
        CopyItem::new("Copy name", name, if header { "header name" } else { "name" }),
        CopyItem::new("Copy value", value, if header { "header value" } else { "value" }),
        match kind {
            KvKind::Header => CopyItem::new("Copy as “Name: value”", header_line(name, value), "header"),
            KvKind::Plain => CopyItem::new("Copy row", format!("{name}: {value}"), "row"),
            KvKind::Query => CopyItem::new("Copy as name=value", form_pair(name, value), "parameter"),
            KvKind::Form => CopyItem::new("Copy as name=value", form_pair(name, value), "field"),
        },
    ];
    if let Some(token) = row_token(name, value, kind) {
        items.push(CopyItem::new("Copy token", token, "token"));
    }
    if let [cookie] = row_cookies(name, value, kind).as_slice() {
        items.push(CopyItem::new(
            "Copy cookie value",
            cookie.value.clone(),
            format!("cookie {}", cookie.name),
        ));
    }
    items
}

/// An `Authorization` header's credentials, without the scheme.
pub fn row_token(name: &str, value: &str, kind: KvKind) -> Option<String> {
    let auth = name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("proxy-authorization");
    (kind == KvKind::Header && auth).then(|| auth_token(value)).flatten()
}

/// The cookies a `Cookie` or `Set-Cookie` header row carries.
pub fn row_cookies(name: &str, value: &str, kind: KvKind) -> Vec<Cookie> {
    if kind != KvKind::Header {
        return Vec::new();
    }
    if name.eq_ignore_ascii_case("cookie") {
        parse_cookie(value)
    } else if name.eq_ignore_ascii_case("set-cookie") {
        parse_set_cookie(value).into_iter().collect()
    } else {
        Vec::new()
    }
}

/// "Filter flows with this header": a `reqheader:`/`resheader:` term.
pub fn header_filter_term(part: BodyPart, name: &str, value: &str) -> String {
    let key = match part {
        BodyPart::Request => "reqheader",
        BodyPart::Response => "resheader",
    };
    dsl_term(key, &header_line(name, value))
}

/// One "Filter by ▸" item: its label, the value shown beside it, the term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterBy {
    pub label: &'static str,
    pub value: String,
    pub term: String,
}

/// "Filter by ▸ Host, Method, Status, App": the facts a flow has.
pub fn filter_by(host: &str, method: &str, status: Option<u16>, client: Option<&str>) -> Vec<FilterBy> {
    let mut items = Vec::new();
    let mut push = |label, value: String, key| {
        let term = dsl_term(key, &value);
        items.push(FilterBy { label, value, term });
    };
    if !host.is_empty() {
        push("Host", host.to_owned(), "host");
    }
    if !method.is_empty() {
        push("Method", method.to_owned(), "method");
    }
    if let Some(status) = status {
        push("Status", status.to_string(), "status");
    }
    if let Some(client) = client.filter(|c| !c.is_empty()) {
        push("App", client.to_owned(), "app");
    }
    items
}

/// The status the URL bar's menu copies: `200 OK` from the response, else
/// the summary's code.
pub fn status_text(flow: &FlowSummary, detail: Option<&FlowDetail>) -> Option<String> {
    match detail.and_then(|d| d.response.as_ref()) {
        Some(r) => Some(format!("{} {}", r.status, r.reason).trim().to_owned()),
        None => flow.status.map(|s| s.to_string()),
    }
}

/// `/path?query`.
pub fn path_and_query(path: &str, query: &str) -> String {
    if query.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{query}")
    }
}

/// The request line the Raw tab starts with.
pub fn request_line(detail: &FlowDetail) -> String {
    let r = &detail.request;
    format!("{} {} {}", r.method, r.url, r.version.as_str())
}

/// The status line the Raw tab starts with.
pub fn status_line(detail: &FlowDetail) -> Option<String> {
    let r = detail.response.as_ref()?;
    Some(format!("{} {} {}", r.version.as_str(), r.status, r.reason))
}

/// Which message panel a flow gets below its messages, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    WebSocket,
    Sse,
}

pub fn stream_kind(flow: &FlowSummary, detail: Option<&FlowDetail>) -> Option<StreamKind> {
    if matches!(flow.scheme, Scheme::Ws | Scheme::Wss) && flow.status == Some(101) {
        return Some(StreamKind::WebSocket);
    }
    let response = detail?.response.as_ref()?;
    let kind = header_value(&response.headers, "content-type")?;
    let mime = kind.split(';').next().unwrap_or("").trim();
    mime.eq_ignore_ascii_case("text/event-stream")
        .then_some(StreamKind::Sse)
}

/// Bodies past this are left out of the Raw tab; the Body tab handles them.
pub const RAW_BODY_MAX: u64 = 1024 * 1024;

/// What the Raw tab puts after the head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawBody {
    /// No body.
    None,
    /// A note in place of the body.
    Note(String),
    /// Fetch the (plain, finished, modest) body, then [`raw_body_text`].
    Fetch,
}

/// Whether the Raw tab can show the body: encoded bodies are binary on the
/// wire, streaming ones aren't finished, and big ones are the Body tab's.
pub fn raw_body(info: &BodyInfo, finished: bool) -> RawBody {
    if info.size == 0 {
        return RawBody::None;
    }
    if let Some(encoding) = &info.encoding {
        return RawBody::Note(format!("[{} {encoding}-encoded body]", format_size(info.size)));
    }
    if !finished {
        return RawBody::Note(format!("[body streaming · {} so far]", format_size(info.size)));
    }
    if info.captured > RAW_BODY_MAX {
        return RawBody::Note(format!("[{} body: see the Body tab]", format_size(info.size)));
    }
    RawBody::Fetch
}

/// A fetched body as the Raw tab shows it: its text, or a note for a
/// binary one.
pub fn raw_body_text(bytes: &[u8], size: u64) -> String {
    match body_text(bytes) {
        Some(text) => text.to_owned(),
        None => format!("[{} binary body]", format_size(size)),
    }
}

/// A body copied from the row menu: its text, or why it can't be.
pub fn copy_body_text(bytes: &[u8], part: BodyPart) -> Result<String, String> {
    body_text(bytes).map(str::to_owned).ok_or_else(|| {
        let part = match part {
            BodyPart::Request => "request",
            BodyPart::Response => "response",
        };
        format!("The {part} body isn't UTF-8 text: copy it as Base64 from the Body tab")
    })
}

/// "Copy <column> <value>": the text of the cell right-clicked, as shown.
/// The URL has its own item; icon columns have none.
pub fn cell_text(column: &Column, row: &FlowRow) -> Option<(String, String)> {
    let FlowRow(
        id,
        state,
        _,
        ref method,
        _,
        _,
        _,
        _,
        status,
        _,
        ref client,
        started_at_ms,
        duration_ms,
        req_size,
        resp_size,
        _,
        http_version,
        ..,
    ) = *row;
    let some = |label: &str, text: String| Some((label.to_owned(), text));
    match &column.id {
        ColumnId::Header(_) => {
            let value = column.header.and_then(|i| row.24.get(i).cloned().flatten())?;
            some(&column.title, value.to_string())
        }
        ColumnId::Builtin(b) => match b {
            Builtin::Id => some("Number", (id as u64 + 1).to_string()),
            Builtin::State => some("State", state_name(state).to_owned()),
            Builtin::Client => client.as_deref().and_then(|c| some("App", c.to_owned())),
            Builtin::Method => some("Method", method.to_string()),
            Builtin::Status => status.and_then(|s| some("Status", s.to_string())),
            Builtin::Time => some("Time", format_time(Some(started_at_ms))),
            Builtin::Duration => duration_ms.and_then(|_| some("Duration", format_duration(duration_ms))),
            Builtin::Req => some("Request size", format_size(req_size)),
            Builtin::Resp => some("Response size", format_size(resp_size)),
            Builtin::Proto => some("Protocol", version_label(http_version).to_owned()),
            Builtin::Notes => row.23.as_deref().and_then(|c| some("Comment", c.to_owned())),
            Builtin::Url | Builtin::Ssl | Builtin::Tools => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columns::TablePrefs;
    use crate::sample;
    use pm_model::{HeaderColumn, HttpVersion};

    #[test]
    fn header_rows_copy_name_value_line_token_and_cookie() {
        let items = row_copies("Authorization", "Bearer abc.def", KvKind::Header);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Copy name", "Copy value", "Copy as “Name: value”", "Copy token"]
        );
        assert_eq!(items[2].text, "Authorization: Bearer abc.def");
        assert_eq!(items[3].text, "abc.def");
        let cookie = row_copies("Set-Cookie", "session=xyz; Path=/", KvKind::Header);
        assert_eq!(
            cookie.last().unwrap(),
            &CopyItem::new("Copy cookie value", "xyz", "cookie session")
        );
        // Several cookies are a submenu, not one item.
        assert_eq!(row_copies("Cookie", "a=1; b=2", KvKind::Header).len(), 3);
        assert_eq!(row_cookies("Cookie", "a=1; b=2", KvKind::Header).len(), 2);
        // Query and form rows have no tokens or cookies.
        let query = row_copies("Cookie", "a b", KvKind::Query);
        assert_eq!(query[2], CopyItem::new("Copy as name=value", "Cookie=a+b", "parameter"));
        assert_eq!(query.len(), 3);
        assert_eq!(row_copies("Issuer", "CN=x", KvKind::Plain)[2].text, "Issuer: CN=x");
    }

    #[test]
    fn whole_tables_copy_by_kind() {
        let rows = [("A", "1"), ("B", "x y")];
        assert_eq!(all_text(&rows, KvKind::Header), "A: 1\nB: x y");
        assert_eq!(all_text(&rows, KvKind::Query), "A=1&B=x+y");
        assert_eq!(all_text(&rows, KvKind::Plain), "A: 1\nB: x y");
        assert_eq!(KvKind::Form.all_label(), "fields");
    }

    #[test]
    fn filters_by_header_and_by_the_flows_facts() {
        assert_eq!(
            header_filter_term(BodyPart::Request, "Accept", "text/html"),
            "reqheader:\"Accept: text/html\""
        );
        assert_eq!(
            header_filter_term(BodyPart::Response, "X-Id", "a\"b"),
            "resheader:\"X-Id: a\""
        );
        let items = filter_by("api.example.com", "GET", Some(404), Some("Google Chrome"));
        let terms: Vec<&str> = items.iter().map(|i| i.term.as_str()).collect();
        assert_eq!(
            terms,
            [
                "host:api.example.com",
                "method:GET",
                "status:404",
                "app:\"Google Chrome\""
            ]
        );
        assert_eq!(filter_by("h", "GET", None, None).len(), 2);
    }

    #[test]
    fn the_url_bar_copies_the_response_status_and_path() {
        let flow = sample::summary();
        let detail = sample::detail();
        assert_eq!(status_text(&flow, Some(&detail)).as_deref(), Some("200 OK"));
        assert_eq!(status_text(&flow, None).as_deref(), Some("200"));
        assert_eq!(path_and_query("/a", "b=1"), "/a?b=1");
        assert_eq!(path_and_query("/a", ""), "/a");
        assert_eq!(
            request_line(&detail),
            "GET https://api.example.com/v1/users?page=2&q=a+b HTTP/2"
        );
        let mut h1 = detail.clone();
        h1.response.as_mut().unwrap().version = HttpVersion::Http11;
        assert_eq!(status_line(&h1).as_deref(), Some("HTTP/1.1 200 OK"));
    }

    #[test]
    fn web_sockets_and_event_streams_get_a_panel() {
        let mut flow = sample::summary();
        let mut detail = sample::detail();
        assert_eq!(stream_kind(&flow, Some(&detail)), None);
        detail.response.as_mut().unwrap().headers =
            vec![sample::header("Content-Type", "text/event-stream; charset=utf-8")];
        assert_eq!(stream_kind(&flow, Some(&detail)), Some(StreamKind::Sse));
        flow.scheme = Scheme::Wss;
        flow.status = Some(101);
        assert_eq!(stream_kind(&flow, None), Some(StreamKind::WebSocket));
    }

    #[test]
    fn the_raw_tab_leaves_out_encoded_streaming_big_and_binary_bodies() {
        let mut info = sample::detail().response_body;
        assert_eq!(raw_body(&info, true), RawBody::Fetch);
        assert_eq!(
            raw_body(&info, false),
            RawBody::Note("[body streaming · 1.5 KB so far]".into())
        );
        info.encoding = Some("gzip".into());
        assert_eq!(
            raw_body(&info, true),
            RawBody::Note("[1.5 KB gzip-encoded body]".into())
        );
        info.encoding = None;
        info.size = 3 * 1024 * 1024;
        info.captured = info.size;
        assert_eq!(
            raw_body(&info, true),
            RawBody::Note("[3.0 MB body: see the Body tab]".into())
        );
        info.size = 0;
        assert_eq!(raw_body(&info, true), RawBody::None);
        assert_eq!(raw_body_text(b"hi", 2), "hi");
        assert_eq!(raw_body_text(&[0, 1, 2], 3), "[3 B binary body]");
        assert!(
            copy_body_text(&[0xff], BodyPart::Response)
                .unwrap_err()
                .contains("Base64")
        );
    }

    #[test]
    fn cells_copy_as_shown() {
        let mut flow = sample::summary();
        flow.comment = Some("check this".into());
        flow.extra = vec![None, Some("req-42".into())];
        let row = sample::row_of(&flow);
        let mut prefs = TablePrefs::default();
        prefs.add_header_column(HeaderColumn {
            part: BodyPart::Request,
            name: "x-a".into(),
        });
        prefs.add_header_column(HeaderColumn {
            part: BodyPart::Response,
            name: "x-request-id".into(),
        });
        let placed = prefs.place();
        let text = |id: &str| {
            let c = placed.iter().find(|c| c.id.to_string() == id).unwrap();
            cell_text(c, &row)
        };
        assert_eq!(text("id"), Some(("Number".into(), "4".into())));
        assert_eq!(text("method"), Some(("Method".into(), "GET".into())));
        assert_eq!(text("status"), Some(("Status".into(), "200".into())));
        assert_eq!(text("duration"), Some(("Duration".into(), "184 ms".into())));
        assert_eq!(text("resp"), Some(("Response size".into(), "1.5 KB".into())));
        assert_eq!(text("proto"), Some(("Protocol".into(), "h2".into())));
        assert_eq!(text("client"), Some(("App".into(), "Safari".into())));
        assert_eq!(text("notes"), Some(("Comment".into(), "check this".into())));
        assert_eq!(text("url"), None);
        assert_eq!(text("hdr:request:x-a"), None, "no value");
        let header = text("hdr:response:x-request-id").unwrap();
        assert_eq!(header.1, "req-42");
    }
}
