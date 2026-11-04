//! File dialogs, shared by the commands that save or open files.

pub(super) async fn save_path(
    app: &tauri::AppHandle,
    path: Option<String>,
    file_name: &str,
    kind: &str,
    extension: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    if let Some(path) = path {
        return Ok(Some(path.into()));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(kind, &[extension])
        .set_file_name(file_name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    match rx.await.ok().flatten() {
        Some(path) => path.into_path().map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// Picks one file with a dialog; `None` if it was cancelled.
pub(super) async fn pick_file(
    app: &tauri::AppHandle,
    title: &str,
    filter: (&str, &[&str]),
) -> Result<Option<std::path::PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(title)
        .add_filter(filter.0, filter.1)
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    match rx.await.ok().flatten() {
        Some(path) => path.into_path().map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// Asks for a file (or, with `folder`, a folder) with the OS dialog, for a
/// path field such as Map Local's. `extensions` limits the files offered
/// (empty: any file); the dialog opens beside `current` when it's set.
/// `None` if it was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn choose_path(
    app: tauri::AppHandle,
    title: String,
    folder: bool,
    extensions: Vec<String>,
    current: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file().set_title(title);
    if !folder && !extensions.is_empty() {
        let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(extensions.join(", "), &extensions);
    }
    if let Some(current) = current.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        let current = std::path::Path::new(current);
        let dir = if folder || current.is_dir() {
            Some(current)
        } else {
            current.parent()
        };
        if let Some(dir) = dir.filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
    }
    let send = move |path| {
        let _ = tx.send(path);
    };
    if folder {
        dialog.pick_folder(send);
    } else {
        dialog.pick_file(send);
    }
    match rx.await.ok().flatten() {
        Some(path) => path
            .into_path()
            .map(|p| Some(p.display().to_string()))
            .map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// Picks the auto-save folder with a dialog; `None` if it was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn choose_autosave_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Auto-save folder")
        .pick_folder(move |path| {
            let _ = tx.send(path);
        });
    match rx.await.ok().flatten() {
        Some(path) => path
            .into_path()
            .map(|p| Some(p.display().to_string()))
            .map_err(|e| e.to_string()),
        None => Ok(None),
    }
}
