//! UI logic for the native app that needs neither gpui nor the engine's
//! internals, so it is tested as plain Rust. Each module ports a file of
//! `apps/desktop/ui/src`, with that file's vitest table as Rust tests:
//! `selection.ts`, `FlowTable/keys.ts`, `FlowTable/columns.ts` (with the
//! column half of `store/prefs.ts`), `lib/flow.ts` and `lib/rowCache.ts`
//! (G1); `dsl.ts`, `copyFormats.ts` and the command list of `commands.ts` (G2); `errors.ts`, `http.ts`, `tls.ts` and the inspector's
//! menus and Raw tab (G3, with [`sample`] fixtures); `stream.ts` and the
//! WebSocket and SSE panels' logic (G6); `rules.ts` (G7); `devices.ts` and the toolbar's integration surfaces (G13); `json/doc.ts`
//! (G8), `rewrite/*` (G9), `compare.ts` (G11) and the body helpers (G4, G5).

pub mod body;
pub mod columns;
pub mod commands;
pub mod copy_formats;
pub mod devices;
pub mod dsl;
pub mod errors;
pub mod flow;
pub mod http;
pub mod inspector;
pub mod integration;
pub mod keys;
pub mod row_cache;
pub mod rules;
pub mod sample;
pub mod selection;
pub mod stream;
pub mod tls;
