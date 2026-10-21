//! Which CONNECT hosts are decrypted, and whose upstream certificate is
//! accepted unverified. Compiled from [`SslSettings`] host globs.

use std::path::Path;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use pm_model::SslSettings;

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("bad host pattern {pattern:?}: {source}")]
    Glob { pattern: String, source: globset::Error },
    #[error("SSL settings file: {0}")]
    Io(#[from] std::io::Error),
    #[error("SSL settings file: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct SslPolicy {
    settings: SslSettings,
    include: GlobSet,
    /// `exclude` and `bypass` together.
    exclude: GlobSet,
    insecure: GlobSet,
}

impl SslPolicy {
    pub fn compile(settings: SslSettings) -> Result<SslPolicy, PolicyError> {
        let exclude: Vec<&String> = settings.exclude.iter().chain(&settings.bypass).collect();
        Ok(SslPolicy {
            include: globs(&settings.include)?,
            exclude: globs(exclude)?,
            insecure: globs(&settings.insecure_upstream)?,
            settings,
        })
    }

    pub fn settings(&self) -> &SslSettings {
        &self.settings
    }

    /// True if a CONNECT to `host` should be decrypted.
    pub fn intercepts(&self, host: &str) -> bool {
        let host = normalize(host);
        self.include.is_match(&host) && !self.exclude.is_match(&host)
    }

    /// True if `host`'s upstream certificate is accepted without verification.
    pub fn insecure_upstream(&self, host: &str) -> bool {
        self.insecure.is_match(normalize(host))
    }

    /// Settings saved by [`SslPolicy::save`], or the defaults.
    pub fn load(path: &Path) -> Result<SslSettings, PolicyError> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(SslSettings::default()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), PolicyError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = pm_platform::fs::temp_sibling(path);
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.settings)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}

fn normalize(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn globs<'a>(patterns: impl IntoIterator<Item = &'a String>) -> Result<GlobSet, PolicyError> {
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            continue;
        }
        let glob = GlobBuilder::new(pattern)
            .case_insensitive(true)
            .literal_separator(false)
            .build()
            .map_err(|source| PolicyError::Glob {
                pattern: pattern.to_owned(),
                source,
            })?;
        set.add(glob);
    }
    set.build().map_err(|source| PolicyError::Glob {
        pattern: String::new(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_decrypt_everything_but_apple_and_ocsp() {
        let policy = SslPolicy::compile(SslSettings::default()).unwrap();
        assert!(policy.intercepts("example.com"));
        assert!(policy.intercepts("api.github.com"));
        assert!(policy.intercepts("localhost"));
        assert!(policy.intercepts("127.0.0.1"));
        for host in [
            "apple.com",
            "gateway.icloud.com",
            "Push.Apple.com.",
            "ocsp.digicert.com",
            "ocsp2.apple.com",
        ] {
            assert!(!policy.intercepts(host), "{host}");
        }
    }

    #[test]
    fn bypass_and_insecure_lists_apply() {
        let settings = SslSettings {
            include: vec!["*.example.com".into()],
            bypass: vec!["pinned.example.com".into()],
            insecure_upstream: vec!["self-signed.test".into()],
            ..SslSettings::default()
        };
        let policy = SslPolicy::compile(settings).unwrap();
        assert!(policy.intercepts("api.example.com"));
        assert!(!policy.intercepts("pinned.example.com"));
        assert!(!policy.intercepts("example.org"));
        assert!(policy.insecure_upstream("SELF-SIGNED.test"));
        assert!(!policy.insecure_upstream("example.com"));
    }

    #[test]
    fn bad_patterns_are_rejected() {
        let settings = SslSettings {
            exclude: vec!["[unclosed".into()],
            ..SslSettings::default()
        };
        assert!(matches!(SslPolicy::compile(settings), Err(PolicyError::Glob { .. })));
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let path = std::env::temp_dir().join(format!("pm-ssl-{}.json", uuid::Uuid::new_v4()));
        assert_eq!(SslPolicy::load(&path).unwrap(), SslSettings::default());
        let mut settings = SslSettings::default();
        settings.bypass.push("pinned.example.com".into());
        SslPolicy::compile(settings.clone()).unwrap().save(&path).unwrap();
        assert_eq!(SslPolicy::load(&path).unwrap(), settings);
        std::fs::remove_file(path).unwrap();
    }
}
