//! Putting a verified update in place of the running app bundle, as
//! tauri-plugin-updater does on macOS:
//!
//! 1. Unpack the archive (`Proxymate.app/…`) into a temp dir next to the
//!    bundle, so the swap is one rename on one volume. Where that folder
//!    isn't writable (a standard user's `/Applications`), unpack into the
//!    temp dir instead if it is on the same volume.
//! 2. Exchange the new bundle and the old one in one step
//!    (`pm_platform::fs::swap`), so the app's path is never empty; where
//!    the file system can't, move the old one aside, move the new one in,
//!    and put the old one back if that fails.
//! 3. If the folder needs an administrator, ask for one through
//!    `osascript` (the system's password prompt) to do the moves.
//!
//! The old bundle goes with the temp dir. The running process keeps its
//! executable open, and the restart launches the new bundle at the old path.

use std::io;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;

/// The `.app` bundle `exe` runs from (`…/Proxymate.app/Contents/MacOS/x`),
/// or `None` when it isn't in one (`cargo run`).
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let named = |p: &Path, name: &str| p.file_name().is_some_and(|n| n == name);
    let is_app = bundle.extension().is_some_and(|e| e == "app");
    (named(macos, "MacOS") && named(contents, "Contents") && is_app).then(|| bundle.to_path_buf())
}

/// Replaces `bundle` with the app in `archive` (a verified `.app.tar.gz`).
pub fn install(bundle: &Path, archive: &[u8]) -> io::Result<()> {
    let parent = bundle
        .parent()
        .ok_or_else(|| io::Error::other(format!("{} has no folder", bundle.display())))?;
    let (stage, privileged) = match stage_in(parent) {
        Ok(stage) => (stage, false),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => (stage_on_same_volume(parent)?, true),
        Err(err) => return Err(err),
    };
    let staged = unpack(archive, stage.path())?;
    if privileged {
        return install_as_admin(bundle, &staged, &stage.path().join("previous.app"));
    }
    match pm_platform::fs::swap(&staged, bundle) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            install_as_admin(bundle, &staged, &stage.path().join("previous.app"))?;
        }
        Err(_) => replace(bundle, &staged, &stage.path().join("previous.app"))?,
    }
    // LaunchServices notices the new bundle (its Info.plist) by its date.
    let _ = std::process::Command::new("touch").arg(bundle).status();
    Ok(())
}

/// The temp dir the update is unpacked in, next to the bundle.
fn stage_in(dir: &Path) -> io::Result<tempfile::TempDir> {
    tempfile::Builder::new().prefix(".proxymate-update-").tempdir_in(dir)
}

/// The system temp dir, if it's on `dir`'s volume (renames can't cross).
fn stage_on_same_volume(dir: &Path) -> io::Result<tempfile::TempDir> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let tmp = std::env::temp_dir();
        if std::fs::metadata(&tmp)?.dev() == std::fs::metadata(dir)?.dev() {
            return stage_in(&tmp);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} isn't writable", dir.display()),
    ))
}

/// Unpacks `archive` into `stage` and returns the one bundle in it. Paths
/// can't leave `stage` (`unpack_in`), and anything but a single top-level
/// `.app` folder is refused.
fn unpack(archive: &[u8], stage: &Path) -> io::Result<PathBuf> {
    let mut tar = tar::Archive::new(GzDecoder::new(archive));
    tar.set_preserve_permissions(true);
    let mut top: Option<PathBuf> = None;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let first = match path.components().next() {
            Some(Component::Normal(first)) => PathBuf::from(first),
            _ => return Err(bad_archive("an entry outside the bundle")),
        };
        if top.get_or_insert_with(|| first.clone()) != &first {
            return Err(bad_archive("more than one top-level entry"));
        }
        if !entry.unpack_in(stage)? {
            return Err(bad_archive("an entry outside the bundle"));
        }
    }
    let top = top.ok_or_else(|| bad_archive("nothing in it"))?;
    let bundle = stage.join(&top);
    let is_app = top.extension().is_some_and(|e| e == "app");
    if !is_app || !bundle.join("Contents/Info.plist").is_file() || !bundle.join("Contents/MacOS").is_dir() {
        return Err(bad_archive("no app bundle in it"));
    }
    #[cfg(unix)]
    {
        // Whatever the archive said, everyone may open the app, as before.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bundle, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(bundle)
}

fn bad_archive(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("the update archive has {why}"))
}

/// Two renames, undoing the first if the second fails, so `target` ends up
/// either the new bundle or the old one.
fn replace(target: &Path, staged: &Path, backup: &Path) -> io::Result<()> {
    std::fs::rename(target, backup)?;
    if let Err(err) = std::fs::rename(staged, target) {
        let _ = std::fs::rename(backup, target);
        return Err(err);
    }
    Ok(())
}

/// The same moves as [`replace`], as an administrator: macOS asks for a
/// password. The old bundle is deleted once the new one is in place.
#[cfg(target_os = "macos")]
fn install_as_admin(target: &Path, staged: &Path, backup: &Path) -> io::Result<()> {
    let (target, staged, backup) = (sh_quote(target), sh_quote(staged), sh_quote(backup));
    let script = format!(
        "mv -f {target} {backup} && {{ mv -f {staged} {target} || {{ mv -f {backup} {target}; exit 1; }}; }} && rm -rf {backup}"
    );
    let apple_script = format!(
        "do shell script \"{}\" with administrator privileges",
        script.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &apple_script])
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        let why = String::from_utf8_lossy(&output.stderr);
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("couldn't replace the app as an administrator: {}", why.trim()),
        ))
    }
}

#[cfg(not(target_os = "macos"))]
fn install_as_admin(target: &Path, _: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} isn't writable", target.display()),
    ))
}

/// `path` as one `sh` word.
#[cfg(target_os = "macos")]
fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;

    /// `Proxymate.app` with a marker, at `dir`.
    fn bundle(dir: &Path, marker: &str) -> PathBuf {
        let app = dir.join("Proxymate.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), marker).unwrap();
        std::fs::write(app.join("Contents/MacOS/proxymate"), marker).unwrap();
        app
    }

    fn archive_of(app: &Path) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        builder.append_dir_all("Proxymate.app", app).unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn the_bundle_is_found_from_its_executable() {
        assert_eq!(
            bundle_of(Path::new("/Applications/Proxymate.app/Contents/MacOS/proxymate")),
            Some(PathBuf::from("/Applications/Proxymate.app"))
        );
        assert_eq!(bundle_of(Path::new("/repo/target/debug/proxymate-native")), None);
        assert_eq!(bundle_of(Path::new("/x/Proxymate/Contents/MacOS/proxymate")), None);
    }

    #[test]
    fn install_swaps_the_new_bundle_in_and_leaves_nothing_behind() {
        let source = tempfile::tempdir().unwrap();
        let archive = archive_of(&bundle(source.path(), "new"));
        let install_dir = tempfile::tempdir().unwrap();
        let installed = bundle(install_dir.path(), "old");
        install(&installed, &archive).unwrap();
        assert_eq!(
            std::fs::read_to_string(installed.join("Contents/Info.plist")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(installed.join("Contents/MacOS/proxymate")).unwrap(),
            "new"
        );
        let left: Vec<_> = std::fs::read_dir(install_dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["Proxymate.app"], "the old bundle and the temp dir are gone");
    }

    #[test]
    fn the_fallback_renames_put_the_new_bundle_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let installed = bundle(dir.path(), "old");
        let stage = dir.path().join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        let staged = bundle(&stage, "new");
        replace(&installed, &staged, &stage.join("previous.app")).unwrap();
        assert_eq!(
            std::fs::read_to_string(installed.join("Contents/Info.plist")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(stage.join("previous.app/Contents/Info.plist")).unwrap(),
            "old"
        );
    }

    #[test]
    fn an_archive_without_one_bundle_changes_nothing() {
        let install_dir = tempfile::tempdir().unwrap();
        let installed = bundle(install_dir.path(), "old");
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_cksum();
        builder.append_data(&mut header, "a/file", &b"x"[..]).unwrap();
        builder.append_data(&mut header, "b/file", &b"x"[..]).unwrap();
        let two_tops = builder.into_inner().unwrap().finish().unwrap();
        let err = install(&installed, &two_tops).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let err = install(&installed, b"not gzip").unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            std::fs::read_to_string(installed.join("Contents/Info.plist")).unwrap(),
            "old"
        );
        let left: Vec<_> = std::fs::read_dir(install_dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["Proxymate.app"]);
    }
}
