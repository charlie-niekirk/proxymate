//! Beautify for JavaScript and CSS (`workers/beautify.worker.ts`):
//! dprint's TypeScript plugin and malva in place of prettier. Both are
//! CPU-bound; the app runs them on the background executor.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Javascript,
    Css,
}

/// `text` reformatted, or why it couldn't be parsed.
pub fn beautify(text: &str, language: Language) -> Result<String, String> {
    match language {
        Language::Javascript => {
            let config = dprint_plugin_typescript::configuration::ConfigurationBuilder::new()
                .indent_width(2)
                .line_width(100)
                .build();
            let formatted = dprint_plugin_typescript::format_text(dprint_plugin_typescript::FormatTextOptions {
                path: Path::new("body.js"),
                extension: Some("js"),
                text: text.into(),
                config: &config,
                external_formatter: None,
                arena_pool: None,
            })
            .map_err(|e| first_line(&e.to_string()))?;
            // `None`: the text asked not to be formatted (an ignore comment).
            Ok(formatted.unwrap_or_else(|| text.to_owned()))
        }
        Language::Css => {
            let mut options = malva::config::FormatOptions::default();
            options.layout.indent_width = 2;
            options.layout.print_width = 100;
            malva::format_text(text, malva::Syntax::Css, &options).map_err(|e| first_line(&format!("{e:?}")))
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_minified_javascript_and_css() {
        let js = beautify("function f(a,b){return a+b}const x={y:1};", Language::Javascript).unwrap();
        assert_eq!(js, "function f(a, b) {\n  return a + b;\n}\nconst x = { y: 1 };\n");
        let css = beautify("a{color:red;margin:0 auto}b,i{x:1}", Language::Css).unwrap();
        assert!(css.starts_with("a {\n  color: red;\n  margin: 0 auto;\n}\n"), "{css}");
    }

    #[test]
    fn reports_what_it_cannot_parse() {
        assert!(beautify("function (", Language::Javascript).is_err());
        assert!(beautify("a { color: red", Language::Css).is_err());
    }
}
