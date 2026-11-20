//! The port of `apps/desktop/ui/src/styles/contrast.test.ts`, run on the
//! same file. Every text colour token must reach 4.5:1 (WCAG AA) against
//! what it is drawn on, in both themes, and every icon-only control colour
//! 3:1 (WCAG 1.4.11). Pills are a translucent tint over the table, so the
//! tint is composited first. The file is scanned so a new token can't skip
//! the check: it is a text token with a pair below, a graphic with a pair in
//! `GRAPHICS`, or a surface or line that `NOT_TEXT` names.
//!
//! Keep `PAIRS`, `GRAPHICS` and `NOT_TEXT` in step with the TypeScript test
//! until G16 deletes it.

use crate::css::{self, Decl};

type Rgba = [f64; 4];

struct Themes {
    dark: Vec<Decl>,
    light: Vec<Decl>,
}

fn themes() -> Themes {
    let sheet = css::parse(crate::SOURCE).expect("tokens.css parses");
    let light = css::layered(&sheet.dark, &sheet.light);
    Themes {
        dark: sheet.dark,
        light,
    }
}

fn value(vars: &[Decl], name: &str) -> Rgba {
    let text = css::resolve(vars, name).unwrap_or_else(|e| panic!("{e}"));
    css::color(text).unwrap_or_else(|| panic!("can't read {name}: {text}"))
}

/// `top` drawn over `bottom` (opaque), as browsers blend: in encoded sRGB.
fn over(top: Rgba, bottom: Rgba) -> Rgba {
    let a = top[3];
    let mix = |i: usize| top[i] * a + bottom[i] * (1.0 - a);
    [mix(0), mix(1), mix(2), 1.0]
}

fn luminance([r, g, b, _]: Rgba) -> f64 {
    let lin = |c: f64| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// The colour of a stack of tokens, bottom first (the first must be opaque).
fn surface(vars: &[Decl], layers: &[String]) -> Rgba {
    layers[1..]
        .iter()
        .fold(value(vars, &layers[0]), |acc, name| over(value(vars, name), acc))
}

fn s(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn with(base: &[&str], top: &str) -> Vec<String> {
    let mut v = s(base);
    v.push(top.to_string());
    v
}

const TABLE: &[&str] = &["--pane-table"];
const ALT_ROW: &[&str] = &["--pane-table", "--pane-row-alt"];

/// [text token, the surface it sits on (bottom first)].
fn pairs() -> Vec<(String, Vec<String>)> {
    let mut p: Vec<(String, Vec<String>)> = [
        ("--foreground", s(&["--background"])),
        ("--muted-foreground", s(&["--background"])),
        ("--muted-foreground", s(&["--pane-toolbar"])),
        ("--muted-foreground", s(&["--pane-header"])),
        ("--muted-foreground", s(&["--popover"])),
        ("--popover-foreground", s(&["--popover"])),
        ("--card-foreground", s(&["--card"])),
        ("--primary-foreground", s(&["--primary"])),
        ("--primary-chip-fg", s(&["--pane-toolbar", "--primary-chip-bg"])),
        ("--secondary-foreground", s(&["--secondary"])),
        ("--accent-foreground", s(&["--accent"])),
        ("--destructive", s(&["--background"])),
        ("--destructive-foreground", s(&["--destructive-solid"])),
        ("--selection-fg", s(&["--selection-bg"])),
        // Pills on a selected row (R4: axe).
        ("--selection-fg", s(&["--selection-bg", "--pill-on-selection"])),
        ("--sidebar-foreground", s(&["--sidebar"])),
        ("--sidebar-accent-foreground", s(&["--sidebar", "--sidebar-accent"])),
        ("--sidebar-primary-foreground", s(&["--sidebar-primary"])),
        ("--muted-foreground", s(ALT_ROW)),
        ("--url-scheme", s(ALT_ROW)),
        ("--url-host", s(ALT_ROW)),
        ("--url-port", s(ALT_ROW)),
        ("--url-path", s(ALT_ROW)),
        ("--url-query", s(ALT_ROW)),
        ("--url-bar-host", s(&["--background"])),
        ("--url-bar-path", s(&["--background"])),
        ("--s-failed", s(ALT_ROW)),
        ("--ssl-pinned", s(ALT_ROW)),
        ("--ssl-passthrough", s(ALT_ROW)),
        ("--rule", s(ALT_ROW)),
    ]
    .into_iter()
    .map(|(fg, bg)| (fg.to_string(), bg))
    .collect();
    for m in [
        "get", "post", "put", "patch", "delete", "head", "options", "connect", "other",
    ] {
        p.push((format!("--m-{m}"), with(ALT_ROW, &format!("--m-{m}-bg"))));
    }
    for st in [
        "1xx", "2xx", "3xx", "4xx", "5xx", "active", "paused", "blocked", "tunnel",
    ] {
        p.push((format!("--s-{st}"), with(ALT_ROW, &format!("--s-{st}-bg"))));
    }
    p.push(("--graphql".into(), with(ALT_ROW, "--graphql-bg")));
    // M17: label dots in the Notes column, label chips in the inspector.
    for l in ["red", "orange", "yellow", "green", "blue", "purple"] {
        p.push((format!("--label-{l}"), s(ALT_ROW)));
        p.push((format!("--label-{l}"), s(&["--background", &format!("--label-{l}-bg")])));
    }
    p.push(("--rule".into(), with(TABLE, "--rule-bg")));
    p.push(("--rule-agent".into(), with(TABLE, "--rule-agent-bg")));
    // M14: a failed script's badge in the inspector.
    p.push(("--s-5xx".into(), s(&["--background", "--s-5xx-bg"])));
    p.push(("--sent".into(), with(ALT_ROW, "--sent-bg")));
    p.push(("--sent-agent".into(), with(ALT_ROW, "--sent-agent-bg")));
    p.push(("--ws-sent".into(), s(&["--background"])));
    p.push(("--ws-received".into(), s(&["--background"])));
    for t in [
        "key", "string", "number", "literal", "keyword", "tag", "attr", "comment", "variable", "type", "punct", "meta",
    ] {
        p.push((format!("--syntax-{t}"), s(&["--background", "--code-active-line"])));
    }
    for d in ["add", "change", "del"] {
        p.push((format!("--diff-{d}"), s(&["--background", &format!("--diff-{d}-bg")])));
    }
    for t in ["key", "string", "number", "literal", "punct"] {
        for d in ["add", "change", "del"] {
            p.push((format!("--syntax-{t}"), s(&["--background", &format!("--diff-{d}-bg")])));
        }
    }
    p.push(("--foreground".into(), s(&["--background", "--diff-change-bg"])));
    p.push(("--code-gutter-fg".into(), s(&["--code-gutter"])));
    p.push(("--foreground".into(), s(&["--background", "--code-selection"])));
    p.push(("--foreground".into(), s(&["--background", "--code-match-current"])));
    p
}

/// Icon-only controls: 3:1 on what they sit on (R4: the record button).
fn graphics() -> Vec<(String, Vec<String>)> {
    vec![
        ("--record-start".into(), s(&["--pane-toolbar"])),
        ("--record-stop".into(), s(&["--pane-toolbar"])),
    ]
}

/// Tokens that are not drawn as text: surfaces, tints, lines, chart and
/// timing bars (labelled elsewhere) and non-colours.
fn not_text(token: &str) -> bool {
    let t = token.trim_start_matches("--");
    let any = |names: &[&str]| names.contains(&t);
    t.ends_with("-bg")
        || any(&[
            "background",
            "card",
            "popover",
            "primary",
            "secondary",
            "muted",
            "accent",
            "destructive-solid",
            "paper",
            "overlay",
            "radius",
            "border",
            "input",
            "ring",
            "sidebar",
            "sidebar-accent",
            "sidebar-primary",
            "sidebar-border",
            "sidebar-ring",
            "pill-on-selection",
            "code-gutter",
            "code-active-line",
            "code-selection",
            "code-match",
            "code-match-current",
            "code-guide",
            "code-row-hover",
        ])
        || t.starts_with("pane-")
        || t.starts_with("checker-")
        || t.starts_with("timing-")
        || t.strip_prefix("chart-")
            .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
}

fn declared(themes: &Themes) -> Vec<String> {
    let mut names: Vec<String> = themes.dark.iter().map(|(n, _)| n.clone()).collect();
    for (n, _) in &themes.light {
        if !names.contains(n) {
            names.push(n.clone());
        }
    }
    names
}

#[test]
fn names_no_text_token_without_a_contrast_pair() {
    let themes = themes();
    let checked: Vec<String> = pairs().into_iter().chain(graphics()).map(|(fg, _)| fg).collect();
    let unchecked: Vec<String> = declared(&themes)
        .into_iter()
        .filter(|t| !checked.contains(t) && !not_text(t))
        .collect();
    assert!(
        unchecked.is_empty(),
        "add a pair to PAIRS (or GRAPHICS), or list a surface in NOT_TEXT: {unchecked:?}"
    );
}

#[test]
fn lists_no_pair_for_a_token_it_doesnt_declare() {
    let names = declared(&themes());
    let missing: Vec<String> = pairs()
        .into_iter()
        .chain(graphics())
        .flat_map(|(fg, bg)| std::iter::once(fg).chain(bg))
        .filter(|n| !names.contains(n))
        .collect();
    assert!(missing.is_empty(), "{missing:?}");
}

fn check(min: f64, list: Vec<(String, Vec<String>)>) {
    let themes = themes();
    let mut failures = Vec::new();
    for (theme, vars) in [("dark", &themes.dark), ("light", &themes.light)] {
        for (fg, bg) in &list {
            let ratio = contrast(value(vars, fg), surface(vars, bg));
            if ratio < min {
                failures.push(format!("{theme}: {fg} on {}: {ratio:.2}", bg.join(" + ")));
            }
        }
    }
    assert!(failures.is_empty(), "below {min}:1:\n{}", failures.join("\n"));
}

#[test]
fn every_text_pair_reaches_4_5_to_1_in_both_themes() {
    check(4.5, pairs());
}

#[test]
fn every_graphic_reaches_3_to_1_in_both_themes() {
    check(3.0, graphics());
}

#[test]
fn the_check_fails_a_low_contrast_pair() {
    let vars = vec![
        ("--fg".to_string(), "oklch(0.5 0 0)".to_string()),
        ("--bg".to_string(), "oklch(0.55 0 0)".to_string()),
    ];
    assert!(contrast(value(&vars, "--fg"), surface(&vars, &s(&["--bg"]))) < 1.5);
}
