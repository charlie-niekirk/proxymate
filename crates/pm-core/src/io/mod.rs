//! Body interpretation (content detection, decoding, event-stream
//! parsing) and the file formats: cURL commands, HAR 1.2 and saved
//! `.proxymate` sessions.

pub mod content;
pub mod curl;
pub mod decode;
pub mod grpc;
pub mod har;
pub mod objects;
pub mod session;
pub mod sse;
pub mod tree;
pub mod wire;
