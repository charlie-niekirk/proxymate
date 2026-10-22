//! Port → pid with a libproc file-descriptor scan, pid → app bundle.
//!
//! `struct socket_fdinfo` (xnu `bsd/sys/proc_info.h`) is read at fixed
//! offsets from a byte buffer instead of being redeclared field by field;
//! `finds_its_own_connection` checks the offsets against the running kernel.
//! A full scan takes about 3 ms on a busy Mac.

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::PlatformError;
use crate::procinfo::{AppInfo, ProcessInfo, app_name, is_helper, is_simulator, outermost_app};

const PROC_PIDFDSOCKETINFO: libc::c_int = 3;
const PROC_PIDPATHINFO_MAXSIZE: usize = 4 * 1024;
const SOCKINFO_TCP: i32 = 2;
const INI_IPV4: u8 = 0x1;

// Offsets into `struct socket_fdinfo`: `proc_fileinfo` (24 bytes), then
// `socket_info` whose `vinfo_stat` is 136 bytes.
const SOI_PROTOCOL: usize = 180;
const SOI_KIND: usize = 256;
const INSI_FPORT: usize = 264;
const INSI_LPORT: usize = 268;
const INSI_VFLAG: usize = 288;
const INSI_FADDR: usize = 296;
/// `sizeof(struct socket_fdinfo)` is 792; the kernel rejects smaller buffers.
const SOCKET_FDINFO_BUF: usize = 1024;

pub struct LibProc;

impl ProcessInfo for LibProc {
    fn loopback_peers(&self, remote_port: u16) -> Result<HashMap<u16, u32>, PlatformError> {
        let mut peers = HashMap::new();
        for pid in all_pids()? {
            // Other users' processes (root daemons) fail with EPERM and are
            // skipped: we couldn't name them anyway.
            let Some(fds) = socket_fds(pid) else { continue };
            for fd in fds {
                if let Some(local) = tcp_peer_port(pid, fd, remote_port) {
                    peers.insert(local, pid as u32);
                }
            }
        }
        Ok(peers)
    }

    fn listener_pid(&self, port: u16) -> Option<u32> {
        let me = std::process::id() as libc::c_int;
        all_pids().ok()?.into_iter().filter(|&pid| pid != me).find_map(|pid| {
            socket_fds(pid)?
                .into_iter()
                .any(|fd| socket_info(pid, fd).is_some_and(|info| listens_on(&info, port)))
                .then_some(pid as u32)
        })
    }

    fn app_for_pid(&self, pid: u32) -> Option<AppInfo> {
        let path = pid_path(pid)?;
        let simulator = is_simulator(&path);
        if let Some(bundle) = outermost_app(&path) {
            return Some(bundle_info(bundle, pid, simulator));
        }
        if is_helper(&path)
            && let Some(owner) = responsible_pid(pid).filter(|&p| p != pid)
            && let Some(bundle) = pid_path(owner).and_then(|p| outermost_app(&p))
        {
            return Some(bundle_info(bundle, owner, simulator));
        }
        Some(AppInfo {
            name: path.file_name()?.to_string_lossy().into_owned(),
            bundle_id: None,
            bundle_path: None,
            pid,
            simulator,
        })
    }

    fn icon_png(&self, bundle_path: &Path, size: u32) -> Option<Vec<u8>> {
        super::icon::icon_png(bundle_path, size)
    }
}

fn bundle_info(bundle: PathBuf, pid: u32, simulator: bool) -> AppInfo {
    let bundle_id = plist::Value::from_file(bundle.join("Contents/Info.plist"))
        .ok()
        .and_then(|info| {
            info.as_dictionary()?
                .get("CFBundleIdentifier")?
                .as_string()
                .map(str::to_owned)
        });
    AppInfo {
        name: app_name(&bundle),
        bundle_id,
        bundle_path: Some(bundle),
        pid,
        simulator,
    }
}

#[allow(unsafe_code)] // libproc FFI into buffers owned here.
fn all_pids() -> Result<Vec<libc::c_int>, PlatformError> {
    // SAFETY: a null buffer asks for the count only.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // Room for processes started between the two calls.
    let mut pids: Vec<libc::c_int> = vec![0; count as usize + 64];
    let bytes = (pids.len() * size_of::<libc::c_int>()) as libc::c_int;
    // SAFETY: the buffer holds `bytes` bytes.
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast::<c_void>(), bytes) };
    if n <= 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    pids.truncate(n as usize);
    Ok(pids)
}

/// The socket descriptors of `pid`, or `None` if we may not look.
#[allow(unsafe_code)] // libproc FFI into buffers owned here.
fn socket_fds(pid: libc::c_int) -> Option<Vec<libc::c_int>> {
    // SAFETY: a null buffer asks for the size only.
    let size = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if size <= 0 {
        return None;
    }
    let entry = size_of::<libc::proc_fdinfo>();
    // Room for descriptors opened between the two calls.
    let mut fds: Vec<libc::proc_fdinfo> = Vec::with_capacity(size as usize / entry + 16);
    let bytes = (fds.capacity() * entry) as libc::c_int;
    // SAFETY: the buffer has capacity for `bytes` bytes; the kernel returns
    // how many it wrote, and only those entries are exposed.
    let written = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, fds.as_mut_ptr().cast(), bytes) };
    if written <= 0 {
        return None;
    }
    // SAFETY: the kernel initialised `written` bytes of whole entries.
    unsafe { fds.set_len(written as usize / entry) };
    Some(
        fds.iter()
            .filter(|fd| fd.proc_fdtype == libc::PROX_FDTYPE_SOCKET as u32)
            .map(|fd| fd.proc_fd)
            .collect(),
    )
}

/// The local port of `fd` if it is a TCP socket connected to loopback
/// `remote_port`.
fn tcp_peer_port(pid: libc::c_int, fd: libc::c_int, remote_port: u16) -> Option<u16> {
    socket_peer_port(&socket_info(pid, fd)?, remote_port)
}

/// The `socket_fdinfo` of `fd`, as bytes.
#[allow(unsafe_code)] // libproc FFI into a buffer owned here.
fn socket_info(pid: libc::c_int, fd: libc::c_int) -> Option<Vec<u8>> {
    let mut buf = [0u64; SOCKET_FDINFO_BUF / 8];
    // SAFETY: the buffer is SOCKET_FDINFO_BUF bytes and 8-byte aligned.
    let n = unsafe {
        libc::proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDSOCKETINFO,
            buf.as_mut_ptr().cast::<c_void>(),
            SOCKET_FDINFO_BUF as libc::c_int,
        )
    };
    if n <= INSI_FADDR as libc::c_int + 16 {
        return None;
    }
    Some(buf.iter().flat_map(|w| w.to_ne_bytes()).collect())
}

/// True for a TCP socket bound to local `port` with no peer: a listener.
fn listens_on(info: &[u8], port: u16) -> bool {
    let i32_at = |at: usize| i32::from_ne_bytes(info[at..at + 4].try_into().unwrap());
    let port_at = |at: usize| u16::from_be(i32_at(at) as u16);
    i32_at(SOI_PROTOCOL) == libc::IPPROTO_TCP
        && i32_at(SOI_KIND) == SOCKINFO_TCP
        && port_at(INSI_FPORT) == 0
        && port_at(INSI_LPORT) == port
}

/// Parses a `socket_fdinfo` buffer.
fn socket_peer_port(info: &[u8], remote_port: u16) -> Option<u16> {
    let i32_at = |at: usize| i32::from_ne_bytes(info[at..at + 4].try_into().unwrap());
    if i32_at(SOI_PROTOCOL) != libc::IPPROTO_TCP || i32_at(SOI_KIND) != SOCKINFO_TCP {
        return None;
    }
    // Ports are stored in network byte order in the low 16 bits.
    let port_at = |at: usize| u16::from_be(i32_at(at) as u16);
    if port_at(INSI_FPORT) != remote_port {
        return None;
    }
    let faddr = &info[INSI_FADDR..INSI_FADDR + 16];
    let loopback = if info[INSI_VFLAG] & INI_IPV4 != 0 {
        faddr[12] == 127
    } else {
        let v6 = std::net::Ipv6Addr::from(<[u8; 16]>::try_from(faddr).unwrap());
        v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
    };
    loopback.then(|| port_at(INSI_LPORT))
}

#[allow(unsafe_code)] // libproc FFI into a buffer owned here.
fn pid_path(pid: u32) -> Option<PathBuf> {
    let mut buf = vec![0u8; PROC_PIDPATHINFO_MAXSIZE];
    // SAFETY: the buffer holds PROC_PIDPATHINFO_MAXSIZE bytes.
    let n = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(PathBuf::from(String::from_utf8_lossy(&buf).into_owned()))
}

/// The process macOS holds responsible for `pid` (what Activity Monitor
/// groups it under): Safari for its WebKit network process. A private but
/// long-stable libsystem call, looked up at runtime so a missing symbol
/// only costs the attribution.
#[allow(unsafe_code)] // dlsym and a call through the found function pointer.
fn responsible_pid(pid: u32) -> Option<u32> {
    type Responsible = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;
    static FUNC: OnceLock<Option<Responsible>> = OnceLock::new();
    let func = FUNC.get_or_init(|| {
        // SAFETY: dlsym with a NUL-terminated name; the symbol, if present,
        // has the declared signature.
        unsafe {
            let sym = libc::dlsym(
                libc::RTLD_DEFAULT,
                c"responsibility_get_pid_responsible_for_pid".as_ptr(),
            );
            (!sym.is_null()).then(|| std::mem::transmute::<*mut c_void, Responsible>(sym))
        }
    });
    // SAFETY: takes and returns a pid; no memory is involved.
    let owner = unsafe { (*func)?(pid as libc::pid_t) };
    u32::try_from(owner).ok().filter(|&p| p > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn finds_its_own_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _server = listener.accept().unwrap();
        let local = client.local_addr().unwrap().port();
        let peers = LibProc.loopback_peers(port).unwrap();
        assert_eq!(peers.get(&local), Some(&std::process::id()));
        // The accepted side (local port = `port`) is not a client of `port`.
        assert_eq!(peers.len(), 1, "{peers:?}");
    }

    #[test]
    fn finds_a_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Our own process is skipped (the proxy asks about others), so
        // look at the socket directly.
        let me = std::process::id() as libc::c_int;
        let found = socket_fds(me)
            .unwrap()
            .into_iter()
            .any(|fd| socket_info(me, fd).is_some_and(|info| listens_on(&info, port)));
        assert!(found);
        // Not `None`: while a parallel test spawns `sleep` or `true`, the
        // child briefly holds copies of our fds (this listener included)
        // until exec closes them, and it is another pid that really listens.
        assert_ne!(LibProc.listener_pid(port), Some(std::process::id()));
    }

    #[test]
    fn names_its_own_process() {
        let app = LibProc.app_for_pid(std::process::id()).unwrap();
        assert!(app.name.starts_with("pm_platform"), "{app:?}");
        assert_eq!(app.bundle_id, None);
    }

    #[test]
    fn reads_bundles() {
        let finder = bundle_info(PathBuf::from("/System/Library/CoreServices/Finder.app"), 1, false);
        assert_eq!(finder.name, "Finder");
        assert_eq!(finder.bundle_id.as_deref(), Some("com.apple.finder"));
        let png = LibProc.icon_png(finder.bundle_path.as_ref().unwrap(), 64).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
