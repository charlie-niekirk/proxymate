//! Editable JSON (G8): the lossless, ordered document with stable ids
//! ([`doc`], `lib/json/doc.ts`) and the JSON editor's logic ([`editor`],
//! `JsonEditor.tsx`). Reading only, the Body tab uses
//! [`crate::body::json_doc`], which [`doc::parse`] builds on.

pub mod doc;
pub mod editor;
