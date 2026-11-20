//! One running copy per data directory (the native app; new in G0, since
//! the Tauri app lets a second copy start its own engine).
//!
//! The first copy listens on a Unix socket in the data dir. A second launch
//! connects, asks the first to come forward, and exits. A socket file that
//! nothing answers is left by a crash: it is removed and taken over.
//!
//! Elsewhere than Unix every launch is the primary (Windows waits for M12).

use std::io;
use std::path::{Path, PathBuf};

/// The socket's file name in the data dir.
pub const SOCKET_NAME: &str = "native-instance.sock";

/// What [`acquire`] found.
pub enum Instance {
    /// No other copy is running: this one owns the lock.
    Primary(Lock),
    /// Another copy is running and has been asked to come forward.
    Secondary,
}

/// The primary's lock: the listening socket. Dropping it removes the file.
pub struct Lock {
    path: PathBuf,
    #[cfg(unix)]
    listener: Option<std::os::unix::net::UnixListener>,
}

/// Takes the lock in `dir` (created if missing), or tells the running copy
/// to activate. Errors (a path too long for a socket, a read-only dir) are
/// for the caller to log; it should then start as if it were the primary.
pub fn acquire(dir: &Path) -> io::Result<Instance> {
    std::fs::create_dir_all(dir)?;
    imp::acquire(socket_path(dir))
}

/// `<dir>/native-instance.sock`, or, when that is too long for a socket
/// address (104 bytes on macOS), a name in the temp dir derived from `dir`.
pub fn socket_path(dir: &Path) -> PathBuf {
    let path = dir.join(SOCKET_NAME);
    if path.as_os_str().len() < 100 {
        return path;
    }
    // FNV-1a: stable across runs, unlike std's RandomState hasher.
    let hash = dir
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
    std::env::temp_dir().join(format!("pm-{hash:016x}.sock"))
}

impl Lock {
    /// Calls `on_activate` on a background thread each time a later launch
    /// asks this copy to come forward.
    pub fn serve(mut self, on_activate: impl Fn() + Send + 'static) {
        imp::serve(&mut self, on_activate);
        // The thread owns the listener now; keep the file until exit.
        std::mem::forget(self);
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Removes the lock's socket file at quit (a served [`Lock`] is never
/// dropped).
pub fn release(dir: &Path) {
    let _ = std::fs::remove_file(socket_path(dir));
}

const ACTIVATE: &[u8] = b"activate\n";
const OK: &[u8] = b"ok\n";

#[cfg(unix)]
mod imp {
    use std::io::{self, BufRead, BufReader, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{ACTIVATE, Instance, Lock, OK};

    pub fn acquire(path: PathBuf) -> io::Result<Instance> {
        match UnixListener::bind(&path) {
            Ok(listener) => return Ok(Instance::Primary(lock(path, listener))),
            Err(err) if err.kind() != io::ErrorKind::AddrInUse => return Err(err),
            Err(_) => {}
        }
        if ask_to_activate(&path).is_ok() {
            return Ok(Instance::Secondary);
        }
        // Nobody answered: a crashed copy's socket file.
        std::fs::remove_file(&path)?;
        Ok(Instance::Primary(lock(path.clone(), UnixListener::bind(&path)?)))
    }

    fn lock(path: PathBuf, listener: UnixListener) -> Lock {
        Lock {
            path,
            listener: Some(listener),
        }
    }

    fn ask_to_activate(path: &PathBuf) -> io::Result<()> {
        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(ACTIVATE)?;
        let mut answer = [0u8; 3];
        stream.read_exact(&mut answer)?;
        if answer == OK {
            Ok(())
        } else {
            Err(io::Error::other("unexpected answer"))
        }
    }

    pub fn serve(lock: &mut Lock, on_activate: impl Fn() + Send + 'static) {
        let Some(listener) = lock.listener.take() else { return };
        let spawned = std::thread::Builder::new()
            .name("pm-single-instance".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut line = Vec::new();
                    let read = BufReader::new(&mut stream).read_until(b'\n', &mut line);
                    if read.is_ok() && line == ACTIVATE {
                        let _ = stream.write_all(OK);
                        on_activate();
                    }
                }
            });
        if let Err(err) = spawned {
            eprintln!("can't listen for later launches: {err}");
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use std::io;
    use std::path::PathBuf;

    use super::{Instance, Lock};

    pub fn acquire(path: PathBuf) -> io::Result<Instance> {
        Ok(Instance::Primary(Lock { path }))
    }

    pub fn serve(_lock: &mut Lock, _on_activate: impl Fn() + Send + 'static) {}
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::{Instance, SOCKET_NAME, acquire};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        // Short: a socket path must fit in sun_path (104 bytes on macOS).
        let dir = std::env::temp_dir().join(format!("pm-si-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_second_launch_activates_the_first() {
        let dir = temp_dir("two");
        let Instance::Primary(lock) = acquire(&dir).unwrap() else {
            panic!("the first launch is the primary")
        };
        let (tx, rx) = mpsc::channel();
        lock.serve(move || tx.send(()).unwrap());
        assert!(matches!(acquire(&dir).unwrap(), Instance::Secondary));
        rx.recv_timeout(Duration::from_secs(5)).expect("activated");
        super::release(&dir);
        assert!(!dir.join(SOCKET_NAME).exists());
    }

    #[test]
    fn a_data_dir_too_long_for_a_socket_gets_a_short_stable_path() {
        let long = std::path::Path::new("/tmp").join("x".repeat(120));
        let path = super::socket_path(&long);
        assert!(path.as_os_str().len() < 104, "{}", path.display());
        assert_eq!(path, super::socket_path(&long));
        assert_ne!(path, super::socket_path(&long.join("other")));
        assert_eq!(
            super::socket_path(std::path::Path::new("/tmp/pm")),
            std::path::Path::new("/tmp/pm").join(SOCKET_NAME)
        );
    }

    #[test]
    fn a_crashed_copys_socket_is_taken_over() {
        let dir = temp_dir("stale");
        std::fs::create_dir_all(&dir).unwrap();
        // A bound socket whose listener is gone: what a crash leaves.
        drop(std::os::unix::net::UnixListener::bind(dir.join(SOCKET_NAME)).unwrap());
        assert!(dir.join(SOCKET_NAME).exists());
        let Instance::Primary(lock) = acquire(&dir).unwrap() else {
            panic!("a stale socket doesn't make a secondary")
        };
        drop(lock);
        assert!(!dir.join(SOCKET_NAME).exists());
    }
}
