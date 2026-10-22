//! App update settings, kept in `app.sqlite` (the `updates` settings row).
//! They share the rules' connection: they're read at launch and written
//! from Settings, never on a hot path.

use pm_model::UpdateSettings;

use crate::rules::RuleError;
use crate::{Engine, Error};

const UPDATE_SETTINGS_KEY: &str = "updates";

impl Engine {
    pub fn update_settings(&self) -> UpdateSettings {
        self.rules.db().setting(UPDATE_SETTINGS_KEY).unwrap_or_else(|err| {
            eprintln!("proxymate: can't read the update settings: {err}");
            UpdateSettings::default()
        })
    }

    pub fn set_update_settings(&self, settings: UpdateSettings) -> Result<UpdateSettings, Error> {
        self.rules
            .db()
            .save_setting(UPDATE_SETTINGS_KEY, &settings)
            .map_err(RuleError::from)?;
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use pm_model::UpdateSettings;

    use crate::{Engine, EngineConfig};

    #[test]
    fn update_settings_default_on_and_persist() {
        let dir = std::env::temp_dir().join(format!("pm-updates-{}", uuid::Uuid::new_v4()));
        let config = || EngineConfig {
            data_dir: Some(dir.clone()),
            ..EngineConfig::default()
        };
        let engine = Engine::with_config(config());
        assert!(engine.update_settings().check_at_launch);
        let off = UpdateSettings { check_at_launch: false };
        engine.set_update_settings(off.clone()).unwrap();
        engine.shutdown();
        let reopened = Engine::with_config(config());
        assert_eq!(reopened.update_settings(), off);
        reopened.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }
}
