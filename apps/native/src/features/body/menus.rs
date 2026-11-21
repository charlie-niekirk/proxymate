//! The body viewers' menus as data (`menus/body.tsx`, `CodeView.tsx`,
//! `HexView.tsx`, `JsonViews.tsx`, `MultipartView.tsx`, `GraphQlView.tsx`):
//! each ends with the body items (Copy body, Copy body as Base64, Save
//! body…), as `BodyItems` ends every viewer's menu in the Tauri UI.

use gpui_kit::SharedString;
use pm_model::BodyPart;
use pm_view::body::graphql::Operation;
use pm_view::body::json_tree::TreeRow;
use pm_view::body::multipart::Part;
use pm_view::copy_formats::{HEX_WIDTH, base64, hex_bytes, hex_dump, hex_row};

use super::{CopyBody, FindInBody, SaveBody, SelectAllInBody};
use crate::features::context_menu::Menu;
use crate::features::inspector::actions::FlowRef;
use pm_widgets::json_tree::{CollapseAll, ExpandAllBelow};

/// The body a menu ends with.
#[derive(Clone, Debug)]
pub struct BodyRef {
    pub flow: FlowRef,
    pub part: BodyPart,
    /// A NUL near the start: "Copy body" is off.
    pub binary: bool,
    pub empty: bool,
    pub file_name: SharedString,
}

/// Copy body, Copy body as Base64, Save body…, after a separator.
pub fn body_items(menu: Menu, body: Option<&BodyRef>) -> Menu {
    let Some(b) = body.filter(|b| !b.empty) else {
        return menu;
    };
    let copy = CopyBody {
        flow: b.flow,
        part: b.part,
        base64: false,
    };
    let menu = menu.separator();
    let menu = if b.binary {
        menu.disabled("Copy body", copy)
    } else {
        menu.item("Copy body", copy)
    };
    menu.item(
        "Copy body as Base64",
        CopyBody {
            flow: b.flow,
            part: b.part,
            base64: true,
        },
    )
    .item(
        "Save body…",
        SaveBody {
            flow: b.flow,
            part: b.part,
            file_name: b.file_name.clone(),
        },
    )
}

/// The menu of viewers without one of their own: the selection, then the
/// body items.
pub fn fallback(selected: Option<String>, body: Option<&BodyRef>) -> Menu {
    body_items(Menu::new().selection(selected), body)
}

/// The code menu: Copy (selection), Copy all, Select all, Find…, a viewer's
/// own items, then the body items.
pub fn code(selected: Option<String>, text: SharedString, extra: Menu, body: Option<&BodyRef>) -> Menu {
    let menu = Menu::new()
        .selection(selected)
        .copy("Copy all", text, "text")
        .item("Select all", SelectAllInBody)
        .item("Find…", FindInBody)
        .separator()
        .append(extra);
    body_items(menu, body)
}

/// The hex menu: the byte and row under the pointer, the whole dump.
pub fn hex(bytes: &[u8], row: Option<usize>, offset: Option<usize>, selected: Option<String>, body: Option<&BodyRef>) -> Menu {
    let mut menu = Menu::new().selection(selected);
    if let Some(row) = row.filter(|r| r * HEX_WIDTH < bytes.len()) {
        if let Some((offset, byte)) = offset.and_then(|o| Some((o, *bytes.get(o)?))) {
            menu = menu.copy(format!("Copy byte 0x{offset:x}"), format!("{byte:02x}"), "byte");
        }
        let start = row * HEX_WIDTH;
        let slice = &bytes[start..(start + HEX_WIDTH).min(bytes.len())];
        menu = menu
            .copy("Copy row as hex", hex_bytes(slice), "row as hex")
            .copy("Copy row as text", hex_row(bytes, row).ascii, "row as text")
            .copy("Copy offset", format!("0x{start:x}"), "offset")
            .separator();
    }
    body_items(menu.copy("Copy as hex dump", hex_dump(bytes), "hex dump"), body)
}

/// The JSON tree's menu for the row under the pointer.
pub fn tree(row: Option<&TreeRow>, value: Option<String>, selected: Option<String>, body: Option<&BodyRef>) -> Menu {
    let mut menu = Menu::new().selection(selected);
    if let Some(row) = row {
        if let Some(value) = value {
            menu = menu.copy("Copy value", value, "value");
        }
        if let Some(key) = row.key_text() {
            menu = menu.copy("Copy key", key, "key");
        }
        menu = menu
            .copy("Copy path", row.json_path(), "path")
            .copy("Copy JSON pointer", row.pointer_text(), "JSON pointer");
        if row.is_container() {
            menu = menu
                .separator()
                .item(
                    "Expand all below",
                    ExpandAllBelow {
                        ptr: row.ptr.clone().into(),
                    },
                )
                .item("Collapse all", CollapseAll);
        }
    }
    body_items(menu, body)
}

/// A multipart part's card: its name, file name and type, then its body.
pub fn part(part: &Part, mime: &str, bytes: &[u8]) -> Menu {
    let body = &bytes[part.body.clone()];
    let mut menu = Menu::new();
    if let Some(name) = &part.name {
        menu = menu.copy("Copy name", name.clone(), "part name");
    }
    if let Some(file) = &part.filename {
        menu = menu.copy("Copy file name", file.clone(), "file name");
    }
    menu = menu.copy(
        "Copy content type",
        part.content_type.clone().unwrap_or_else(|| mime.to_owned()),
        "content type",
    );
    if body.is_empty() {
        return menu;
    }
    let menu = menu.separator();
    let menu = match pm_view::body::text_of(body) {
        Some(text) => menu.copy("Copy part body", text.to_owned(), "part body"),
        None => menu.disabled("Copy part body", SelectAllInBody),
    };
    menu.copy("Copy part body as Base64", base64(body), "part body as Base64")
}

/// The GraphQL operation's name, query and variables.
pub fn graphql(op: &Operation) -> Menu {
    let mut menu = Menu::new();
    if let Some(name) = &op.operation_name {
        menu = menu.copy("Copy operation name", name.clone(), "operation name");
    }
    if !op.query.is_empty() {
        menu = menu.copy("Copy query", op.query.clone(), "query");
    }
    if let Some(vars) = op.variables_text() {
        menu = menu.copy("Copy variables", vars, "variables");
    }
    menu
}

/// The Raw tab: the code menu, with the start line and headers before the
/// body items.
pub fn raw(
    part: BodyPart,
    start_line: &str,
    headers: &[pm_model::Header],
    text: SharedString,
    selected: Option<String>,
) -> Menu {
    let line = match part {
        BodyPart::Request => "request line",
        BodyPart::Response => "status line",
    };
    let extra = Menu::new()
        .copy(format!("Copy {line}"), start_line.to_owned(), line)
        .copy(
            "Copy headers",
            pm_view::copy_formats::headers_block(&pm_view::http::header_rows(headers)),
            "headers",
        );
    code(selected, text, extra, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_view::body::json_doc::parse;
    use pm_view::body::json_tree::{flatten, initial_expansion, roots};

    fn body(binary: bool) -> BodyRef {
        BodyRef {
            flow: FlowRef { store: 0, id: 1 },
            part: BodyPart::Response,
            binary,
            empty: false,
            file_name: "users.json".into(),
        }
    }

    #[test]
    fn every_menu_ends_with_the_body_items() {
        let menu = code(Some("sel".into()), "all text".into(), Menu::new(), Some(&body(false)));
        assert_eq!(
            menu.labels(),
            [
                "Copy",
                "—",
                "Copy all",
                "Select all",
                "Find…",
                "—",
                "Copy body",
                "Copy body as Base64",
                "Save body…"
            ]
        );
        assert_eq!(menu.copy_text("Copy all"), Some("all text"));
        let menu = fallback(None, Some(&body(true)));
        assert_eq!(menu.labels(), ["Copy body", "Copy body as Base64", "Save body…"]);
        assert!(!menu.is_enabled("Copy body"), "binary bodies copy as Base64 only");
        assert!(fallback(None, None).is_empty());
    }

    #[test]
    fn hex_menu_copies_the_byte_row_and_dump() {
        let bytes: Vec<u8> = (0..40).collect();
        let menu = hex(&bytes, Some(1), Some(17), None, None);
        assert_eq!(menu.copy_text("Copy byte 0x11"), Some("11"));
        assert_eq!(
            menu.copy_text("Copy row as hex"),
            Some("10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f")
        );
        assert_eq!(menu.copy_text("Copy offset"), Some("0x10"));
        assert!(menu.copy_text("Copy as hex dump").unwrap().starts_with("00000000  00 01 02"));
        let blank = hex(&bytes, None, None, None, None);
        assert_eq!(blank.labels(), ["Copy as hex dump"]);
    }

    #[test]
    fn tree_menu_copies_value_key_path_and_pointer() {
        let doc = parse(r#"{"user":{"id":12345678901234567890,"tags":["a/b"]}}"#).unwrap();
        let (r, _) = roots(&doc, "");
        let mut expanded = initial_expansion(&doc);
        expanded.insert("/user/tags".into());
        let rows = flatten(&r, &expanded);
        let id = rows.iter().find(|r| r.ptr == "/user/id").unwrap();
        let menu = tree(Some(id), Some("12345678901234567890".into()), None, None);
        assert_eq!(menu.copy_text("Copy value"), Some("12345678901234567890"));
        assert_eq!(menu.copy_text("Copy key"), Some("id"));
        assert_eq!(menu.copy_text("Copy path"), Some("$.user.id"));
        assert_eq!(menu.copy_text("Copy JSON pointer"), Some("/user/id"));
        assert!(menu.find("Expand all below").is_none());
        let tags = rows.iter().find(|r| r.ptr == "/user/tags").unwrap();
        let menu = tree(Some(tags), None, None, None);
        assert!(menu.is_enabled("Expand all below") && menu.is_enabled("Collapse all"));
        assert_eq!(menu.copy_text("Copy JSON pointer"), Some("/user/tags"));
    }

    #[test]
    fn part_and_graphql_menus() {
        let body = b"--b\r\nContent-Disposition: form-data; name=\"f\"; filename=\"a.txt\"\r\n\r\nhi\r\n--b--";
        let parts = pm_view::body::multipart::parse_multipart(body, "b");
        let menu = part(&parts[0], "text/plain", body);
        assert_eq!(menu.copy_text("Copy name"), Some("f"));
        assert_eq!(menu.copy_text("Copy file name"), Some("a.txt"));
        assert_eq!(menu.copy_text("Copy content type"), Some("text/plain"));
        assert_eq!(menu.copy_text("Copy part body"), Some("hi"));
        let ops = pm_view::body::graphql::parse_body(r#"{"query":"query Q { a }","variables":{"x":1}}"#, None).unwrap();
        let menu = graphql(&ops[0]);
        assert_eq!(menu.labels(), ["Copy operation name", "Copy query", "Copy variables"]);
        assert_eq!(menu.copy_text("Copy variables"), Some("{\n  \"x\": 1\n}"));
    }
}
