//! Waiting for a process to exit with kqueue `NOTE_EXIT`.

use crate::PlatformError;

#[allow(unsafe_code)] // kqueue(2)/kevent(2) FFI; every buffer is a local owned here.
pub fn wait_for_exit(pid: u32) -> Result<(), PlatformError> {
    let Ok(ident) = usize::try_from(pid) else {
        return Ok(());
    };
    // SAFETY: plain syscalls on a descriptor this function owns and closes;
    // the change and event lists are valid for the duration of each call.
    unsafe {
        let kq = libc::kqueue();
        if kq < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let change = libc::kevent {
            ident,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let mut event: libc::kevent = std::mem::zeroed();
        let result = loop {
            let n = libc::kevent(kq, &change, 1, &mut event, 1, std::ptr::null());
            if n >= 0 {
                break Ok(());
            }
            let err = std::io::Error::last_os_error();
            match err.raw_os_error() {
                Some(libc::EINTR) => continue,
                // Already gone before we started watching.
                Some(libc::ESRCH) => break Ok(()),
                _ => break Err(err.into()),
            }
        };
        libc::close(kq);
        result
    }
}
