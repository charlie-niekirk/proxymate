//! GraphQL operations from a request body or a GET query string
//! (`lib/body/graphql.ts`), read as [`JNode`]s so variables keep their
//! member order and exact numbers.

use std::sync::LazyLock;

use regex::Regex;

use super::json_doc::{self, JNode};

#[derive(Clone, Debug, PartialEq)]
pub struct Operation {
    pub operation_name: Option<String>,
    /// `query`, `mutation`, `subscription`, or `None` for a persisted query.
    pub kind: Option<String>,
    pub query: String,
    /// As sent; `None` when absent or `null`.
    pub variables: Option<JNode>,
    pub extensions: Option<JNode>,
}

impl Operation {
    /// Pretty JSON of the variables, as sent.
    pub fn variables_text(&self) -> Option<String> {
        self.variables.as_ref().map(|v| v.to_json(2))
    }

    pub fn extensions_text(&self) -> Option<String> {
        self.extensions.as_ref().map(|v| v.to_json(2))
    }
}

static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#[^\n]*").expect("valid"));
static KIND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(query|mutation|subscription)\b").expect("valid"));
static NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:query|mutation|subscription)\s+([_A-Za-z]\w*)").expect("valid"));

fn operation_kind(query: &str) -> Option<String> {
    let q = COMMENT.replace_all(query, "");
    let q = q.trim_start();
    if q.starts_with('{') {
        return Some("query".into());
    }
    KIND.captures(q).map(|c| c[1].to_owned())
}

fn name_in(query: &str) -> Option<String> {
    NAME.captures(query).map(|c| c[1].to_owned())
}

fn member<'a>(node: &'a JNode, key: &str) -> Option<&'a JNode> {
    node.member(key).filter(|v| **v != JNode::Null)
}

fn from_node(node: &JNode) -> Option<Operation> {
    if !matches!(node, JNode::Obj(_)) {
        return None;
    }
    let q = member(node, "query");
    let extensions = member(node, "extensions").cloned();
    let query = match q {
        Some(JNode::Str(s)) => s.clone(),
        _ if extensions.is_some() => String::new(),
        _ => return None,
    };
    let name = match member(node, "operationName") {
        Some(JNode::Str(s)) if !s.is_empty() => Some(s.clone()),
        _ => name_in(&query),
    };
    Some(Operation {
        operation_name: name,
        kind: operation_kind(&query),
        variables: member(node, "variables").cloned(),
        extensions,
        query,
    })
}

/// The operations in a body (one, or a batch), or `None` if it isn't GraphQL.
pub fn parse_body(text: &str, content_type: Option<&str>) -> Option<Vec<Operation>> {
    let raw_document = content_type.is_some_and(|ct| ct.to_ascii_lowercase().starts_with("application/graphql"));
    if raw_document && !text.trim_start().starts_with("{\"") {
        return Some(vec![Operation {
            operation_name: name_in(text),
            kind: operation_kind(text),
            query: text.to_owned(),
            variables: None,
            extensions: None,
        }]);
    }
    let value = json_doc::parse(text).ok()?;
    let nodes: Vec<&JNode> = match &value {
        JNode::Arr(items) => items.iter().collect(),
        other => vec![other],
    };
    let ops: Option<Vec<Operation>> = nodes.into_iter().map(from_node).collect();
    ops.filter(|ops| !ops.is_empty())
}

/// A GET operation from `?query=…&variables=…&operationName=…`.
pub fn parse_query_string(url: &str) -> Option<Operation> {
    let pairs = crate::http::query_pairs(url);
    let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let query = get("query")?;
    let json = |key: &str| -> Option<JNode> {
        let raw = get(key)?;
        match json_doc::parse(&raw) {
            Ok(JNode::Null) => None,
            Ok(node) => Some(node),
            Err(_) => Some(JNode::Str(raw)),
        }
    };
    Some(Operation {
        operation_name: get("operationName").filter(|n| !n.is_empty()).or_else(|| name_in(&query)),
        kind: operation_kind(&query),
        variables: json("variables"),
        extensions: json("extensions"),
        query,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_json_envelope() {
        let ops = parse_body(
            r#"{"query":"query GetUser($id: ID!) { user(id: $id) { name } }","variables":{"id":"1"}}"#,
            Some("application/json"),
        )
        .unwrap();
        assert_eq!(ops[0].operation_name.as_deref(), Some("GetUser"));
        assert_eq!(ops[0].kind.as_deref(), Some("query"));
        assert_eq!(ops[0].extensions, None);
        assert_eq!(ops[0].variables_text().unwrap(), "{\n  \"id\": \"1\"\n}");
    }

    #[test]
    fn keeps_variables_as_sent() {
        let ops = parse_body(r#"{"query":"{ a }","variables":{"z":1,"2":12345678901234567890,"a":1.50}}"#, None).unwrap();
        assert_eq!(
            ops[0].variables_text().unwrap(),
            "{\n  \"z\": 1,\n  \"2\": 12345678901234567890,\n  \"a\": 1.50\n}"
        );
    }

    #[test]
    fn reads_batches_raw_documents_and_persisted_queries() {
        let ops = parse_body(r#"[{"query":"mutation A { a }"},{"query":"{ b }"}]"#, None).unwrap();
        let names: Vec<_> = ops.iter().map(|o| (o.operation_name.clone(), o.kind.clone())).collect();
        assert_eq!(
            names,
            [
                (Some("A".into()), Some("mutation".into())),
                (None, Some("query".into()))
            ]
        );
        let raw = parse_body("subscription S { s }", Some("application/graphql")).unwrap();
        assert_eq!(raw[0].operation_name.as_deref(), Some("S"));
        let persisted =
            parse_body(r#"{"operationName":"P","extensions":{"persistedQuery":{"version":1}}}"#, None).unwrap();
        assert_eq!(persisted[0].operation_name.as_deref(), Some("P"));
        assert_eq!(persisted[0].query, "");
        assert_eq!(persisted[0].kind, None);
    }

    #[test]
    fn rejects_other_json() {
        assert!(parse_body(r#"{"q":1}"#, Some("application/json")).is_none());
        assert!(parse_body("not json", Some("application/json")).is_none());
    }

    #[test]
    fn reads_get_operations() {
        let op = parse_query_string(
            "https://x/graphql?query=%7B%20me%20%7B%20id%20%7D%20%7D&variables=%7B%22a%22%3A1%7D&operationName=Me",
        )
        .unwrap();
        assert_eq!(op.operation_name.as_deref(), Some("Me"));
        assert_eq!(op.query, "{ me { id } }");
        assert_eq!(op.variables_text().unwrap(), "{\n  \"a\": 1\n}");
        assert!(parse_query_string("https://x/graphql").is_none());
    }
}
