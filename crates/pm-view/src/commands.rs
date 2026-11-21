//! The command list of `lib/commands.ts`, without its handlers: ids,
//! titles, palette groups, keywords and keys. The native app registers a
//! handler per id (each feature for its own commands, in its `register`);
//! the ⌘K palette lists the commands that have a handler and apply now, and
//! a command with a key is bound to it.
//!
//! Key rules (`commandForKey`): ⇧ never matches (a binding is exact, so
//! ⇧⌘E isn't ⌘E); ⌥ matches the physical key (gpui matches the key, not the
//! character ⌥ types); bare keys and `in_table` keys are bound only in the
//! flow table; ⌘⌫ and ⌘F are left to text fields and editors, whose own
//! bindings are deeper and win.

use std::borrow::Cow;

/// The palette's groups, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Flows,
    Capture,
    Navigate,
    Windows,
    Settings,
}

impl Group {
    pub const ALL: [Group; 5] = [
        Group::Flows,
        Group::Capture,
        Group::Navigate,
        Group::Windows,
        Group::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::Flows => "Flows",
            Group::Capture => "Capture",
            Group::Navigate => "Navigate",
            Group::Windows => "Windows",
            Group::Settings => "Settings",
        }
    }
}

/// One thing the app can do from the keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: Cow<'static, str>,
    pub title: Cow<'static, str>,
    pub group: Group,
    /// The key pressed with ⌘ (Ctrl elsewhere): a letter, `backspace` or `,`.
    pub key: Option<&'static str>,
    /// With ⌥ as well (⌥⌘R).
    pub alt: bool,
    /// The key alone, without ⌘ (⌫ deletes the selection), in the flow
    /// table only.
    pub bare: bool,
    /// Only while the flow table has the focus (⌘A).
    pub in_table: bool,
    /// Extra words the palette matches on.
    pub keywords: &'static [&'static str],
}

impl Command {
    const fn new(id: &'static str, title: &'static str, group: Group) -> Command {
        Command {
            id: Cow::Borrowed(id),
            title: Cow::Borrowed(title),
            group,
            key: None,
            alt: false,
            bare: false,
            in_table: false,
            keywords: &[],
        }
    }

    fn owned(id: String, title: String, group: Group) -> Command {
        Command {
            id: Cow::Owned(id),
            title: Cow::Owned(title),
            ..Command::new("", "", group)
        }
    }

    const fn key(mut self, key: &'static str) -> Command {
        self.key = Some(key);
        self
    }

    const fn alt(mut self) -> Command {
        self.alt = true;
        self
    }

    const fn bare(mut self) -> Command {
        self.bare = true;
        self
    }

    const fn in_table(mut self) -> Command {
        self.in_table = true;
        self
    }

    const fn keywords(mut self, keywords: &'static [&'static str]) -> Command {
        self.keywords = keywords;
        self
    }

    /// The gpui keystroke it is bound to: `cmd-e`, `alt-cmd-r`, `backspace`.
    pub fn keystroke(&self, mac: bool) -> Option<String> {
        let key = self.key?;
        if self.bare {
            return Some(key.to_owned());
        }
        let primary = if mac { "cmd" } else { "ctrl" };
        Some(if self.alt {
            format!("alt-{primary}-{key}")
        } else {
            format!("{primary}-{key}")
        })
    }

    /// How its key is shown: ⌘R, ⌘⌫, ⌥⌘R, ⌫ (Ctrl+R elsewhere).
    pub fn shortcut(&self, mac: bool) -> Option<String> {
        Some(shortcut_label(self.key?, self.alt, self.bare, mac))
    }
}

/// How a key is shown: `⌘R`, `⌥⌘R`, `⌫`, or `Ctrl+Alt+R` off macOS.
pub fn shortcut_label(key: &str, alt: bool, bare: bool, mac: bool) -> String {
    let name = match key {
        "backspace" if mac => "⌫".to_owned(),
        "backspace" => "Backspace".to_owned(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => k.to_owned(),
    };
    if bare {
        return name;
    }
    match (mac, alt) {
        (true, true) => format!("⌥⌘{name}"),
        (true, false) => format!("⌘{name}"),
        (false, true) => format!("Ctrl+Alt+{name}"),
        (false, false) => format!("Ctrl+{name}"),
    }
}

/// Copy as Code's languages (`CODE_LANGUAGES`), by id and label.
pub const CODE_LANGUAGES: [(&str, &str); 7] = [
    ("fetch", "JavaScript (fetch)"),
    ("python", "Python (requests)"),
    ("httpie", "HTTPie"),
    ("go", "Go (net/http)"),
    ("swift", "Swift (URLSession)"),
    ("kotlin", "Kotlin (OkHttp)"),
    ("powershell", "PowerShell"),
];

/// Label colours, in menu order.
pub const FLOW_LABELS: [&str; 6] = ["red", "orange", "yellow", "green", "blue", "purple"];

/// Intercept targets: id, app name, and whether it is a terminal.
pub const INTERCEPT_TARGETS: [(&str, &str, bool); 7] = [
    ("terminal", "Terminal", true),
    ("iterm", "iTerm", true),
    ("chrome", "Google Chrome", false),
    ("edge", "Microsoft Edge", false),
    ("brave", "Brave Browser", false),
    ("arc", "Arc", false),
    ("firefox", "Firefox", false),
];

/// The Settings window's pages, in its order; the palette opens each one
/// directly. The Tauri palette left out Protobuf and Updates (G12 adds them).
pub const SETTINGS_PAGES: [(&str, &str); 11] = [
    ("proxy", "Proxy"),
    ("network", "Network"),
    ("ssl", "SSL Proxying"),
    ("clients", "Clients"),
    ("storage", "Storage"),
    ("protobuf", "Protobuf"),
    ("mocking", "Mocking"),
    ("appearance", "Appearance"),
    ("advanced", "Advanced"),
    ("mcp", "MCP"),
    ("updates", "Updates"),
];

fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Every command, in the palette's order within each group.
pub fn all() -> Vec<Command> {
    use Group::*;
    let mut out = vec![
        Command::new("replay", "Replay selected flow", Flows).key("r"),
        Command::new("edit-replay", "Edit & Replay selected flow…", Flows).keywords(&["compose", "resend"]),
        Command::new("rewrite", "Rewrite selected flow…", Flows).keywords(&["edit", "modify", "rule", "json"]),
        Command::new("pin", "Pin or unpin selected flow", Flows).keywords(&["keep", "favorite"]),
        Command::new("copy-url", "Copy URL of selected flow", Flows),
    ];
    out.extend(CODE_LANGUAGES.iter().map(|(id, label)| {
        Command::owned(
            format!("copy-code-{id}"),
            format!("Copy selected flow as {label}"),
            Flows,
        )
        .keywords(&["code", "snippet", "export"])
    }));
    out.extend([
        Command::new("repeat", "Repeat selected flow…", Flows)
            .key("r")
            .alt()
            .keywords(&["load", "iterations", "rate limit", "stress"]),
        Command::new("select-all", "Select all flows", Flows)
            .key("a")
            .in_table(),
        Command::new("delete", "Delete selected flows", Flows)
            .key("backspace")
            .bare()
            .keywords(&["remove"]),
        Command::new("replay-selection", "Replay selected flows in order", Flows),
        Command::new("compare", "Compare the two selected flows", Flows).keywords(&["diff"]),
        Command::new("comment", "Comment on selected flow…", Flows).keywords(&["note", "annotate"]),
    ]);
    out.extend(FLOW_LABELS.iter().map(|label| {
        Command::owned(
            format!("label-{label}"),
            format!("Label selected flows {}", capitalised(label)),
            Flows,
        )
        .keywords(&["colour", "color", "tag", "mark"])
    }));
    out.extend([
        Command::new("label-none", "Remove label from selected flows", Flows).keywords(&["colour", "color", "tag"]),
        Command::new("add-header-column", "Add header column…", Flows).keywords(&["column", "header", "table"]),
        Command::new("copy-curl", "Copy selected flow as cURL", Flows),
        Command::new("compose", "New request…", Flows).keywords(&["compose"]),
        Command::new("clear", "Clear all flows", Capture).key("backspace"),
        Command::new("record-start", "Start recording", Capture)
            .key("e")
            .keywords(&["record", "capture", "resume"]),
        Command::new("record-stop", "Stop recording", Capture)
            .key("e")
            .keywords(&["record", "capture", "pause"]),
        Command::new("auto-select", "Toggle Auto Select", Capture).keywords(&["follow", "scroll"]),
    ]);
    out.extend(INTERCEPT_TARGETS.iter().map(|(id, name, terminal)| {
        let title = if *terminal {
            format!("Intercept: New {name} window")
        } else {
            format!("Intercept: Launch {name}")
        };
        let keywords: &'static [&'static str] = if *terminal {
            &["intercept", "launch", "terminal shell node python curl"]
        } else {
            &["intercept", "launch", "browser"]
        };
        Command::owned(format!("intercept-{id}"), title, Capture).keywords(keywords)
    }));
    out.extend([
        Command::new("no-cache", "Toggle No Caching", Capture).keywords(&["cache", "etag", "304", "fresh"]),
        Command::new("no-cookies", "Toggle Block Cookies", Capture).keywords(&["cookie", "set-cookie", "session"]),
        Command::new("autosave-now", "Auto-save now", Capture).keywords(&["backup", "save", "session"]),
        Command::new("open", "Open session, HAR or Charles file…", Capture).keywords(&["chlsj", "chlsx", "import"]),
        Command::new("mock-from-session", "Mock from this session…", Capture).keywords(&[
            "serve",
            "offline",
            "recording",
            "replay",
        ]),
        Command::new("mock-from-file", "Mock from a file…", Capture).keywords(&[
            "serve",
            "offline",
            "recording",
            "har",
            "charles",
        ]),
        Command::new("import-openapi", "Import OpenAPI spec…", Capture).keywords(&[
            "swagger",
            "yaml",
            "mock",
            "operation",
        ]),
        Command::new("save-session", "Save session…", Capture),
        Command::new("export-har", "Export HAR…", Capture),
        Command::new("search", "Search flows", Navigate)
            .key("f")
            .keywords(&["find", "filter", "dsl"]),
        Command::new("filter", "Filter apps and domains", Navigate)
            .key("l")
            .keywords(&["sidebar"]),
        Command::new("pinned", "Show pinned flows", Navigate),
        Command::new("labelled", "Show labelled flows", Navigate).keywords(&["colour", "color", "tag"]),
        Command::new("commented", "Show commented flows", Navigate).keywords(&["note"]),
        Command::new("deep-search", "Search request and response bodies…", Navigate)
            .keywords(&["body", "header", "deep"]),
        Command::new("table", "Focus the flow table", Navigate),
        Command::new("inspector", "Focus the inspector", Navigate),
        Command::new("live", "Show live capture", Navigate),
        Command::new("rules", "Rules…", Windows).keywords(&["map local", "rewrite"]),
        Command::new("scripts", "Scripts…", Windows).keywords(&["javascript", "decode", "base64", "mock", "view"]),
        Command::new("breakpoints", "Breakpoints…", Windows),
        Command::new("devices", "Device setup…", Windows).keywords(&["iphone", "android", "simulator"]),
        Command::new("certificate", "Certificate wizard…", Windows).keywords(&["ca", "trust", "https"]),
        Command::new("settings", "Settings…", Settings).key(","),
    ]);
    out.extend(
        SETTINGS_PAGES
            .iter()
            .map(|(page, label)| Command::owned(format!("settings-{page}"), format!("Settings: {label}"), Settings)),
    );
    out.extend(["system", "light", "dark"].iter().map(|appearance| {
        Command::owned(
            format!("appearance-{appearance}"),
            format!("Appearance: {}", capitalised(appearance)),
            Settings,
        )
        .keywords(&["theme", "mode"])
    }));
    out
}

/// The command with `id`.
pub fn get(id: &str) -> Option<Command> {
    all().into_iter().find(|c| c.id == id)
}

/// Text the search box gets from "Search request and response bodies…":
/// `body:` appended unless the text already has a `body:` term.
pub fn with_body_term(text: &str) -> String {
    // `\bbody:`: not preceded by a word character.
    let has = text.match_indices("body:").any(|(i, _)| {
        !text[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    });
    if has {
        text.to_owned()
    } else if text.is_empty() {
        "body:".into()
    } else {
        format!("{text} body:")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn ids_are_unique_and_titles_match_the_tauri_list() {
        let commands = all();
        let ids: HashSet<_> = commands.iter().map(|c| c.id.clone()).collect();
        assert_eq!(ids.len(), commands.len());
        assert_eq!(commands.len(), 77);
        let title = |id: &str| get(id).unwrap().title.into_owned();
        assert_eq!(title("copy-code-go"), "Copy selected flow as Go (net/http)");
        assert_eq!(title("label-purple"), "Label selected flows Purple");
        assert_eq!(title("intercept-iterm"), "Intercept: New iTerm window");
        assert_eq!(title("intercept-arc"), "Intercept: Launch Arc");
        assert_eq!(title("settings-ssl"), "Settings: SSL Proxying");
        assert_eq!(title("appearance-dark"), "Appearance: Dark");
    }

    #[test]
    fn only_mutually_exclusive_commands_share_a_key() {
        let commands = all();
        let mut seen: Vec<(String, &str)> = Vec::new();
        for c in &commands {
            let Some(stroke) = c.keystroke(true) else { continue };
            if let Some((_, other)) = seen.iter().find(|(s, _)| *s == stroke) {
                // ⌘E starts or stops recording, whichever applies.
                let pair = [*other, c.id.as_ref()];
                assert!(
                    pair.contains(&"record-start") && pair.contains(&"record-stop"),
                    "{stroke}"
                );
            }
            seen.push((stroke, c.id.as_ref()));
        }
    }

    #[test]
    fn keys_bind_and_show_like_the_tauri_app() {
        let k = |id: &str| get(id).unwrap();
        assert_eq!(k("record-stop").keystroke(true).as_deref(), Some("cmd-e"));
        assert_eq!(k("repeat").keystroke(true).as_deref(), Some("alt-cmd-r"));
        assert_eq!(k("repeat").keystroke(false).as_deref(), Some("alt-ctrl-r"));
        assert_eq!(k("delete").keystroke(true).as_deref(), Some("backspace"));
        assert_eq!(k("clear").keystroke(true).as_deref(), Some("cmd-backspace"));
        assert_eq!(k("open").keystroke(true), None);
        assert_eq!(k("replay").shortcut(true).as_deref(), Some("⌘R"));
        assert_eq!(k("repeat").shortcut(true).as_deref(), Some("⌥⌘R"));
        assert_eq!(k("clear").shortcut(true).as_deref(), Some("⌘⌫"));
        assert_eq!(k("delete").shortcut(true).as_deref(), Some("⌫"));
        assert_eq!(k("settings").shortcut(true).as_deref(), Some("⌘,"));
        assert_eq!(k("repeat").shortcut(false).as_deref(), Some("Ctrl+Alt+R"));
        assert_eq!(k("clear").shortcut(false).as_deref(), Some("Ctrl+Backspace"));
        assert!(k("select-all").in_table && k("delete").bare);
    }

    #[test]
    fn groups_keep_their_order() {
        let groups: Vec<_> = Group::ALL.iter().map(|g| g.label()).collect();
        assert_eq!(groups, ["Flows", "Capture", "Navigate", "Windows", "Settings"]);
    }

    #[test]
    fn the_body_search_command_appends_body_once() {
        assert_eq!(with_body_term(""), "body:");
        assert_eq!(with_body_term("host:a"), "host:a body:");
        assert_eq!(with_body_term("body:x"), "body:x");
        assert_eq!(with_body_term("x resbody:y"), "x resbody:y body:");
    }
}
