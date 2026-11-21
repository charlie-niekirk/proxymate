//! Pretty-prints XML and HTML by re-indenting tags (`lib/body/markup.ts`).
//! A tokenizer, not a parser: it never drops or reorders content, so
//! malformed documents come out readable rather than "fixed". Text inside
//! script, style, pre and textarea is kept verbatim.

use std::sync::LazyLock;

use regex::Regex;

const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr",
];
const RAW: [&str; 4] = ["script", "style", "pre", "textarea"];

static TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<!--.*?(?:-->|\z)|<!\[CDATA\[.*?(?:\]\]>|\z)|<\?.*?(?:\?>|\z)|<![^>]*>?|</?[A-Za-z][^\s/>]*(?:"[^"]*"|'[^']*'|[^'">])*>?"#,
    )
    .expect("valid tag pattern")
});
static MULTI_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n\s*\S[\s\S]*\n").expect("valid"));
static NEWLINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\n\s*").expect("valid"));

#[derive(Debug)]
enum Token<'a> {
    Open {
        name: String,
        text: &'a str,
        self_closing: bool,
    },
    Close {
        name: String,
        text: &'a str,
    },
    Other(&'a str),
    Text(&'a str),
}

fn tag_name(after: &str, stop: &[char]) -> String {
    after
        .split(|c: char| c.is_whitespace() || stop.contains(&c))
        .next()
        .unwrap_or("")
        .to_owned()
}

fn tokenize(src: &str, html: bool) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut last = 0;
    let mut from = 0;
    while let Some(m) = TAG.find_at(src, from) {
        if m.start() > last {
            out.push(Token::Text(&src[last..m.start()]));
        }
        let text = m.as_str();
        last = m.end();
        from = m.end().max(m.start() + 1);
        if let Some(rest) = text.strip_prefix("</") {
            out.push(Token::Close {
                name: tag_name(rest, &['>']).to_lowercase(),
                text,
            });
        } else if text.len() > 1 && text.as_bytes()[1].is_ascii_alphabetic() {
            let name = tag_name(&text[1..], &['/', '>']);
            let lower = name.to_lowercase();
            let self_closing = text.ends_with("/>") || (html && VOID.contains(&lower.as_str()));
            out.push(Token::Open {
                name: if html { lower.clone() } else { name },
                text,
                self_closing,
            });
            if html && RAW.contains(&lower.as_str()) && !self_closing {
                // Everything up to the matching close tag is content.
                let close = Regex::new(&format!(r"(?i)</{lower}\s*>")).expect("valid close pattern");
                let stop = close.find_at(src, last).map_or(src.len(), |m| m.start());
                if stop > last {
                    out.push(Token::Text(&src[last..stop]));
                }
                last = stop;
                from = stop;
            }
        } else {
            out.push(Token::Other(text));
        }
    }
    if last < src.len() {
        out.push(Token::Text(&src[last..]));
    }
    out
}

/// `src` re-indented by `indent` per level.
pub fn format_markup(src: &str, html: bool, indent: &str) -> String {
    let tokens = tokenize(src, html);
    let mut lines: Vec<String> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let pad = |stack: &Vec<String>| indent.repeat(stack.len());
    let key = |name: &str| if html { name.to_lowercase() } else { name.to_owned() };
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Open {
                name,
                text,
                self_closing,
            } => {
                if *self_closing {
                    lines.push(pad(&stack) + text);
                    i += 1;
                    continue;
                }
                let raw = html && RAW.contains(&name.to_lowercase().as_str());
                if let Some(Token::Close { name: close, text: ct }) = tokens.get(i + 1)
                    && key(close) == key(name)
                {
                    lines.push(format!("{}{text}{ct}", pad(&stack)));
                    i += 2;
                    continue;
                }
                if let (Some(Token::Text(inner)), Some(Token::Close { name: close, text: ct })) =
                    (tokens.get(i + 1), tokens.get(i + 2))
                    && key(close) == key(name)
                {
                    let fits = if raw {
                        !inner.contains('\n')
                    } else {
                        let trimmed = inner.trim();
                        !MULTI_LINE.is_match(trimmed) && trimmed.chars().count() <= 120
                    };
                    if fits {
                        let inner = if raw {
                            (*inner).to_owned()
                        } else {
                            NEWLINES.replace_all(inner.trim(), " ").into_owned()
                        };
                        lines.push(format!("{}{text}{inner}{ct}", pad(&stack)));
                        i += 3;
                        continue;
                    }
                }
                lines.push(pad(&stack) + text);
                stack.push(name.clone());
                if raw && let Some(Token::Text(inner)) = tokens.get(i + 1) {
                    lines.push(reindent(inner, &pad(&stack)));
                    i += 1;
                }
            }
            Token::Close { name, text } => {
                if let Some(at) = stack.iter().rposition(|n| n == name) {
                    stack.truncate(at);
                }
                lines.push(pad(&stack) + text);
            }
            Token::Other(text) => lines.push(pad(&stack) + text),
            Token::Text(text) => {
                for line in text.trim().lines() {
                    if !line.trim().is_empty() {
                        lines.push(pad(&stack) + line.trim());
                    }
                }
            }
        }
        i += 1;
    }
    lines.join("\n")
}

/// A block of raw text (a script, a style) under `pad`, keeping its
/// relative indentation.
fn reindent(text: &str, pad: &str) -> String {
    let trimmed = text.trim_end();
    // Leading blank lines go, up to the last newline before the content.
    let start = trimmed
        .char_indices()
        .take_while(|(_, c)| c.is_whitespace())
        .filter(|(_, c)| *c == '\n')
        .last()
        .map_or(0, |(i, _)| i + 1);
    let lines: Vec<&str> = trimmed[start..].lines().collect();
    let indent_of = |l: &str| l.len() - l.trim_start().len();
    let common = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent_of(l))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                format!("{pad}{}", &l[common.min(indent_of(l))..])
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indents_nested_xml_and_keeps_short_text_inline() {
        let xml = r#"<?xml version="1.0"?><feed><title>Hi &amp; bye</title><entry id="1"><a/><b></b></entry><!-- c --></feed>"#;
        assert_eq!(
            format_markup(xml, false, "  "),
            [
                r#"<?xml version="1.0"?>"#,
                "<feed>",
                "  <title>Hi &amp; bye</title>",
                r#"  <entry id="1">"#,
                "    <a/>",
                "    <b></b>",
                "  </entry>",
                "  <!-- c -->",
                "</feed>"
            ]
            .join("\n")
        );
    }

    #[test]
    fn knows_html_void_and_raw_text_elements() {
        let html = "<!DOCTYPE html><html><head><meta charset=utf-8><script>if (a < b) { go(); }</script></head><body><p>x<br>y</p></body></html>";
        assert_eq!(
            format_markup(html, true, "  "),
            [
                "<!DOCTYPE html>",
                "<html>",
                "  <head>",
                "    <meta charset=utf-8>",
                "    <script>if (a < b) { go(); }</script>",
                "  </head>",
                "  <body>",
                "    <p>",
                "      x",
                "      <br>",
                "      y",
                "    </p>",
                "  </body>",
                "</html>",
            ]
            .join("\n")
        );
    }

    #[test]
    fn keeps_multi_line_scripts_verbatim_but_re_indented() {
        let out = format_markup(
            "<div><style>\n    a { b: c }\n      d { e: f }\n</style></div>",
            true,
            "  ",
        );
        assert_eq!(
            out,
            [
                "<div>",
                "  <style>",
                "    a { b: c }",
                "      d { e: f }",
                "  </style>",
                "</div>"
            ]
            .join("\n")
        );
    }

    #[test]
    fn survives_unbalanced_and_truncated_markup() {
        assert_eq!(
            format_markup("<a><b></a><c", false, "  "),
            ["<a>", "  <b>", "</a>", "<c"].join("\n")
        );
    }
}
