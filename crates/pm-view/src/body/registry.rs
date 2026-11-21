//! The viewer registry (`BodyViewer/registry.ts`): every way a body can be
//! shown, and which ones fit each content kind (best first). The pane picks
//! the first, or the one the user last chose for that kind; Text and Hex
//! are there as fallbacks.

use pm_model::ContentKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewerId {
    Json,
    JsonTree,
    Graphql,
    Xml,
    Html,
    HtmlPreview,
    Form,
    Multipart,
    Javascript,
    Css,
    Text,
    Image,
    Video,
    Audio,
    Pdf,
    Font,
    Grpc,
    Decoded,
    Hex,
}

/// Where a viewer gets its body from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The decoded bytes.
    Bytes,
    /// A file or URL the viewer loads itself (G5's media).
    Url,
    /// The engine's `decode_body_in` (G5), live while a stream arrives.
    Decoded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerDef {
    pub label: &'static str,
    pub source: Source,
    /// Syntax-highlighted text, plain above the highlight limit.
    pub code: bool,
}

impl ViewerId {
    pub const ALL: [ViewerId; 19] = [
        ViewerId::Json,
        ViewerId::JsonTree,
        ViewerId::Graphql,
        ViewerId::Xml,
        ViewerId::Html,
        ViewerId::HtmlPreview,
        ViewerId::Form,
        ViewerId::Multipart,
        ViewerId::Javascript,
        ViewerId::Css,
        ViewerId::Text,
        ViewerId::Image,
        ViewerId::Video,
        ViewerId::Audio,
        ViewerId::Pdf,
        ViewerId::Font,
        ViewerId::Grpc,
        ViewerId::Decoded,
        ViewerId::Hex,
    ];

    pub const fn def(self) -> ViewerDef {
        use Source::*;
        let (label, source, code) = match self {
            ViewerId::Json => ("JSON", Bytes, true),
            ViewerId::JsonTree => ("JSON Tree", Bytes, false),
            ViewerId::Graphql => ("GraphQL", Bytes, true),
            ViewerId::Xml => ("XML", Bytes, true),
            ViewerId::Html => ("HTML", Bytes, true),
            ViewerId::HtmlPreview => ("Preview", Bytes, false),
            ViewerId::Form => ("Form", Bytes, false),
            ViewerId::Multipart => ("Multipart", Bytes, false),
            ViewerId::Javascript => ("JavaScript", Bytes, true),
            ViewerId::Css => ("CSS", Bytes, true),
            ViewerId::Text => ("Text", Bytes, true),
            ViewerId::Image => ("Image", Url, false),
            ViewerId::Video => ("Video", Url, false),
            ViewerId::Audio => ("Audio", Url, false),
            ViewerId::Pdf => ("PDF", Url, false),
            ViewerId::Font => ("Font", Url, false),
            ViewerId::Grpc => ("gRPC", Decoded, false),
            ViewerId::Decoded => ("Decoded", Decoded, false),
            ViewerId::Hex => ("Hex", Bytes, false),
        };
        ViewerDef { label, source, code }
    }

    pub const fn label(self) -> &'static str {
        self.def().label
    }

    /// The id the Tauri UI and the dev remote use (`json-tree`, `html-preview`).
    pub const fn id(self) -> &'static str {
        match self {
            ViewerId::Json => "json",
            ViewerId::JsonTree => "json-tree",
            ViewerId::Graphql => "graphql",
            ViewerId::Xml => "xml",
            ViewerId::Html => "html",
            ViewerId::HtmlPreview => "html-preview",
            ViewerId::Form => "form",
            ViewerId::Multipart => "multipart",
            ViewerId::Javascript => "javascript",
            ViewerId::Css => "css",
            ViewerId::Text => "text",
            ViewerId::Image => "image",
            ViewerId::Video => "video",
            ViewerId::Audio => "audio",
            ViewerId::Pdf => "pdf",
            ViewerId::Font => "font",
            ViewerId::Grpc => "grpc",
            ViewerId::Decoded => "decoded",
            ViewerId::Hex => "hex",
        }
    }

    /// The viewer an id (or a label) names, case-insensitively.
    pub fn parse(s: &str) -> Option<ViewerId> {
        ViewerId::ALL
            .into_iter()
            .find(|v| v.id().eq_ignore_ascii_case(s) || v.label().eq_ignore_ascii_case(s))
    }
}

/// The viewers that fit `kind`, best first (`BY_KIND`).
pub fn viewers_for(kind: ContentKind) -> &'static [ViewerId] {
    use ViewerId::*;
    match kind {
        ContentKind::None => &[Text, Hex],
        ContentKind::Json => &[Json, JsonTree, Text, Hex],
        ContentKind::GraphQl => &[Graphql, Json, JsonTree, Text, Hex],
        ContentKind::Xml => &[Xml, Text, Hex],
        ContentKind::Html => &[Html, HtmlPreview, Text, Hex],
        ContentKind::FormUrlEncoded => &[Form, Text, Hex],
        ContentKind::Multipart => &[Multipart, Text, Hex],
        ContentKind::Javascript => &[Javascript, Text, Hex],
        ContentKind::Css => &[Css, Text, Hex],
        ContentKind::Text => &[Text, Json, Hex],
        ContentKind::Image => &[Image, Hex],
        ContentKind::Svg => &[Image, Xml, Text, Hex],
        ContentKind::Video => &[Video, Hex],
        ContentKind::Audio => &[Audio, Hex],
        ContentKind::Font => &[Font, Hex],
        ContentKind::Pdf => &[Pdf, Hex],
        ContentKind::Binary => &[Hex, Text],
        ContentKind::Grpc => &[Grpc, Hex],
        ContentKind::Protobuf => &[Decoded, Hex],
        ContentKind::MessagePack => &[Decoded, Hex],
        ContentKind::Cbor => &[Decoded, Hex],
    }
}

/// The user's choice for this kind if it applies, else the best viewer.
pub fn pick_viewer(kind: ContentKind, preferred: Option<ViewerId>) -> ViewerId {
    let options = viewers_for(kind);
    match preferred {
        Some(p) if options.contains(&p) => p,
        _ => options[0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [ContentKind; 21] = [
        ContentKind::None,
        ContentKind::Json,
        ContentKind::Xml,
        ContentKind::Html,
        ContentKind::FormUrlEncoded,
        ContentKind::Multipart,
        ContentKind::GraphQl,
        ContentKind::Javascript,
        ContentKind::Css,
        ContentKind::Text,
        ContentKind::Image,
        ContentKind::Svg,
        ContentKind::Video,
        ContentKind::Audio,
        ContentKind::Font,
        ContentKind::Pdf,
        ContentKind::Binary,
        ContentKind::Grpc,
        ContentKind::Protobuf,
        ContentKind::MessagePack,
        ContentKind::Cbor,
    ];

    #[test]
    fn offers_hex_for_every_kind_and_a_known_viewer_first() {
        for kind in KINDS {
            assert!(viewers_for(kind).contains(&ViewerId::Hex), "{kind:?}");
            assert!(!viewers_for(kind).is_empty());
        }
    }

    #[test]
    fn picks_the_best_viewer_unless_the_user_chose_another_that_fits() {
        assert_eq!(pick_viewer(ContentKind::Json, None), ViewerId::Json);
        assert_eq!(
            pick_viewer(ContentKind::Json, Some(ViewerId::JsonTree)),
            ViewerId::JsonTree
        );
        assert_eq!(
            pick_viewer(ContentKind::Image, Some(ViewerId::JsonTree)),
            ViewerId::Image
        );
        assert_eq!(pick_viewer(ContentKind::Svg, None), ViewerId::Image);
        assert_eq!(pick_viewer(ContentKind::Binary, None), ViewerId::Hex);
        assert_eq!(pick_viewer(ContentKind::Grpc, None), ViewerId::Grpc);
        assert_eq!(
            pick_viewer(ContentKind::Cbor, Some(ViewerId::JsonTree)),
            ViewerId::Decoded
        );
    }

    #[test]
    fn ids_round_trip() {
        for v in ViewerId::ALL {
            assert_eq!(ViewerId::parse(v.id()), Some(v));
            assert_eq!(ViewerId::parse(v.label()), Some(v));
        }
        assert_eq!(ViewerId::parse("nope"), None);
    }
}
