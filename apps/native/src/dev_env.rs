//! The `PROXYMATE_DEV_*` environment variables (the native app's
//! `VITE_PM_*`): read only in debug builds or with the `dev-harness`
//! feature, so a shipped build ignores them.

/// Whether dev variables are honoured in this build.
pub const fn enabled() -> bool {
    cfg!(any(debug_assertions, feature = "dev-harness"))
}

/// `name=1`.
pub fn flag(name: &str) -> bool {
    enabled() && std::env::var(name).as_deref() == Ok("1")
}

/// `name=<value>` (`PROXYMATE_DEV_FILTER=host:example.com`).
pub fn value(name: &str) -> Option<String> {
    if !enabled() {
        return None;
    }
    std::env::var(name).ok().filter(|v| !v.is_empty())
}
