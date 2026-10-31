//! Rotating append-only segment files under `<root>/live/<uuid>/` that hold
//! large bodies and encoded flow details. Nothing here is durable: the
//! directory is removed on clear, on shutdown and when the store is dropped,
//! and a later run sweeps directories whose owner died.
//!
//! Each segment remembers the newest flow that wrote to it. Flow ids grow
//! with time and only the newest segment is written, so once every flow up
//! to that id has been evicted (bar the few kept, whose segments the caller
//! names), the whole file can go.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use pm_model::FlowId;
use serde::{Deserialize, Serialize};

/// A segment is closed and a new one started past this size. Small enough
/// that eviction frees disk in useful steps.
const SEGMENT_SIZE: u64 = 64 * 1024 * 1024;
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
    /// Every segment on disk, the current one included.
    segs: BTreeMap<u32, SegInfo>,
    /// Bytes in `segs`.
    bytes: u64,
    /// The last write failure, until a write succeeds again.
    error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct SegInfo {
    len: u64,
    /// The newest flow that wrote to the segment.
    max_owner: FlowId,
    /// The oldest (M17: deleting flows frees a segment once no flow left
    /// falls between the two).
    min_owner: FlowId,
}

impl SegInfo {
    const EMPTY: SegInfo = SegInfo {
        len: 0,
        max_owner: 0,
        min_owner: FlowId::MAX,
    };
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

    /// Appends `bytes` on behalf of flow `owner`.
    pub fn append(&self, owner: FlowId, bytes: &[u8]) -> io::Result<Extent> {
        let len = u32::try_from(bytes.len()).map_err(|_| io::Error::other("blob larger than 4 GiB"))?;
        let mut inner = self.inner.lock();
        let result = self.write(&mut inner, owner, bytes, len);
        inner.error = result.as_ref().err().map(describe_write_error);
        result
    }

    fn write(&self, inner: &mut Inner, owner: FlowId, bytes: &[u8], len: u32) -> io::Result<Extent> {
        if inner.current.as_ref().is_none_or(|w| w.len >= SEGMENT_SIZE) {
            self.rotate(inner)?;
        }
        let writer = inner.current.as_mut().expect("rotate opened a segment");
        let extent = Extent {
            seg: writer.seg,
            offset: writer.len,
            len,
        };
        writer.file.write_all(bytes)?;
        writer.len += bytes.len() as u64;
        let info = inner.segs.entry(extent.seg).or_insert(SegInfo::EMPTY);
        info.len += bytes.len() as u64;
        info.max_owner = info.max_owner.max(owner);
        info.min_owner = info.min_owner.min(owner);
        inner.bytes += bytes.len() as u64;
        Ok(extent)
    }

    /// Bytes of segment files on disk.
    pub fn disk_bytes(&self) -> u64 {
        self.inner.lock().bytes
    }

    /// Why the last write failed, if the next hasn't succeeded yet.
    pub fn error(&self) -> Option<String> {
        self.inner.lock().error.clone()
    }

    pub fn dir_path(&self) -> &Path {
        &self.dir
    }

    /// Deletes the finished segments only flows up to `through` wrote to,
    /// except `keep` (segments flows that outlive eviction still use).
    /// Returns the bytes freed.
    pub fn drop_through(&self, through: FlowId, keep: &HashSet<u32>) -> u64 {
        let mut inner = self.inner.lock();
        let current = inner.current.as_ref().map(|w| w.seg);
        let doomed: Vec<u32> = inner
            .segs
            .iter()
            .filter(|(seg, info)| Some(**seg) != current && info.max_owner <= through && !keep.contains(seg))
            .map(|(seg, _)| *seg)
            .collect();
        let mut freed = 0;
        for seg in doomed {
            if let Some(info) = inner.segs.remove(&seg) {
                freed += info.len;
            }
            let _ = fs::remove_file(self.segment_path(seg));
        }
        inner.bytes -= freed;
        freed
    }

    /// The finished segments up to `through` that flows matching `in_use`
    /// (given a segment's oldest and newest writer) may still need.
    pub fn in_use_through(&self, through: FlowId, in_use: impl Fn(FlowId, FlowId) -> bool) -> HashSet<u32> {
        let inner = self.inner.lock();
        inner
            .segs
            .iter()
            .filter(|(_, info)| info.max_owner <= through && in_use(info.min_owner, info.max_owner))
            .map(|(seg, _)| *seg)
            .collect()
    }

    /// The flow id through which eviction must go so that deleting the
    /// oldest segments brings the disk use to `target` bytes or less
    /// (`None` if it already is, or only the current segment is left).
    pub fn owner_to_free(&self, target: u64) -> Option<FlowId> {
        let inner = self.inner.lock();
        let current = inner.current.as_ref().map(|w| w.seg);
        let mut bytes = inner.bytes;
        let mut through = None;
        for (seg, info) in &inner.segs {
            if bytes <= target || Some(*seg) == current {
                break;
            }
            bytes -= info.len;
            through = Some(through.map_or(info.max_owner, |t: FlowId| t.max(info.max_owner)));
        }
        through
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
        for seg in std::mem::take(&mut inner.segs).into_keys() {
            let _ = fs::remove_file(self.segment_path(seg));
        }
        inner.bytes = 0;
        inner.error = None;
    }

    /// Deletes the whole directory.
    pub fn remove_all(&self) {
        let mut inner = self.inner.lock();
        inner.current = None;
        inner.lock = None;
        inner.segs.clear();
        inner.bytes = 0;
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
        inner.segs.insert(seg, SegInfo::EMPTY);
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

/// A write failure as the user should read it: a full disk says so.
fn describe_write_error(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
            "the disk is full: free some space, or lower the disk limit in Settings › Storage".into()
        }
        io::ErrorKind::ReadOnlyFilesystem | io::ErrorKind::PermissionDenied => {
            format!("can't write to the capture directory: {err}")
        }
        _ => format!("can't write captured data: {err}"),
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
        let a = store.append(0, b"hello ").unwrap();
        let b = store.append(1, b"world").unwrap();
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
        let a = store.append(0, b"x").unwrap();
        let dir = store.dir().to_owned();
        store.clear();
        assert!(store.read(a).is_err());
        assert_eq!(store.disk_bytes(), 0);
        let b = store.append(1, b"y").unwrap();
        assert_ne!(a.seg, b.seg);
        drop(store);
        assert!(!dir.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn orphaned_directories_are_swept_but_live_ones_kept() {
        let root = temp_root();
        let live = SegmentStore::new(&root);
        live.append(0, b"keep").unwrap();
        let orphan = root.join("live").join("orphan");
        fs::create_dir_all(&orphan).unwrap();
        File::create(orphan.join(LOCK_FILE)).unwrap();

        let _other = SegmentStore::new(&root);
        assert!(!orphan.exists());
        assert!(live.dir().exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn old_segments_are_dropped_once_their_flows_are_gone() {
        let root = temp_root();
        let store = SegmentStore::new(&root);
        let chunk = vec![0u8; 1024 * 1024];
        // Flows 0..64 fill segment 0; 64..128 segment 1; then segment 2.
        let mut extents = Vec::new();
        for id in 0..130u32 {
            extents.push(store.append(id, &chunk).unwrap());
        }
        assert_eq!(extents[0].seg, 0);
        assert_eq!(extents[64].seg, 1);
        assert_eq!(extents[129].seg, 2);
        let total = store.disk_bytes();
        assert_eq!(total, 130 * 1024 * 1024);

        // Freeing down to 70 MB means evicting through the newest owner of
        // segment 0 (63), then segment 1 (127).
        assert_eq!(store.owner_to_free(100 * 1024 * 1024), Some(63));
        assert_eq!(store.owner_to_free(10 * 1024 * 1024), Some(127));

        // Segment 0 is kept while a pinned flow still uses it.
        assert_eq!(store.drop_through(100, &HashSet::from([0])), 0);
        assert!(store.read(extents[0]).is_ok());
        assert_eq!(store.drop_through(127, &HashSet::new()), 128 * 1024 * 1024);
        assert!(store.read(extents[0]).is_err());
        assert!(store.read(extents[129]).is_ok());
        // The current segment is never dropped.
        assert_eq!(store.drop_through(u32::MAX, &HashSet::new()), 0);
        assert_eq!(store.disk_bytes(), 2 * 1024 * 1024);
        let _ = fs::remove_dir_all(root);
    }
}
