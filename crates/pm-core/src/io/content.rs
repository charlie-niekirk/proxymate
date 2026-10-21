//! Content-kind detection. M1 maps the `Content-Type` header only; M3 adds
//! magic-byte sniffing and the text heuristic.

use pm_model::ContentKind;

/// The kind a `Content-Type` value denotes, ignoring parameters.
pub fn kind_from_content_type(value: &str) -> ContentKind {
    let mime = value.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let (top, sub) = mime.split_once('/').unwrap_or((mime.as_str(), ""));
    match (top, sub) {
        ("", _) => ContentKind::None,
        (_, "json") | (_, "graphql-response+json") => ContentKind::Json,
        (_, "graphql") | (_, "graphql+json") => ContentKind::GraphQl,
        (_, s) if s.ends_with("+json") => ContentKind::Json,
        ("text", "html") | ("application", "xhtml+xml") => ContentKind::Html,
        ("image", "svg+xml") => ContentKind::Svg,
        (_, "xml") => ContentKind::Xml,
        (_, s) if s.ends_with("+xml") => ContentKind::Xml,
        ("application", "x-www-form-urlencoded") => ContentKind::FormUrlEncoded,
        ("multipart", _) => ContentKind::Multipart,
        (_, "javascript" | "ecmascript" | "x-javascript") => ContentKind::Javascript,
        ("text", "css") => ContentKind::Css,
        ("text", _) => ContentKind::Text,
        ("image", _) => ContentKind::Image,
        ("video", _) => ContentKind::Video,
        ("audio", _) => ContentKind::Audio,
        ("font", _) | ("application", "font-woff" | "font-woff2" | "x-font-ttf" | "vnd.ms-fontobject") => {
            ContentKind::Font
        }
        ("application", "pdf") => ContentKind::Pdf,
        _ => ContentKind::Binary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_content_types() {
        let cases = [
            ("application/json; charset=utf-8", ContentKind::Json),
            ("application/problem+json", ContentKind::Json),
            ("application/graphql", ContentKind::GraphQl),
            ("text/html", ContentKind::Html),
            ("application/xml", ContentKind::Xml),
            ("application/atom+xml", ContentKind::Xml),
            ("image/svg+xml", ContentKind::Svg),
            ("application/x-www-form-urlencoded", ContentKind::FormUrlEncoded),
            ("multipart/form-data; boundary=x", ContentKind::Multipart),
            ("text/javascript", ContentKind::Javascript),
            ("TEXT/CSS", ContentKind::Css),
            ("text/plain", ContentKind::Text),
            ("image/png", ContentKind::Image),
            ("video/mp4", ContentKind::Video),
            ("font/woff2", ContentKind::Font),
            ("application/pdf", ContentKind::Pdf),
            ("application/octet-stream", ContentKind::Binary),
            ("", ContentKind::None),
        ];
        for (value, kind) in cases {
            assert_eq!(kind_from_content_type(value), kind, "{value}");
        }
    }
}
