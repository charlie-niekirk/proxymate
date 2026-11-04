//! Protobuf schemas, mappings and decoded bodies (M16).

use pm_model::{BodyPart, DecodedBody, FlowId, ProtoMapping, ProtoSchema, StoreId};

use super::{EngineState, blocking};

/// A gRPC, protobuf, MessagePack or CBOR body decoded to JSON trees (live
/// flows too, as far as they've arrived).
#[tauri::command]
#[specta::specta]
pub async fn decode_body(
    engine: EngineState<'_>,
    store: StoreId,
    flow: FlowId,
    part: BodyPart,
) -> Result<DecodedBody, String> {
    blocking(&engine, move |e| e.decode_body_in(store, flow, part)).await
}

#[tauri::command]
#[specta::specta]
pub fn list_proto_schemas(engine: EngineState<'_>) -> Vec<ProtoSchema> {
    engine.proto_schemas()
}

/// Imports `.proto` files or one descriptor set; with no paths, asks with
/// a file dialog. `None` if the dialog was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn import_proto_schema(
    app: tauri::AppHandle,
    engine: EngineState<'_>,
    paths: Option<Vec<String>>,
) -> Result<Option<ProtoSchema>, String> {
    use tauri_plugin_dialog::DialogExt;
    let paths: Vec<std::path::PathBuf> = match paths {
        Some(paths) => paths.into_iter().map(Into::into).collect(),
        None => {
            let (tx, rx) = tokio::sync::oneshot::channel();
            app.dialog()
                .file()
                .add_filter("Protobuf schema", &["proto", "pb", "binpb", "desc", "protoset"])
                .pick_files(move |paths| {
                    let _ = tx.send(paths);
                });
            let Some(picked) = rx.await.ok().flatten() else {
                return Ok(None);
            };
            picked
                .into_iter()
                .map(|p| p.into_path().map_err(|e| e.to_string()))
                .collect::<Result<_, _>>()?
        }
    };
    blocking(&engine, move |e| e.import_proto_files(&paths)).await.map(Some)
}

#[tauri::command]
#[specta::specta]
pub async fn remove_proto_schema(engine: EngineState<'_>, id: u32) -> Result<(), String> {
    blocking(&engine, move |e| e.remove_proto_schema(id)).await
}

/// Every message type the schemas define, for "Decode as…".
#[tauri::command]
#[specta::specta]
pub fn list_proto_message_types(engine: EngineState<'_>) -> Vec<String> {
    engine.proto_message_types()
}

#[tauri::command]
#[specta::specta]
pub fn list_proto_mappings(engine: EngineState<'_>) -> Vec<ProtoMapping> {
    engine.proto_mappings()
}

#[tauri::command]
#[specta::specta]
pub async fn set_proto_mappings(engine: EngineState<'_>, mappings: Vec<ProtoMapping>) -> Result<(), String> {
    blocking(&engine, move |e| e.set_proto_mappings(mappings)).await
}
