//! Offline mocking (M20): the mock library that `Action::ServeFrom`
//! answers from, and imported OpenAPI specs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

/// A recorded session copied into the mock library
/// (`<data dir>/mocks/<id>.proxymate`), so a rule serving from it keeps
/// working when the original file moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct MockSource {
    pub id: String,
    pub name: String,
    /// The session, HAR or Charles file (or the live capture) it was
    /// copied from.
    pub origin: String,
    /// Flows with a recorded response, which are the ones served.
    pub flows: u32,
    /// The hosts of those flows, sorted.
    pub hosts: Vec<String>,
    /// Milliseconds since the Unix epoch.
    pub created_at_ms: f64,
    /// The library file's size in bytes.
    pub size: u64,
}

/// An imported OpenAPI 3.0 or 3.1 spec, stored in `app.sqlite`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct OpenApiSpec {
    pub id: u32,
    /// `info.title`, or the file name.
    pub name: String,
    /// `info.version`.
    pub version: String,
    /// The `openapi` field, e.g. `3.0.3`.
    pub openapi: String,
    pub operations: u32,
    /// `servers[].url`, as written.
    pub servers: Vec<String>,
    /// Milliseconds since the Unix epoch.
    pub added_at_ms: f64,
}

/// The spec operation a flow's method and path match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, JsonSchema)]
pub struct OperationMatch {
    pub spec_id: u32,
    pub spec_name: String,
    pub operation_id: Option<String>,
    pub summary: Option<String>,
    /// Upper case.
    pub method: String,
    /// The path template, e.g. `/pets/{petId}`.
    pub path: String,
}
