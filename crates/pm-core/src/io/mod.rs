//! Body interpretation: content detection, decoding and event-stream
//! parsing. HAR, cURL and session import/export join this module in M8.

pub mod content;
pub mod decode;
pub mod sse;
