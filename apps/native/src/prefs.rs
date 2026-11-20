//! What persists across launches: `<data dir>/ui-prefs.json` (the WebView's
//! localStorage is not migrated; the defaults apply once). G0 keeps the
//! appearance; G1 adds columns and sort, G11 header columns.

use std::path::PathBuf;

use gpui_kit::{App, AppContext as _, Global};
use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "ui-prefs.json";

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
}

/// The prefs and where they're saved (`None`: in memory only).
pub struct Stored {
    pub prefs: Prefs,
    path: Option<PathBuf>,
}

impl Global for Stored {}

pub fn init(data_dir: Option<PathBuf>, cx: &mut App) {
    let path = data_dir.map(|dir| dir.join(FILE_NAME));
    let prefs = path.as_deref().map(load).unwrap_or_default();
    cx.set_global(Stored { prefs, path });
}

pub fn get(cx: &App) -> &Prefs {
    &cx.global::<Stored>().prefs
}

/// Changes the prefs and writes them out (off the foreground thread).
pub fn update(cx: &mut App, change: impl FnOnce(&mut Prefs)) {
    let stored = cx.global_mut::<Stored>();
    change(&mut stored.prefs);
    let (prefs, path) = (stored.prefs.clone(), stored.path.clone());
    if let Some(path) = path {
        cx.background_spawn(async move {
            if let Err(err) = save(&path, &prefs) {
                eprintln!("can't save {}: {err}", path.display());
            }
        })
        .detach();
    }
}

fn load(path: &std::path::Path) -> Prefs {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            eprintln!("{}: {err}; using the defaults", path.display());
            Prefs::default()
        }),
        Err(_) => Prefs::default(),
    }
}

fn save(path: &std::path::Path, prefs: &Prefs) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(prefs).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_tolerate_unknown_or_missing_fields() {
        let dir = std::env::temp_dir().join(format!("pm-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        let prefs = Prefs {
            appearance: Appearance::Light,
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path), prefs);
        std::fs::write(&path, br#"{"columns": [], "later": 1}"#).unwrap();
        assert_eq!(load(&path), Prefs::default());
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(load(&path), Prefs::default());
        assert_eq!(load(&dir.join("missing.json")), Prefs::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
