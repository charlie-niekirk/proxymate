//! Files only the current user may read, and where the app keeps its data.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `contents` to `path` atomically (a sibling temp file, then a
/// rename), readable and writable by the owner only (mode 0600 on Unix). On
/// Windows the file inherits the profile directory's ACL, which is already
/// per-user.
pub fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let tmp = temp_sibling(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

/// A temp file next to `path`, unique per process and per call, so
/// concurrent writers never share one.
pub fn temp_sibling(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}-{n}", std::process::id()));
    path.with_file_name(name)
}

/// The app's data directory, as the desktop app resolves it (Tauri's data
/// dir plus `Proxymate`), for processes started without the app: the
/// `mcp-stdio` bridge reads the MCP token from there.
pub fn app_data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home?.join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".local/share")))?
    };
    Some(base.join("Proxymate"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_files_are_owner_only() {
        let dir = std::env::temp_dir().join(format!("pm-platform-fs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("key.pem");
        write_private(&path, b"secret").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"secret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
