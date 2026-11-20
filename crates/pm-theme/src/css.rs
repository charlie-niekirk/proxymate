//! Reads `tokens.css`: the `:root, [data-theme="dark"]` block, the light
//! overrides and the macOS overrides, with `var()` references resolved and
//! `oklch()` converted to sRGB exactly as `contrast.test.ts` does. Shared by
//! `build.rs` (which bakes the values into Rust) and the crate's tests.

/// A declared token: its CSS name and value text.
pub type Decl = (String, String);

/// The three blocks of `tokens.css`, each in declaration order.
pub struct Sheet {
    pub dark: Vec<Decl>,
    pub light: Vec<Decl>,
    pub macos: Vec<Decl>,
}

pub fn parse(css: &str) -> Result<Sheet, String> {
    Ok(Sheet {
        dark: block(css, ":root,")?,
        light: block(css, "[data-theme=\"light\"] {")?,
        macos: block(css, "[data-platform=\"macos\"] {")?,
    })
}

/// The declarations of the block whose selector starts with `selector`.
fn block(css: &str, selector: &str) -> Result<Vec<Decl>, String> {
    let start = css.find(selector).ok_or_else(|| format!("no {selector} block"))?;
    let open = start + css[start..].find('{').ok_or("unterminated selector")?;
    let close = open + css[open..].find("\n}").ok_or("unterminated block")?;
    let body = strip_comments(&css[open + 1..close]);
    let mut decls = Vec::new();
    for decl in body.split(';') {
        let decl = decl.trim();
        let Some(rest) = decl.strip_prefix("--") else { continue };
        let (name, value) = rest.split_once(':').ok_or_else(|| format!("bad declaration {decl}"))?;
        decls.push((format!("--{}", name.trim()), value.trim().to_string()));
    }
    Ok(decls)
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i..].find("*/") {
            Some(j) => rest = &rest[i + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// `base` with `overrides` laid over it (the light theme is the dark block
/// plus its own declarations, as the cascade has it).
pub fn layered(base: &[Decl], overrides: &[Decl]) -> Vec<Decl> {
    let mut out = base.to_vec();
    for (name, value) in overrides {
        match out.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = value.clone(),
            None => out.push((name.clone(), value.clone())),
        }
    }
    out
}

/// The value of `name`, following `var(--x)` references.
pub fn resolve<'a>(vars: &'a [Decl], name: &str) -> Result<&'a str, String> {
    let mut name = name;
    for _ in 0..10 {
        let value = vars
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .ok_or_else(|| format!("{name} is not defined"))?;
        match value.strip_prefix("var(").and_then(|v| v.strip_suffix(')')) {
            Some(next) => name = next.trim(),
            None => return Ok(value),
        }
    }
    Err(format!("{name}: var() nests too deep"))
}

/// sRGB (0..1, gamma-encoded) and alpha of `transparent` or
/// `oklch(L C H / A)`; `None` for anything else (`--radius`).
pub fn color(value: &str) -> Option<[f64; 4]> {
    if value == "transparent" {
        return Some([0.0; 4]);
    }
    let inner = value.strip_prefix("oklch(")?.strip_suffix(')')?;
    let (lch, alpha) = match inner.split_once('/') {
        Some((lch, alpha)) => (lch, Some(alpha.trim())),
        None => (inner, None),
    };
    let mut parts = lch.split_whitespace().map(str::parse::<f64>);
    let (l, c, h) = (parts.next()?.ok()?, parts.next()?.ok()?, parts.next()?.ok()?);
    if parts.next().is_some() {
        return None;
    }
    let alpha = match alpha {
        None => 1.0,
        Some(a) => match a.strip_suffix('%') {
            Some(pct) => pct.trim().parse::<f64>().ok()? / 100.0,
            None => a.parse::<f64>().ok()?,
        },
    };
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.291485548 * b).powi(3);
    let linear = [
        4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
        -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
        -0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_,
    ];
    let encode = |c: f64| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.0031308 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    };
    Some([encode(linear[0]), encode(linear[1]), encode(linear[2]), alpha])
}

/// `--m-get-bg` → `m_get_bg`.
pub fn field_name(token: &str) -> String {
    token.trim_start_matches("--").replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklch_converts_like_the_ui_test() {
        let white = color("oklch(1 0 0)").unwrap();
        assert!(white.iter().all(|c| (c - 1.0).abs() < 1e-6), "{white:?}");
        assert_eq!(color("oklch(0 0 0 / 50%)"), Some([0.0, 0.0, 0.0, 0.5]));
        assert_eq!(color("transparent"), Some([0.0; 4]));
        assert_eq!(color("0.375rem"), None);
        // #0A84FF-ish system blue.
        let [r, g, b, _] = color("oklch(0.56 0.19 258)").unwrap();
        assert!(r < 0.2 && g > 0.3 && b > 0.85, "{r} {g} {b}");
    }

    #[test]
    fn blocks_layer_and_resolve() {
        let css = ":root,\n[data-theme=\"dark\"] {\n  /* c */ --a: oklch(1 0 0);\n  --b: var(--a);\n}\n\
                   [data-theme=\"light\"] {\n  --a: oklch(0 0 0);\n}\n[data-platform=\"macos\"] {\n  --b: transparent;\n}\n";
        let sheet = parse(css).unwrap();
        assert_eq!(sheet.dark.len(), 2);
        let light = layered(&sheet.dark, &sheet.light);
        assert_eq!(resolve(&light, "--b").unwrap(), "oklch(0 0 0)");
        assert_eq!(sheet.macos, vec![("--b".to_string(), "transparent".to_string())]);
    }
}
