//! Line highlighting for the CodeView (bodies over the 2 MB editor limit,
//! the Raw tab): a small lexer per language that colours one line with no
//! state from the lines before it, so only the rows on screen are ever
//! highlighted. Multi-line strings and comments are coloured on their first
//! line only; that's the price of highlighting 20 MB without parsing it.
//! Bodies under the limit use the editor's tree-sitter grammars instead.

use std::ops::Range;

/// What a span is, mapped onto the `--syntax-*` tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tone {
    Key,
    String,
    Number,
    Literal,
    Punct,
    Keyword,
    Comment,
    Tag,
    Attr,
    Meta,
    Type,
    Variable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Plain,
    Json,
    /// XML and HTML.
    Markup,
    Javascript,
    Css,
    Graphql,
    /// An HTTP message head (request or status line, then headers).
    HttpHead,
}

pub type Spans = Vec<(Range<usize>, Tone)>;

/// The spans of one line (no line break in it).
pub fn highlight_line(lang: Lang, line: &str) -> Spans {
    match lang {
        Lang::Plain => Vec::new(),
        Lang::Json => json(line),
        Lang::Markup => markup(line),
        Lang::Javascript => c_like(line, JS_KEYWORDS, false),
        Lang::Css => css(line),
        Lang::Graphql => graphql(line),
        Lang::HttpHead => Vec::new(),
    }
}

/// A line of an HTTP head: the first is the request or status line.
pub fn highlight_http_head(line: &str, first: bool) -> Spans {
    if first {
        let mut out = Vec::new();
        let mut at = 0;
        for (i, word) in line.split(' ').enumerate() {
            let range = at..at + word.len();
            at += word.len() + 1;
            if word.is_empty() {
                continue;
            }
            let tone = if word.starts_with("HTTP/") {
                Tone::Meta
            } else if i == 0 {
                Tone::Keyword
            } else if word.bytes().all(|b| b.is_ascii_digit()) && i == 1 {
                Tone::Number
            } else if i == 1 {
                Tone::String
            } else {
                continue;
            };
            out.push((range, tone));
        }
        return out;
    }
    match line.find(':') {
        Some(colon) if colon > 0 => vec![(0..colon, Tone::Attr), (colon..colon + 1, Tone::Punct)],
        _ => Vec::new(),
    }
}

fn string_end(b: &[u8], start: usize, quote: u8) -> usize {
    let mut j = start + 1;
    while j < b.len() && b[j] != quote {
        j += if b[j] == b'\\' { 2 } else { 1 };
    }
    (j + 1).min(b.len())
}

fn number_end(b: &[u8], start: usize) -> usize {
    let mut j = start;
    while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'.' | b'-' | b'+' | b'_')) {
        j += 1;
    }
    j
}

fn word_end(b: &[u8], start: usize) -> usize {
    let mut j = start;
    while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'_' | b'$')) {
        j += 1;
    }
    j
}

fn json(line: &str) -> Spans {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let end = string_end(b, i, b'"');
                let mut k = end;
                while k < b.len() && b[k] == b' ' {
                    k += 1;
                }
                let tone = if b.get(k) == Some(&b':') { Tone::Key } else { Tone::String };
                out.push((i..end, tone));
                i = end;
            }
            b'-' | b'0'..=b'9' => {
                let end = number_end(b, i);
                out.push((i..end, Tone::Number));
                i = end;
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                out.push((i..i + 1, Tone::Punct));
                i += 1;
            }
            c if c.is_ascii_alphabetic() => {
                let end = word_end(b, i);
                if matches!(&line[i..end], "true" | "false" | "null") {
                    out.push((i..end, Tone::Literal));
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

fn markup(line: &str) -> Spans {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            let end = b[i..]
                .iter()
                .take(12)
                .position(|c| *c == b';')
                .map(|p| i + p + 1);
            if let Some(end) = end {
                out.push((i..end, Tone::Literal));
                i = end;
                continue;
            }
        }
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if line[i..].starts_with("<!--") {
            let end = line[i + 4..].find("-->").map_or(b.len(), |p| i + 4 + p + 3);
            out.push((i..end, Tone::Comment));
            i = end;
            continue;
        }
        if line[i..].starts_with("<?") || line[i..].starts_with("<!") {
            let end = line[i..].find('>').map_or(b.len(), |p| i + p + 1);
            out.push((i..end, Tone::Meta));
            i = end;
            continue;
        }
        // A tag: `<`, `/`, the name, then attributes up to `>`.
        let mut j = i + 1;
        if b.get(j) == Some(&b'/') {
            j += 1;
        }
        if !b.get(j).is_some_and(u8::is_ascii_alphabetic) {
            i += 1;
            continue;
        }
        out.push((i..j, Tone::Punct));
        let name_end = {
            let mut k = j;
            while k < b.len() && !b[k].is_ascii_whitespace() && !matches!(b[k], b'>' | b'/') {
                k += 1;
            }
            k
        };
        out.push((j..name_end, Tone::Tag));
        j = name_end;
        while j < b.len() && b[j] != b'>' {
            match b[j] {
                b'"' | b'\'' => {
                    let end = string_end(b, j, b[j]);
                    out.push((j..end, Tone::String));
                    j = end;
                }
                b'/' | b'=' => {
                    out.push((j..j + 1, Tone::Punct));
                    j += 1;
                }
                c if c.is_ascii_whitespace() => j += 1,
                _ => {
                    let mut k = j;
                    while k < b.len() && !b[k].is_ascii_whitespace() && !matches!(b[k], b'>' | b'=' | b'/') {
                        k += 1;
                    }
                    out.push((j..k.max(j + 1), Tone::Attr));
                    j = k.max(j + 1);
                }
            }
        }
        if j < b.len() {
            out.push((j..j + 1, Tone::Punct));
            j += 1;
        }
        i = j;
    }
    out
}

const JS_KEYWORDS: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else",
    "export", "extends", "finally", "for", "function", "if", "import", "in", "instanceof", "let", "new", "of", "return",
    "static", "super", "switch", "this", "throw", "try", "typeof", "var", "void", "while", "yield", "from",
];

/// JavaScript (and the base of CSS): comments, strings, numbers, keywords.
fn c_like(line: &str, keywords: &[&str], css: bool) -> Spans {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') && !css => {
                out.push((i..b.len(), Tone::Comment));
                break;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = line[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2);
                out.push((i..end, Tone::Comment));
                i = end;
            }
            q @ (b'"' | b'\'' | b'`') => {
                let end = string_end(b, i, q);
                out.push((i..end, Tone::String));
                i = end;
            }
            b'0'..=b'9' => {
                let end = number_end(b, i);
                out.push((i..end, Tone::Number));
                i = end;
            }
            c if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => {
                let end = word_end(b, i);
                let word = &line[i..end];
                if keywords.contains(&word) {
                    out.push((i..end, Tone::Keyword));
                } else if matches!(word, "true" | "false" | "null" | "undefined") {
                    out.push((i..end, Tone::Literal));
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

fn css(line: &str) -> Spans {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = line[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2);
                out.push((i..end, Tone::Comment));
                i = end;
            }
            q @ (b'"' | b'\'') => {
                let end = string_end(b, i, q);
                out.push((i..end, Tone::String));
                i = end;
            }
            b'@' => {
                let end = word_end(b, i + 1).max(i + 1);
                out.push((i..end, Tone::Keyword));
                i = end;
            }
            b'#' if b.get(i + 1).is_some_and(u8::is_ascii_hexdigit) => {
                let end = word_end(b, i + 1);
                out.push((i..end, Tone::Number));
                i = end;
            }
            b'0'..=b'9' | b'.' if b.get(i).is_some_and(u8::is_ascii_digit) => {
                let end = number_end(b, i);
                out.push((i..end, Tone::Number));
                i = end;
            }
            b'{' | b'}' | b';' | b':' | b',' => {
                out.push((i..i + 1, Tone::Punct));
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == b'-' => {
                let mut end = i;
                while end < b.len() && (b[end].is_ascii_alphanumeric() || matches!(b[end], b'-' | b'_')) {
                    end += 1;
                }
                // A property name is followed by `:` inside a block (`color: red`).
                let mut k = end;
                while k < b.len() && b[k] == b' ' {
                    k += 1;
                }
                let property = b.get(k) == Some(&b':') && !line[k..].starts_with(":hover") && {
                    // `a:hover {` is a selector; `color: red;` a declaration.
                    !line[k..].contains('{')
                };
                if property {
                    out.push((i..end, Tone::Key));
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

const GRAPHQL_KEYWORDS: &[&str] = &[
    "query",
    "mutation",
    "subscription",
    "fragment",
    "on",
    "schema",
    "type",
    "input",
    "enum",
    "interface",
    "union",
    "scalar",
    "directive",
    "extend",
    "implements",
];

fn graphql(line: &str) -> Spans {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut after_colon = false;
    while i < b.len() {
        match b[i] {
            b'#' => {
                out.push((i..b.len(), Tone::Comment));
                break;
            }
            b'"' => {
                let end = string_end(b, i, b'"');
                out.push((i..end, Tone::String));
                i = end;
            }
            b'$' => {
                let end = word_end(b, i + 1).max(i + 1);
                out.push((i..end, Tone::Variable));
                i = end;
            }
            b'@' => {
                let end = word_end(b, i + 1).max(i + 1);
                out.push((i..end, Tone::Meta));
                i = end;
            }
            b'-' | b'0'..=b'9' => {
                let end = number_end(b, i);
                out.push((i..end, Tone::Number));
                i = end;
            }
            b':' => {
                after_colon = true;
                out.push((i..i + 1, Tone::Punct));
                i += 1;
            }
            b'{' | b'}' | b'(' | b')' | b'[' | b']' | b'!' | b'=' | b',' => {
                out.push((i..i + 1, Tone::Punct));
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let end = word_end(b, i);
                let word = &line[i..end];
                if GRAPHQL_KEYWORDS.contains(&word) {
                    out.push((i..end, Tone::Keyword));
                } else if matches!(word, "true" | "false" | "null") {
                    out.push((i..end, Tone::Literal));
                } else if after_colon && word.starts_with(|c: char| c.is_ascii_uppercase()) {
                    out.push((i..end, Tone::Type));
                }
                after_colon = false;
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

/// `top` laid over `base` (both sorted, each without overlaps): the
/// result is split at every boundary, and where both cover a piece its
/// style is `combine(base, top)`. Highlight runs need exactly this shape.
pub fn layer<T: Clone>(
    base: &[(Range<usize>, T)],
    top: &[(Range<usize>, T)],
    combine: impl Fn(&T, &T) -> T,
) -> Vec<(Range<usize>, T)> {
    if top.is_empty() {
        return base.to_vec();
    }
    let mut cuts: Vec<usize> = base
        .iter()
        .chain(top)
        .flat_map(|(r, _)| [r.start, r.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let find = |spans: &[(Range<usize>, T)], at: usize| {
        let i = spans.partition_point(|(r, _)| r.end <= at);
        spans.get(i).filter(|(r, _)| r.start <= at).map(|(_, t)| t.clone())
    };
    let mut out: Vec<(Range<usize>, T)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let style = match (find(base, a), find(top, a)) {
            (Some(x), Some(y)) => Some(combine(&x, &y)),
            (x, y) => x.or(y),
        };
        if let Some(style) = style {
            out.push((a..b, style));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_split_at_every_boundary() {
        let base = [(0..4, 'a'), (6..8, 'b')];
        let top = [(2..7, 'm')];
        let got = layer(&base, &top, |x, y| if *x == 'a' { 'A' } else { (*y).max(*x) });
        assert_eq!(got, [(0..2, 'a'), (2..4, 'A'), (4..6, 'm'), (6..7, 'm'), (7..8, 'b')]);
        assert_eq!(layer(&base, &[], |x, _| *x), base);
    }

    fn spans(lang: Lang, line: &str) -> Vec<(&str, Tone)> {
        highlight_line(lang, line)
            .into_iter()
            .map(|(r, t)| (&line[r], t))
            .collect()
    }

    #[test]
    fn json_keys_strings_numbers_and_literals() {
        assert_eq!(
            spans(Lang::Json, r#"  "id": 1.50, "s": "a\"b", "ok": true"#),
            [
                (r#""id""#, Tone::Key),
                (":", Tone::Punct),
                ("1.50", Tone::Number),
                (",", Tone::Punct),
                (r#""s""#, Tone::Key),
                (":", Tone::Punct),
                (r#""a\"b""#, Tone::String),
                (",", Tone::Punct),
                (r#""ok""#, Tone::Key),
                (":", Tone::Punct),
                ("true", Tone::Literal)
            ]
        );
    }

    #[test]
    fn markup_tags_attributes_and_comments() {
        assert_eq!(
            spans(Lang::Markup, r#"<a href="x">Hi &amp; bye</a><!-- c -->"#),
            [
                ("<", Tone::Punct),
                ("a", Tone::Tag),
                ("href", Tone::Attr),
                ("=", Tone::Punct),
                (r#""x""#, Tone::String),
                (">", Tone::Punct),
                ("&amp;", Tone::Literal),
                ("</", Tone::Punct),
                ("a", Tone::Tag),
                (">", Tone::Punct),
                ("<!-- c -->", Tone::Comment)
            ]
        );
    }

    #[test]
    fn script_css_and_graphql() {
        assert_eq!(
            spans(Lang::Javascript, "const a = 'x'; // done"),
            [("const", Tone::Keyword), ("'x'", Tone::String), ("// done", Tone::Comment)]
        );
        assert_eq!(
            spans(Lang::Css, "a { color: #fff; }"),
            [
                ("{", Tone::Punct),
                ("color", Tone::Key),
                (":", Tone::Punct),
                ("#fff", Tone::Number),
                (";", Tone::Punct),
                ("}", Tone::Punct)
            ]
        );
        assert_eq!(
            spans(Lang::Graphql, "query Q($id: ID!) { a }"),
            [
                ("query", Tone::Keyword),
                ("(", Tone::Punct),
                ("$id", Tone::Variable),
                (":", Tone::Punct),
                ("ID", Tone::Type),
                ("!", Tone::Punct),
                (")", Tone::Punct),
                ("{", Tone::Punct),
                ("}", Tone::Punct)
            ]
        );
    }

    #[test]
    fn http_heads() {
        let line = "GET /a?b=1 HTTP/1.1";
        let got: Vec<_> = highlight_http_head(line, true).into_iter().map(|(r, t)| (&line[r], t)).collect();
        assert_eq!(got, [("GET", Tone::Keyword), ("/a?b=1", Tone::String), ("HTTP/1.1", Tone::Meta)]);
        let line = "HTTP/2 404 Not Found";
        let got: Vec<_> = highlight_http_head(line, true).into_iter().map(|(r, t)| (&line[r], t)).collect();
        assert_eq!(got, [("HTTP/2", Tone::Meta), ("404", Tone::Number)]);
        assert_eq!(highlight_http_head("Host: a", false), [(0..4, Tone::Attr), (4..5, Tone::Punct)]);
    }
}
