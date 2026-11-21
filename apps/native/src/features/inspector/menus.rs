//! The inspector's context menus, built by plain functions from what they
//! act on (G2's rule): the URL bar's, a name/value table's (keyed by the
//! row right-clicked), a rule badge's, a certificate's, the Timing tab's,
//! the failure banner's and the Raw tab's. `tests/inspector.rs` runs their
//! entries against the test clipboard; the dev remote's `contextMenu`
//! reads them.

use pm_model::{AppliedRule, BodyPart, CertSummary, FlowDetail, FlowSummary, Phase};
use pm_view::copy_formats::{pairs_json, table_text};
use pm_view::errors::FailureHelp;
use pm_view::inspector::{self as view, KvKind};

use super::actions::{AddFilterTerm, CopyFlowCurl, DisableRule, FlowRef};
use crate::features::chrome::OpenRule;
use crate::features::context_menu::Menu;

/// "Filter by ▸ Host, Method, Status, App" (the URL bar and the flow menu).
pub fn filter_by(host: &str, method: &str, status: Option<u16>, client: Option<&str>) -> Menu {
    view::filter_by(host, method, status, client)
        .into_iter()
        .fold(Menu::new(), |menu, item| {
            menu.item(
                format!("{}: {}", item.label, item.value),
                AddFilterTerm { term: item.term.into() },
            )
        })
}

/// The URL bar's menu: the URL and its parts, method, status, cURL, and
/// filters for this flow's host, method, status and app.
pub fn url_bar(store: pm_model::StoreId, flow: &FlowSummary, detail: Option<&FlowDetail>) -> Menu {
    let url = detail.map_or_else(
        || pm_view::flow::flow_url(flow.scheme, &flow.host, flow.port, &flow.path, &flow.query),
        |d| d.request.url.clone(),
    );
    let status = view::status_text(flow, detail);
    let connect = &*flow.method == "CONNECT";
    Menu::new()
        .copy("Copy URL", url, "URL")
        .copy("Copy host", flow.host.to_string(), "host")
        .copy("Copy path", flow.path.clone(), "path")
        .when(!flow.query.is_empty(), |m| {
            m.copy(
                "Copy path and query",
                view::path_and_query(&flow.path, &flow.query),
                "path and query",
            )
        })
        .copy("Copy method", flow.method.to_string(), "method")
        .when(status.is_some(), |m| {
            m.copy("Copy status", status.unwrap_or_default(), "status")
        })
        .when(!connect, |m| {
            m.item(
                "Copy as cURL",
                CopyFlowCurl {
                    flow: FlowRef { store, id: flow.id },
                },
            )
        })
        .separator()
        .submenu(
            "Filter by",
            filter_by(&flow.host, &flow.method, flow.status, flow.client.as_deref()),
        )
}

/// A name/value table's menu (`KvMenu`): the text selected, the row's
/// items (with cookies, a token and a filter for headers) when a row was
/// right-clicked, then the whole table. `part` is the message headers
/// belong to, so "Filter flows" searches that side.
pub fn kv(
    rows: &[(String, String)],
    kind: KvKind,
    part: Option<BodyPart>,
    index: Option<usize>,
    selected: Option<String>,
) -> Menu {
    let mut menu = Menu::new().selection(selected);
    if let Some((name, value)) = index.and_then(|i| rows.get(i)) {
        for item in view::row_copies(name, value, kind) {
            menu = menu.copy(item.label, item.text, item.what);
        }
        let cookies = view::row_cookies(name, value, kind);
        if cookies.len() > 1 {
            let sub = cookies.into_iter().fold(Menu::new(), |m, c| {
                let what = format!("cookie {}", c.name);
                m.copy(c.name, c.value, what)
            });
            menu = menu.submenu("Copy cookie", sub);
        }
        if let (KvKind::Header, Some(part)) = (kind, part) {
            menu = menu.separator().item(
                "Filter flows with this header",
                AddFilterTerm {
                    term: view::header_filter_term(part, name, value).into(),
                },
            );
        }
        menu = menu.separator();
    }
    let all = kind.all_label();
    menu.copy(format!("Copy all {all}"), view::all_text(rows, kind), all)
        .copy("Copy all as JSON", pairs_json(rows), format!("{all} as JSON"))
}

/// A rule badge's menu: its name and id, Open rule (G7's window) and
/// Disable rule.
pub fn rule_badge(rule: &AppliedRule) -> Menu {
    Menu::new()
        .copy("Copy rule name", rule.name.clone(), "rule name")
        .copy("Copy rule id", rule.rule_id.to_string(), "rule id")
        .separator()
        .item(
            "Open rule",
            OpenRule {
                id: rule.rule_id.to_string().into(),
            },
        )
        .item(
            "Disable rule",
            DisableRule {
                id: rule.rule_id.to_string().into(),
            },
        )
}

/// What a rule badge says after the name: which message it changed.
pub fn phase_short(phase: Phase) -> &'static str {
    match phase {
        Phase::Request => "req",
        Phase::Response => "resp",
    }
}

/// A certificate card's menu.
pub fn certificate(cert: &CertSummary) -> Menu {
    Menu::new()
        .copy("Copy subject", cert.subject.clone(), "subject")
        .copy("Copy SHA-256", cert.sha256.clone(), "fingerprint")
        .copy(
            "Copy SHA-256 without colons",
            cert.sha256.replace(':', ""),
            "fingerprint",
        )
        .copy("Copy serial", cert.serial.clone(), "serial")
        .when(!cert.san.is_empty(), |m| {
            m.copy("Copy names", cert.san.join("\n"), "names")
        })
}

/// The Timing tab's menu: the phase right-clicked, and the whole table
/// tab-separated (for a spreadsheet).
pub fn timing(table: &[(String, String)], index: Option<usize>) -> Menu {
    let menu = match index.and_then(|i| table.get(i)) {
        Some((label, value)) => Menu::new().copy(format!("Copy {label}"), value.clone(), label.clone()),
        None => Menu::new(),
    };
    menu.copy("Copy timing as table", table_text(table), "timing")
}

/// The failure banner's menu.
pub fn failure(help: &FailureHelp) -> Menu {
    Menu::new().copy("Copy error", help.detail.clone(), "error").copy(
        "Copy title and error",
        format!("{}\n{}", help.title, help.detail),
        "error",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::{RuleOrigin, Scheme};
    use pm_view::sample;

    #[test]
    fn the_url_bar_copies_the_url_its_parts_and_filters() {
        let flow = sample::summary();
        let detail = sample::detail();
        let menu = url_bar(0, &flow, Some(&detail));
        assert_eq!(
            menu.labels(),
            [
                "Copy URL",
                "Copy host",
                "Copy path",
                "Copy path and query",
                "Copy method",
                "Copy status",
                "Copy as cURL",
                "—",
                "Filter by"
            ]
        );
        assert_eq!(
            menu.copy_text("Copy URL"),
            Some("https://api.example.com/v1/users?page=2&q=a+b")
        );
        assert_eq!(menu.copy_text("Copy path and query"), Some("/v1/users?page=2&q=a+b"));
        assert_eq!(menu.copy_text("Copy status"), Some("200 OK"));
        assert!(menu.find("Filter by › App: Safari").is_some());
        let mut connect = flow.clone();
        connect.method = "CONNECT".into();
        connect.query.clear();
        connect.scheme = Scheme::Https;
        connect.status = None;
        let menu = url_bar(0, &connect, None);
        assert!(menu.find("Copy as cURL").is_none());
        assert!(menu.find("Copy path and query").is_none());
        assert!(menu.find("Copy status").is_none());
        assert_eq!(menu.copy_text("Copy URL"), Some("https://api.example.com/v1/users"));
    }

    #[test]
    fn a_header_row_has_its_items_then_the_table() {
        let rows = pm_view::http::header_rows(&sample::detail().request.headers);
        let auth = rows.iter().position(|(n, _)| n == "Authorization").unwrap();
        let menu = kv(&rows, KvKind::Header, Some(BodyPart::Request), Some(auth), None);
        assert_eq!(
            menu.labels(),
            [
                "Copy name",
                "Copy value",
                "Copy as “Name: value”",
                "Copy token",
                "—",
                "Filter flows with this header",
                "—",
                "Copy all headers",
                "Copy all as JSON"
            ]
        );
        assert_eq!(menu.copy_text("Copy token"), Some("eyJhbGciOi.payload.sig"));
        let cookie = rows.iter().position(|(n, _)| n == "Cookie").unwrap();
        let menu = kv(
            &rows,
            KvKind::Header,
            Some(BodyPart::Request),
            Some(cookie),
            Some("sel".into()),
        );
        assert_eq!(menu.labels()[0], "Copy");
        assert_eq!(menu.copy_text("Copy cookie › theme"), Some("dark"));
        // Outside the rows: the table's items only.
        let menu = kv(&rows, KvKind::Plain, None, None, None);
        assert_eq!(menu.labels(), ["Copy all rows", "Copy all as JSON"]);
    }

    #[test]
    fn badges_certificates_timings_and_failures_copy_their_text() {
        let rule = AppliedRule {
            rule_id: "r1".into(),
            name: "Mock users".into(),
            origin: RuleOrigin::User,
            phase: Phase::Response,
        };
        let menu = rule_badge(&rule);
        assert_eq!(menu.copy_text("Copy rule id"), Some("r1"));
        assert!(menu.is_enabled("Disable rule"));
        let cert = &sample::detail().tls.unwrap().chain[0];
        let menu = certificate(cert);
        assert_eq!(menu.copy_text("Copy SHA-256 without colons"), Some("ABCDEF01"));
        assert_eq!(menu.copy_text("Copy names"), Some("api.example.com\n*.example.com"));
        let table = vec![
            ("DNS".to_owned(), "3.2 ms".to_owned()),
            ("Total".into(), "184 ms".into()),
        ];
        let menu = timing(&table, Some(0));
        assert_eq!(menu.copy_text("Copy DNS"), Some("3.2 ms"));
        assert_eq!(
            menu.copy_text("Copy timing as table"),
            Some("DNS\t3.2 ms\nTotal\t184 ms")
        );
        let help = FailureHelp {
            title: "The flow failed".into(),
            detail: "boom".into(),
            actions: vec![],
        };
        assert_eq!(
            failure(&help).copy_text("Copy title and error"),
            Some("The flow failed\nboom")
        );
    }
}
