//! UI logic for the native app that needs neither gpui nor the engine's
//! internals, so it is tested as plain Rust. Each module ports a file of
//! `apps/desktop/ui/src`, with that file's vitest table as Rust tests:
//! `selection.ts`, `FlowTable/keys.ts`, `FlowTable/columns.ts` (with the
//! column half of `store/prefs.ts`), `lib/flow.ts` and `lib/rowCache.ts`
//! (G1); `dsl.ts`, `copyFormats.ts` and the command list of `commands.ts` (G2), `errors.ts` (G3), `json/doc.ts`
//! (G8), `rewrite/*` (G9), `compare.ts` (G11) and the body helpers (G4, G5).

pub mod columns;
pub mod commands;
pub mod copy_formats;
pub mod dsl;
pub mod flow;
pub mod keys;
pub mod row_cache;
pub mod selection;
