//! What persists across launches: `<data dir>/ui-prefs.json` (the WebView's
//! localStorage is not migrated; the defaults apply once). The appearance
//! (G0) and the flow table's columns, sort and header columns (G1), with
//! the table's keys spelt as the Tauri UI's `store/prefs.ts` spells them.
//!
//! Changes apply at once and are written a moment later (a column drag
//! changes the layout many times a second), and on quit.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Global};
use pm_view::columns::TablePrefs;
use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "ui-prefs.json";
/// How long a change waits before it is written, so a burst writes once.
const SAVE_DELAY: Duration = Duration::from_millis(300);

/// Settings › Appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub appearance: Appearance,
    /// `columns`, `sort` and `headerColumns`.
    #[serde(flatten)]
    pub table: TablePrefs,
}

/// The prefs and where they're saved (`None`: in memory only).
pub struct Stored {
    pub prefs: Prefs,
    path: Option<PathBuf>,
    save_pending: bool,
}

impl Global for Stored {}

pub fn init(data_dir: Option<PathBuf>, cx: &mut App) {
    let path = data_dir.map(|dir| dir.join(FILE_NAME));
    let prefs = path.as_deref().map(load).unwrap_or_default();
    cx.set_global(Stored {
        prefs,
        path,
        save_pending: false,
    });
}

pub fn get(cx: &App) -> &Prefs {
    &cx.global::<Stored>().prefs
}

/// Changes the prefs; they are written shortly after (off the foreground
/// thread), once for a burst of changes.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Prefs)) {
    let stored = cx.global_mut::<Stored>();
    change(&mut stored.prefs);
    if stored.path.is_none() || stored.save_pending {
        return;
    }
    stored.save_pending = true;
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SAVE_DELAY).await;
        let pending = cx.update(|cx| {
            let stored = cx.global_mut::<Stored>();
            stored.save_pending = false;
            stored.path.clone().map(|path| (path, stored.prefs.clone()))
        });
        if let Some((path, prefs)) = pending {
            cx.background_spawn(async move { write(&path, &prefs) }).await;
        }
    })
    .detach();
}

/// Writes now (quit: a change made in the last moment must not be lost).
pub fn flush(cx: &mut App) {
    let Some(stored) = cx.try_global::<Stored>() else {
        return;
    };
    if let Some(path) = &stored.path {
        write(path, &stored.prefs);
    }
}

fn write(path: &std::path::Path, prefs: &Prefs) {
    if let Err(err) = save(path, prefs) {
        eprintln!("can't save {}: {err}", path.display());
    }
}

fn load(path: &std::path::Path) -> Prefs {
    let mut prefs = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            eprintln!("{}: {err}; using the defaults", path.display());
            Prefs::default()
        }),
        Err(_) => Prefs::default(),
    };
    prefs.table.normalize();
    prefs
}

fn save(path: &std::path::Path, prefs: &Prefs) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(prefs).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use pm_model::{BodyPart, HeaderColumn, SortDir, SortKey};
    use pm_view::columns::{Builtin, ColumnId};

    use super::*;

    #[test]
    fn prefs_round_trip_and_tolerate_unknown_or_missing_fields() {
        let dir = std::env::temp_dir().join(format!("pm-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        let mut prefs = Prefs {
            appearance: Appearance::Light,
            ..Default::default()
        };
        prefs.table.sort.key = SortKey::Duration;
        prefs.table.sort.dir = SortDir::Desc;
        prefs.table.set_width(&ColumnId::Builtin(Builtin::Url), 400.0);
        prefs.table.set_hidden(&ColumnId::Builtin(Builtin::Client), true);
        prefs.table.add_header_column(HeaderColumn {
            part: BodyPart::Response,
            name: "x-cache".into(),
        });
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path), prefs);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(r#""headerColumns""#) && text.contains(r#""hdr:response:x-cache""#),
            "{text}"
        );
        std::fs::write(&path, br#"{"columns": [], "later": 1}"#).unwrap();
        assert_eq!(load(&path), Prefs::default());
        std::fs::write(&path, br#"{"appearance": "dark", "columns": {"order": ["url", "id"]}}"#).unwrap();
        let partial = load(&path);
        assert_eq!(partial.appearance, Appearance::Dark);
        assert_eq!(partial.table.columns.order[0], ColumnId::Builtin(Builtin::Url));
        assert_eq!(partial.table.columns.order.len(), 14, "the rest join at their defaults");
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(load(&path), Prefs::default());
        assert_eq!(load(&dir.join("missing.json")), Prefs::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
