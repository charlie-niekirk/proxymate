//! App update settings (M11). The updater itself lives in the desktop app;
//! the engine only keeps its settings in `app.sqlite`.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct UpdateSettings {
    /// Look for a new release quietly at launch; only an available update
    /// is shown. "Check for Updates…" works either way.
    pub check_at_launch: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        UpdateSettings { check_at_launch: true }
    }
}
