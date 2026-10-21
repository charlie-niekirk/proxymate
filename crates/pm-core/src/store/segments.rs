//! Rotating append-only segment files under `<root>/live/<uuid>/` that hold
//! large bodies and encoded flow details. Nothing here is durable: the
//! directory is removed on clear, on shutdown and when the store is dropped,
//! and a later run sweeps directories whose owner died.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// A segment is closed and a new one started past this size.
const SEGMENT_SIZE: u64 = 256 * 1024 * 1024;
const LOCK_FILE: &str = "lock";

/// A byte range in one segment file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Extent {
    pub seg: u32,
    pub offset: u64,
    pub len: u32,
}

pub(crate) struct SegmentStore {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Held exclusively while the directory exists, so a sweep in another
    /// process can tell a live directory from an orphan.
    lock: Option<File>,
    current: Option<Writer>,
    /// Numbers keep increasing across clears, so a stale extent from a
    /// cleared flow can never alias new data.
    next_seg: u32,
}

struct Writer {
    seg: u32,
    file: BufWriter<File>,
    len: u64,
}

impl SegmentStore {
    /// A store under `<root>/live/<uuid>/`. The directory is created on the
    /// first append; orphaned siblings are removed now.
    pub fn new(root: &Path) -> Self {
        let live = root.join("live");
        sweep_orphans(&live);
        SegmentStore {
            dir: live.join(uuid::Uuid::new_v4().to_string()),
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn append(&self, bytes: &[u8]) -> io::Result<Extent> {
        let len = u32::try_from(bytes.len()).map_err(|_| io::Error::other("blob larger than 4 GiB"))?;
        let mut inner = self.inner.lock();
        if inner.current.as_ref().is_none_or(|w| w.len >= SEGMENT_SIZE) {
            self.rotate(&mut inner)?;
        }
        let writer = inner.current.as_mut().expect("rotate opened a segment");
        let extent = Extent {
            seg: writer.seg,
            offset: writer.len,
            len,
        };
        writer.file.write_all(bytes)?;
        writer.len += bytes.len() as u64;
        Ok(extent)
    }

    /// Appends the bytes of `extents` to `out`.
    pub fn read_into(&self, extents: &[Extent], out: &mut Vec<u8>) -> io::Result<()> {
        let mut open: Option<(u32, File)> = None;
        for extent in extents {
            // Buffered writes must reach the file before it is read back.
            {
                let mut inner = self.inner.lock();
                if let Some(w) = inner.current.as_mut()
                    && w.seg == extent.seg
                {
                    w.file.flush()?;
                }
            }
            let file = match &mut open {
                Some((seg, file)) if *seg == extent.seg => file,
                _ => &mut open.insert((extent.seg, File::open(self.segment_path(extent.seg))?)).1,
            };
            file.seek(SeekFrom::Start(extent.offset))?;
            let start = out.len();
            out.resize(start + extent.len as usize, 0);
            file.read_exact(&mut out[start..])?;
        }
        Ok(())
    }

    pub fn read(&self, extent: Extent) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(extent.len as usize);
        self.read_into(&[extent], &mut out)?;
        Ok(out)
    }

    /// Deletes every segment. Later appends start a new one.
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.current = None;
        for seg in 0..inner.next_seg {
            let _ = fs::remove_file(self.segment_path(seg));
        }
    }

    /// Deletes the whole directory.
    pub fn remove_all(&self) {
        let mut inner = self.inner.lock();
        inner.current = None;
        inner.lock = None;
        let _ = fs::remove_dir_all(&self.dir);
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn rotate(&self, inner: &mut Inner) -> io::Result<()> {
        if inner.lock.is_none() {
            fs::create_dir_all(&self.dir)?;
            let lock = File::create(self.dir.join(LOCK_FILE))?;
            lock.try_lock().map_err(io::Error::from)?;
            inner.lock = Some(lock);
        }
        if let Some(mut old) = inner.current.take() {
            old.file.flush()?;
        }
        let seg = inner.next_seg;
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(self.segment_path(seg))?;
        inner.next_seg += 1;
        inner.current = Some(Writer {
            seg,
            file: BufWriter::with_capacity(64 * 1024, file),
            len: 0,
        });
        Ok(())
    }

    fn segment_path(&self, seg: u32) -> PathBuf {
        self.dir.join(format!("{seg:06}.seg"))
    }
}

impl Drop for SegmentStore {
    fn drop(&mut self) {
        self.remove_all();
    }
}

/// Removes directories under `live` whose lock nobody holds: their owner
/// exited without cleaning up (a crash, or `kill -9`).
fn sweep_orphans(live: &Path) {
    let Ok(entries) = fs::read_dir(live) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let orphaned = match File::open(dir.join(LOCK_FILE)) {
            Ok(lock) => lock.try_lock().is_ok(),
            // No lock yet: either half-created long ago, or being created
            // right now by another engine. Only the former is ours to remove.
            Err(_) => fs::metadata(&dir)
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().unwrap_or_default() > std::time::Duration::from_secs(60)),
        };
        if orphaned {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("pm-seg-test-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn appends_read_back_across_extents() {
        let root = temp_root();
        let store = SegmentStore::new(&root);
        let a = store.append(b"hello ").unwrap();
        let b = store.append(b"world").unwrap();
        assert_eq!(b.offset, 6);
        let mut out = Vec::new();
        store.read_into(&[a, b], &mut out).unwrap();
        assert_eq!(out, b"hello world");
        assert_eq!(store.read(b).unwrap(), b"world");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn clear_and_drop_remove_files() {
        let root = temp_root();
        let store = SegmentStore::new(&root);
        let a = store.append(b"x").unwrap();
        let dir = store.dir().to_owned();
        store.clear();
        assert!(store.read(a).is_err());
        let b = store.append(b"y").unwrap();
        assert_ne!(a.seg, b.seg);
        drop(store);
        assert!(!dir.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn orphaned_directories_are_swept_but_live_ones_kept() {
        let root = temp_root();
        let live = SegmentStore::new(&root);
        live.append(b"keep").unwrap();
        let orphan = root.join("live").join("orphan");
        fs::create_dir_all(&orphan).unwrap();
        File::create(orphan.join(LOCK_FILE)).unwrap();

        let _other = SegmentStore::new(&root);
        assert!(!orphan.exists());
        assert!(live.dir().exists());
        let _ = fs::remove_dir_all(root);
    }
}
