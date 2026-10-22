//! Process liveness, and waiting for another process to exit. The system
//! proxy guardian uses these to restore the proxy after a crash or SIGKILL.
//! Also launching the app by bundle id (the `mcp-stdio` bridge, M7).

use std::time::Duration;

use crate::PlatformError;

/// Starts an installed app by bundle id without waiting for it (macOS
/// `open -b`). Elsewhere: unsupported for now.
pub fn launch_app(bundle_id: &str) -> Result<(), PlatformError> {
    if !cfg!(target_os = "macos") {
        return Err(PlatformError::Unsupported);
    }
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-g", "-b", bundle_id])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::Os(format!("open -b {bundle_id} failed ({status})")))
    }
}

/// True if a process with this pid exists (it may belong to another user).
#[cfg(unix)]
#[allow(unsafe_code)] // kill(2) with signal 0 only checks; no memory is shared.
pub fn is_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 performs the permission and existence checks only.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
pub fn is_alive(_pid: u32) -> bool {
    false
}

/// Blocks until `pid` exits. Returns at once if it is already gone.
///
/// On macOS this is a kqueue `EVFILT_PROC`/`NOTE_EXIT` wait, so the caller
/// wakes within microseconds of a SIGKILL. Elsewhere it polls.
pub fn wait_for_exit(pid: u32) -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        crate::macos::process::wait_for_exit(pid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        while is_alive(pid) {
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(())
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn waits_for_a_child_to_exit() {
        let mut child = std::process::Command::new("sleep").arg("0.3").spawn().unwrap();
        let pid = child.id();
        assert!(is_alive(pid));
        let reaper = std::thread::spawn(move || child.wait());
        let start = Instant::now();
        wait_for_exit(pid).unwrap();
        let waited = start.elapsed();
        reaper.join().unwrap().unwrap();
        assert!(waited >= Duration::from_millis(200), "returned after {waited:?}");
        assert!(waited < Duration::from_secs(5));
    }

    #[test]
    fn a_dead_pid_returns_at_once() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!is_alive(pid));
        wait_for_exit(pid).unwrap();
        assert!(is_alive(std::process::id()));
    }
}
