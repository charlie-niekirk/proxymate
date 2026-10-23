//! Scripts as `.js` files: the source, after a one-line metadata header
//!
//! ```text
//! /* proxymate: {"name":"Decode token","kind":"view","enabled":true,"view":{…}} */
//! ```
//!
//! A file without the header imports too: its kind comes from what it
//! exports and its name from the file name.

use pm_model::{RuleOrigin, Script, ScriptKind, ViewConfig};
use serde::{Deserialize, Serialize};

use super::{Call, Code, Output, ScriptError};
use crate::{Engine, Error};

const HEADER: &str = "/* proxymate: ";

#[derive(Serialize, Deserialize)]
struct Meta {
    name: String,
    kind: ScriptKind,
    #[serde(default = "enabled")]
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    view: Option<ViewConfig>,
}

fn enabled() -> bool {
    true
}

/// The `.js` text of a script.
pub(crate) fn to_file(script: &Script) -> String {
    let meta = Meta {
        name: script.name.clone(),
        kind: script.kind,
        enabled: script.enabled,
        view: script.view.clone(),
    };
    // `*/` can only occur inside a JSON string, where `\/` means `/`.
    let json = serde_json::to_string(&meta).unwrap_or_default().replace("*/", "*\\/");
    format!("{HEADER}{json} */\n{}", script.source)
}

/// Splits a `.js` file into its metadata (if it has the header) and source.
fn from_file(text: &str) -> Result<(Option<Meta>, String), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text.trim_start().strip_prefix(HEADER) else {
        return Ok((None, text.to_owned()));
    };
    let end = rest.find("*/").ok_or("the proxymate header isn't closed with */")?;
    let meta: Meta =
        serde_json::from_str(rest[..end].trim()).map_err(|e| format!("the proxymate header isn't valid: {e}"))?;
    let source = rest[end + 2..].strip_prefix('\n').unwrap_or(&rest[end + 2..]);
    Ok((Some(meta), source.to_owned()))
}

impl Engine {
    /// A script as a `.js` file with a metadata header.
    pub fn export_script(&self, id: &str) -> Result<String, Error> {
        let script = self.script(id).ok_or_else(|| ScriptError::Unknown(id.to_owned()))?;
        Ok(to_file(&script))
    }

    /// Saves a script from a `.js` file. Without a metadata header the kind
    /// comes from its exports and the name from `file_name`.
    pub fn import_script(&self, text: &str, file_name: Option<&str>, origin: RuleOrigin) -> Result<Script, Error> {
        let (meta, source) = from_file(text).map_err(|message| ScriptError::Invalid {
            name: file_name.unwrap_or("script").to_owned(),
            message,
        })?;
        let meta = match meta {
            Some(meta) => meta,
            None => {
                let name = file_name
                    .map(|n| n.trim_end_matches(".js").trim_end_matches(".mjs").to_owned())
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| "Imported script".into());
                let code = Code::new(String::new(), name.clone(), source.as_str().into());
                let exports = match self
                    .scripts
                    .host
                    .run_blocking(code, Call::Inspect, self.scripts.traffic_timeout())
                    .output
                {
                    Ok(Output::Exports(exports)) => exports,
                    Ok(_) => Default::default(),
                    Err(message) => return Err(ScriptError::Invalid { name, message }.into()),
                };
                Meta {
                    name,
                    kind: if exports.on_view {
                        ScriptKind::View
                    } else if exports.on_mock {
                        ScriptKind::Mock
                    } else {
                        ScriptKind::Rewrite
                    },
                    enabled: true,
                    view: None,
                }
            }
        };
        self.create_script(Script {
            id: String::new(),
            name: meta.name,
            kind: meta.kind,
            enabled: meta.enabled,
            source,
            origin,
            view: match meta.kind {
                ScriptKind::View => Some(meta.view.unwrap_or_default()),
                _ => None,
            },
            updated_at_ms: 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_round_trips_and_survives_comment_ends() {
        let script = Script {
            id: "x".into(),
            name: "odd */ name".into(),
            kind: ScriptKind::View,
            enabled: false,
            source: "export function onView() { return []; }\n".into(),
            origin: RuleOrigin::User,
            view: Some(ViewConfig::default()),
            updated_at_ms: 0.0,
        };
        let text = to_file(&script);
        assert!(text.starts_with("/* proxymate: {"));
        assert_eq!(text.lines().count(), 2);
        let (meta, source) = from_file(&text).unwrap();
        let meta = meta.unwrap();
        assert_eq!(
            (meta.name.as_str(), meta.kind, meta.enabled),
            ("odd */ name", ScriptKind::View, false)
        );
        assert_eq!(source, script.source);
    }

    #[test]
    fn files_without_a_header_are_all_source() {
        let (meta, source) = from_file("export function onRequest(c, u, r) { return r; }").unwrap();
        assert!(meta.is_none());
        assert!(source.starts_with("export"));
        assert!(from_file("/* proxymate: {\"name\":1 */").is_err());
    }
}
