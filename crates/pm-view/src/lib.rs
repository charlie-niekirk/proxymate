//! UI logic for the native app that needs neither gpui nor the engine's
//! internals, so it is tested as plain Rust. Each module ports a file of
//! `apps/desktop/ui/src/lib`, with that file's vitest table as Rust tests:
//! `dsl.ts` and `copyFormats.ts` (G2), `selection.ts` and `keys.ts` (G1),
//! `errors.ts` (G3), `json/doc.ts` (G8), `rewrite/*` (G9), `compare.ts`
//! (G11) and the body helpers (G4, G5). It depends on `pm-model` for the
//! types that cross from the engine once the first port lands.
//!
//! G0 creates the crate empty, so CI builds and tests it from the start.
