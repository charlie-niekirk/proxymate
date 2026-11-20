//! Bakes `apps/desktop/ui/src/styles/tokens.css` (the one source of colours
//! until G16 moves it here) into `$OUT_DIR/tokens.rs`: a `Tokens` struct with
//! one field per colour token, `DARK` and `LIGHT`, and the macOS overrides.

#[allow(dead_code)]
#[path = "src/css.rs"]
mod css;

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest.join("../../apps/desktop/ui/src/styles/tokens.css");
    println!("cargo::rerun-if-changed={}", source.display());
    println!("cargo::rerun-if-changed=src/css.rs");
    let text = std::fs::read_to_string(&source).unwrap_or_else(|e| panic!("{}: {e}", source.display()));
    let sheet = css::parse(&text).unwrap_or_else(|e| panic!("tokens.css: {e}"));
    let dark = sheet.dark.clone();
    let light = css::layered(&sheet.dark, &sheet.light);

    // Colour tokens in declaration order (the dark block declares them all).
    let names: Vec<&str> = dark
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| css::resolve(&dark, n).ok().and_then(css::color).is_some())
        .collect();
    let mut out = String::new();
    let _ = writeln!(out, "/// The path the tokens were read from.");
    let _ = writeln!(out, "pub const SOURCE_PATH: &str = {:?};", source.display().to_string());
    let _ = writeln!(out, "/// tokens.css itself, for the contrast test.");
    let _ = writeln!(
        out,
        "pub const SOURCE: &str = include_str!({:?});",
        source.display().to_string()
    );
    let _ = writeln!(out, "/// Every colour token, as tokens.css names it.");
    let _ = writeln!(out, "pub const NAMES: &[&str] = &{names:?};");
    let _ = writeln!(
        out,
        "/// One theme's colours, one field per token (`--m-get-bg` is `m_get_bg`)."
    );
    let _ = writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq)]\npub struct Tokens {{");
    for name in &names {
        let _ = writeln!(out, "    /// `{name}`\n    pub {}: Rgba,", css::field_name(name));
    }
    let _ = writeln!(out, "}}");
    for (konst, vars) in [("DARK", &dark), ("LIGHT", &light)] {
        let _ = writeln!(out, "pub const {konst}: Tokens = Tokens {{");
        for name in &names {
            let value = css::resolve(vars, name).unwrap();
            let [r, g, b, a] = css::color(value).unwrap_or_else(|| panic!("{konst} {name}: {value}"));
            let _ = writeln!(
                out,
                "    {}: Rgba {{ r: {:?}, g: {:?}, b: {:?}, a: {:?} }},",
                css::field_name(name),
                r as f32,
                g as f32,
                b as f32,
                a as f32
            );
        }
        let _ = writeln!(out, "}};");
    }
    let _ = writeln!(out, "impl Tokens {{");
    let _ = writeln!(out, "    /// The colour of a token by its CSS name (`--m-get`).");
    let _ = writeln!(
        out,
        "    pub fn get(&self, name: &str) -> Option<Rgba> {{\n        Some(match name {{"
    );
    for name in &names {
        let _ = writeln!(out, "            {name:?} => self.{},", css::field_name(name));
    }
    let _ = writeln!(out, "            _ => return None,\n        }})\n    }}");
    let _ = writeln!(
        out,
        "    /// The `[data-platform=\"macos\"]` overrides (the sidebar shows vibrancy)."
    );
    let _ = writeln!(out, "    pub const fn macos(mut self) -> Self {{");
    for (name, value) in &sheet.macos {
        let [r, g, b, a] = css::color(value).unwrap_or_else(|| panic!("macos {name}: {value}"));
        let _ = writeln!(
            out,
            "        self.{} = Rgba {{ r: {:?}, g: {:?}, b: {:?}, a: {:?} }};",
            css::field_name(name),
            r as f32,
            g as f32,
            b as f32,
            a as f32
        );
    }
    let _ = writeln!(out, "        self\n    }}\n}}");
    let dest = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("tokens.rs");
    std::fs::write(dest, out).unwrap();
}
