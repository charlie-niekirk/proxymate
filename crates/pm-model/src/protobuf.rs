//! Decoded binary bodies (M16): gRPC messages, protobuf with or without a
//! schema, MessagePack and CBOR, plus the imported `.proto` schemas and the
//! "Decode as…" mappings for protobuf that isn't gRPC.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::detail::{BodyPart, Header};
use crate::rules::Matcher;

/// What `Engine::decode_body` made of a body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct DecodedBody {
    pub format: DecodedFormat,
    /// gRPC: one per length-prefixed message. Otherwise exactly one.
    pub messages: Vec<DecodedMessage>,
    /// The protobuf message type the messages were decoded as, when a
    /// schema applied.
    pub message_type: Option<String>,
    /// gRPC responses: the call's status, from the HTTP/2 trailers, a
    /// trailers-only response's headers or the gRPC-web trailer frame.
    pub status: Option<GrpcStatus>,
    /// gRPC responses: the trailers (HTTP/2, or the gRPC-web trailer frame).
    pub trailers: Vec<Header>,
    /// Bytes after the last whole message: the rest of a message still
    /// streaming, or a body cut at the capture limit.
    pub pending_bytes: u32,
    /// Why the body as a whole couldn't be decoded (bad base64, an unknown
    /// `grpc-encoding`); messages decoded before the problem are kept.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DecodedFormat {
    Grpc,
    GrpcWeb,
    Protobuf,
    MessagePack,
    Cbor,
}

/// One decoded message (the whole body, outside gRPC).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct DecodedMessage {
    pub index: u32,
    /// The gRPC frame's compressed flag.
    pub compressed: bool,
    /// Payload bytes as framed on the wire, before decompression.
    pub size: u32,
    /// The value as pretty JSON text, so 64-bit numbers stay exact.
    pub json: String,
    /// Notes on values, by JSON pointer: protobuf wire types, `bytes`,
    /// MessagePack ext types, CBOR tags.
    pub badges: Vec<Badge>,
    /// Decoded from the wire format alone: object keys are field numbers.
    pub schema_less: bool,
    /// Why this message didn't decode (it then shows schema-less, or as
    /// Base64).
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct Badge {
    /// JSON pointer of the value, `""` for the root.
    pub pointer: String,
    pub label: String,
}

/// A gRPC call's outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct GrpcStatus {
    pub code: u8,
    /// `OK`, `NOT_FOUND`, … (`UNKNOWN(<n>)` past 16).
    pub name: String,
    /// `grpc-message`, percent-decoded.
    pub message: Option<String>,
    /// `grpc-status-details-bin` decoded as `google.rpc.Status`, as JSON
    /// text.
    pub details: Option<String>,
}

/// An imported schema: `.proto` files compiled in Rust, or a
/// `FileDescriptorSet`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ProtoSchema {
    pub id: u32,
    pub name: String,
    /// The `.proto` files it defines (imports from other schemas and the
    /// well-known types aren't listed).
    pub files: Vec<String>,
    /// Fully qualified service names.
    pub services: Vec<String>,
    /// Number of message types it defines.
    pub messages: u32,
    pub added_at_ms: u64,
}

/// "Decode as…": protobuf bodies matching `matcher` (and `part`, when set)
/// decode as `message_type`. gRPC needs none: its path names the method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct ProtoMapping {
    pub id: String,
    pub matcher: Matcher,
    #[serde(default)]
    pub part: Option<BodyPart>,
    /// Fully qualified, e.g. `shop.v1.Order`.
    pub message_type: String,
}
