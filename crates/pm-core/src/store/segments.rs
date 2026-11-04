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
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender};

use parking_lot::{Condvar, Mutex};
use pm_model::FlowId;
use serde::{Deserialize, Serialize};

/// A segment is closed and a new one started past this size. Small enough
/// that eviction frees disk in useful steps.
const SEGMENT_SIZE: u64 = 64 * 1024 * 1024;
const LOCK_FILE: &str = "lock";
/// Writes queued for the writer thread before appends wait for it: with
/// bodies spilling 256 KB at a time, at most about 16 MB in flight.
const QUEUE: usize = 64;
/// Appends smaller than this (encoded details, small payloads) are queued
/// together, in one write of about this size.
const BATCH: usize = 64 * 1024;
/// Read handles kept open at most (a disk cap of a few GB is that many
/// 64 MB segments).
const MAX_READERS: usize = 64;

/// A byte range in one segment file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Extent {
    pub seg: u32,
    pub offset: u64,
    pub len: u32,
}

/// Appends reserve their extent here, under a short lock, and hand the
/// bytes to the `pm-segments` thread, which does the file I/O (R2): a
/// recorder never waits on the disk unless the queue is full. Reads wait
/// until the writes before them are on disk.
pub(crate) struct SegmentStore {
    dir: PathBuf,
    inner: Mutex<Inner>,
    /// Open files to read segments with, shared by every reader (R2: a
    /// deep search reads 100k details; opening the file for each was most
    /// of its time). Dropped with the segment.
    readers: Mutex<std::collections::HashMap<u32, Arc<File>>>,
    writer: Arc<WriterState>,
    /// Sends are made under `inner`, so jobs arrive in reservation order.
    jobs: Option<SyncSender<Job>>,
}

#[derive(Default)]
struct Inner {
    /// Held exclusively while the directory exists, so a sweep in another
    /// process can tell a live directory from an orphan.
    lock: Option<File>,
    /// The segment being appended to and its reserved length.
    current: Option<(u32, u64)>,
    /// Numbers keep increasing across clears, so a stale extent from a
    /// cleared flow can never alias new data.
    next_seg: u32,
    /// Every segment on disk, the current one included.
    segs: BTreeMap<u32, SegInfo>,
    /// Bytes in `segs`.
    bytes: u64,
    /// The last job numbered; the writer reports how far it has got.
    seq: u64,
    /// Small appends to the current segment not queued yet, and the job
    /// number they will go out as.
    batch: Vec<u8>,
    batch_seq: u64,
    /// Opening a segment failed, until an append succeeds again.
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
    /// The last job that writes to it.
    last_seq: u64,
}

impl SegInfo {
    const EMPTY: SegInfo = SegInfo {
        len: 0,
        max_owner: 0,
        min_owner: FlowId::MAX,
        last_seq: 0,
    };
}

enum Job {
    /// Later writes go to this file, in order.
    Open {
        seg: u32,
        file: File,
        seq: u64,
    },
    Write {
        seg: u32,
        bytes: Vec<u8>,
        seq: u64,
    },
    /// Close the current file (the segments are about to be deleted).
    Close {
        seq: u64,
    },
}

/// What the writer thread reports back.
#[derive(Default)]
struct WriterState {
    progress: Mutex<Progress>,
    advanced: Condvar,
    /// `progress.broken` is set: lets appends skip its lock.
    has_broken: std::sync::atomic::AtomicBool,
}

#[derive(Default)]
struct Progress {
    /// Every job up to this one is done: its bytes are flushed to the file
    /// (or lost to a failed write).
    done: u64,
    /// The writer is gone; nobody waits for it any more.
    stopped: bool,
    /// Why the last write failed, until one succeeds again.
    error: Option<String>,
    /// A write to this segment failed: the rest of it can't be trusted,
    /// so the next append starts a new one.
    broken: Option<u32>,
}

impl WriterState {
    /// Blocks until every job up to `seq` is done.
    fn wait(&self, seq: u64) {
        let mut progress = self.progress.lock();
        while progress.done < seq && !progress.stopped {
            self.advanced.wait(&mut progress);
        }
    }
}

impl SegmentStore {
    /// A store under `<root>/live/<uuid>/`. The directory is created on the
    /// first append; orphaned siblings are removed now.
    pub fn new(root: &Path) -> Self {
        let live = root.join("live");
        sweep_orphans(&live);
        let writer = Arc::new(WriterState::default());
        let (tx, rx) = std::sync::mpsc::sync_channel(QUEUE);
        let state = writer.clone();
        let spawned = std::thread::Builder::new()
            .name("pm-segments".into())
            .spawn(move || write_loop(rx, &state));
        let jobs = match spawned {
            Ok(_) => Some(tx),
            Err(err) => {
                eprintln!("proxymate: can't start the segment writer: {err}");
                None
            }
        };
        SegmentStore {
            dir: live.join(uuid::Uuid::new_v4().to_string()),
            inner: Mutex::new(Inner::default()),
            readers: Mutex::new(Default::default()),
            writer,
            jobs,
        }
    }

    /// Appends `bytes` on behalf of flow `owner`.
    pub fn append(&self, owner: FlowId, bytes: &[u8]) -> io::Result<Extent> {
        self.append_owned(owner, bytes.to_vec())
    }

    /// Like [`SegmentStore::append`], without copying the bytes. They reach
    /// the file on the writer thread; a failure there shows in
    /// [`SegmentStore::error`], and reads of what it lost fail.
    pub fn append_owned(&self, owner: FlowId, bytes: Vec<u8>) -> io::Result<Extent> {
        let len = u32::try_from(bytes.len()).map_err(|_| io::Error::other("blob larger than 4 GiB"))?;
        let mut inner = self.inner.lock();
        let result = self.reserve(&mut inner, owner, bytes, len);
        inner.error = result.as_ref().err().map(describe_write_error);
        result
    }

    fn reserve(&self, inner: &mut Inner, owner: FlowId, bytes: Vec<u8>, len: u32) -> io::Result<Extent> {
        let jobs = self
            .jobs
            .as_ref()
            .ok_or_else(|| io::Error::other("the segment writer isn't running"))?;
        if self.writer.has_broken.swap(false, std::sync::atomic::Ordering::AcqRel) {
            let broken = self.writer.progress.lock().broken.take();
            if broken.is_some() && inner.current.map(|(seg, _)| seg) == broken {
                inner.batch.clear();
                inner.current = None;
            }
        }
        if inner.current.is_none_or(|(_, len)| len >= SEGMENT_SIZE) {
            self.rotate(inner, jobs)?;
        }
        let (seg, offset) = inner.current.expect("rotate opened a segment");
        let extent = Extent { seg, offset, len };
        let seq = if bytes.len() < BATCH {
            if inner.batch.is_empty() {
                inner.seq += 1;
                inner.batch_seq = inner.seq;
            }
            inner.batch.extend_from_slice(&bytes);
            let seq = inner.batch_seq;
            if inner.batch.len() >= BATCH {
                send_batch(inner, jobs)?;
            }
            seq
        } else {
            send_batch(inner, jobs)?;
            inner.seq += 1;
            let seq = inner.seq;
            jobs.send(Job::Write { seg, bytes, seq })
                .map_err(|_| io::Error::other("the segment writer stopped"))?;
            seq
        };
        inner.current = Some((seg, offset + u64::from(len)));
        let info = inner.segs.entry(seg).or_insert(SegInfo::EMPTY);
        info.len += u64::from(len);
        info.max_owner = info.max_owner.max(owner);
        info.min_owner = info.min_owner.min(owner);
        info.last_seq = seq;
        inner.bytes += u64::from(len);
        Ok(extent)
    }

    /// Blocks until every append so far has reached its file.
    pub fn sync(&self) {
        let seq = {
            let mut inner = self.inner.lock();
            if let Some(jobs) = &self.jobs {
                let _ = send_batch(&mut inner, jobs);
            }
            inner.seq
        };
        self.writer.wait(seq);
    }

    /// Bytes of segment files on disk.
    pub fn disk_bytes(&self) -> u64 {
        self.inner.lock().bytes
    }

    /// Why the last write failed, if the next hasn't succeeded yet.
    pub fn error(&self) -> Option<String> {
        let opening = self.inner.lock().error.clone();
        opening.or_else(|| self.writer.progress.lock().error.clone())
    }

    pub fn dir_path(&self) -> &Path {
        &self.dir
    }

    /// Deletes the finished segments only flows up to `through` wrote to,
    /// except `keep` (segments flows that outlive eviction still use).
    /// Returns the bytes freed.
    pub fn drop_through(&self, through: FlowId, keep: &HashSet<u32>) -> u64 {
        let mut inner = self.inner.lock();
        let current = inner.current.map(|(seg, _)| seg);
        let doomed: Vec<u32> = inner
            .segs
            .iter()
            .filter(|(seg, info)| Some(**seg) != current && info.max_owner <= through && !keep.contains(seg))
            .map(|(seg, _)| *seg)
            .collect();
        // Their writes went out before the current segment opened; they
        // are almost always on disk already.
        let last = doomed
            .iter()
            .filter_map(|seg| inner.segs.get(seg))
            .map(|i| i.last_seq)
            .max();
        if let Some(last) = last {
            self.writer.wait(last);
        }
        let mut freed = 0;
        self.forget_readers(doomed.iter().copied());
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
        let current = inner.current.map(|(seg, _)| seg);
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
        // Writes queued before this read must reach the file first.
        let pending = {
            let mut inner = self.inner.lock();
            let pending = extents
                .iter()
                .filter_map(|e| inner.segs.get(&e.seg))
                .map(|info| info.last_seq)
                .max();
            if pending >= Some(inner.batch_seq)
                && let Some(jobs) = &self.jobs
            {
                send_batch(&mut inner, jobs)?;
            }
            pending
        };
        if let Some(seq) = pending {
            self.writer.wait(seq);
        }
        let mut open: Option<(u32, Arc<File>)> = None;
        for extent in extents {
            let file = match &open {
                Some((seg, file)) if *seg == extent.seg => file,
                _ => &open.insert((extent.seg, self.reader(extent.seg)?)).1,
            };
            let start = out.len();
            out.resize(start + extent.len as usize, 0);
            pm_platform::fs::read_exact_at(file, &mut out[start..], extent.offset)?;
        }
        Ok(())
    }

    /// A shared read handle on segment `seg`, opened on first use.
    fn reader(&self, seg: u32) -> io::Result<Arc<File>> {
        if let Some(file) = self.readers.lock().get(&seg) {
            return Ok(file.clone());
        }
        let file = Arc::new(File::open(self.segment_path(seg))?);
        // Opened outside the lock; a segment deleted meanwhile mustn't be
        // cached (its number is never reused, but the handle would leak).
        if self.inner.lock().segs.contains_key(&seg) {
            let mut readers = self.readers.lock();
            if readers.len() >= MAX_READERS {
                readers.clear();
            }
            readers.insert(seg, file.clone());
        }
        Ok(file)
    }

    fn forget_readers(&self, segs: impl IntoIterator<Item = u32>) {
        let mut readers = self.readers.lock();
        for seg in segs {
            readers.remove(&seg);
        }
    }

    /// Like [`SegmentStore::read_into`], stopping once `out` has grown by
    /// `max` bytes: only the extents (and the part of the last one) that
    /// hold the first `max` bytes are read.
    pub fn read_prefix_into(&self, extents: &[Extent], max: usize, out: &mut Vec<u8>) -> io::Result<()> {
        let mut want = max;
        let mut prefix = Vec::new();
        for extent in extents {
            if want == 0 {
                break;
            }
            let len = (extent.len as usize).min(want);
            prefix.push(Extent {
                len: len as u32,
                ..*extent
            });
            want -= len;
        }
        self.read_into(&prefix, out)
    }

    pub fn read(&self, extent: Extent) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(extent.len as usize);
        self.read_into(&[extent], &mut out)?;
        Ok(out)
    }

    /// Deletes every segment. Later appends start a new one.
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        self.close_current(&mut inner);
        self.readers.lock().clear();
        for seg in std::mem::take(&mut inner.segs).into_keys() {
            let _ = fs::remove_file(self.segment_path(seg));
        }
        inner.bytes = 0;
        inner.error = None;
        self.writer.progress.lock().error = None;
    }

    /// Deletes the whole directory.
    pub fn remove_all(&self) {
        let mut inner = self.inner.lock();
        self.close_current(&mut inner);
        self.readers.lock().clear();
        inner.lock = None;
        inner.segs.clear();
        inner.bytes = 0;
        let _ = fs::remove_dir_all(&self.dir);
    }

    /// Waits for the writer to finish what is queued and close its file,
    /// so the segments can be deleted (an open file can't be on Windows).
    fn close_current(&self, inner: &mut Inner) {
        let Some(jobs) = &self.jobs else {
            inner.current = None;
            return;
        };
        let _ = send_batch(inner, jobs);
        inner.current = None;
        inner.seq += 1;
        if jobs.send(Job::Close { seq: inner.seq }).is_ok() {
            self.writer.wait(inner.seq);
        }
    }

    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn rotate(&self, inner: &mut Inner, jobs: &SyncSender<Job>) -> io::Result<()> {
        if inner.lock.is_none() {
            fs::create_dir_all(&self.dir)?;
            let lock = File::create(self.dir.join(LOCK_FILE))?;
            lock.try_lock().map_err(io::Error::from)?;
            inner.lock = Some(lock);
        }
        // The batch belongs to the segment being closed.
        send_batch(inner, jobs)?;
        let seg = inner.next_seg;
        // Opened here so a failure is the append's error.
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(self.segment_path(seg))?;
        inner.next_seg += 1;
        inner.seq += 1;
        jobs.send(Job::Open {
            seg,
            file,
            seq: inner.seq,
        })
        .map_err(|_| io::Error::other("the segment writer stopped"))?;
        inner.segs.insert(seg, SegInfo::EMPTY);
        inner.current = Some((seg, 0));
        Ok(())
    }

    fn segment_path(&self, seg: u32) -> PathBuf {
        self.dir.join(format!("{seg:06}.seg"))
    }
}

impl Drop for SegmentStore {
    fn drop(&mut self) {
        self.remove_all();
        // Closing the channel stops the writer.
        self.jobs = None;
    }
}

/// Queues the small appends gathered so far, as one write.
fn send_batch(inner: &mut Inner, jobs: &SyncSender<Job>) -> io::Result<()> {
    let Some((seg, _)) = inner.current else {
        inner.batch.clear();
        return Ok(());
    };
    if inner.batch.is_empty() {
        return Ok(());
    }
    let bytes = std::mem::replace(&mut inner.batch, Vec::with_capacity(BATCH));
    jobs.send(Job::Write {
        seg,
        bytes,
        seq: inner.batch_seq,
    })
    .map_err(|_| io::Error::other("the segment writer stopped"))
}

/// The `pm-segments` thread: writes queued bytes in order, flushing
/// whenever the queue runs dry, until the store goes.
fn write_loop(jobs: Receiver<Job>, state: &WriterState) {
    let mut current: Option<(u32, BufWriter<File>)> = None;
    let mut failure: Option<(u32, String)> = None;
    while let Ok(first) = jobs.recv() {
        let mut job = Some(first);
        let mut done = 0;
        let mut wrote = false;
        while let Some(next) = job.take() {
            match next {
                Job::Open { seg, file, seq } => {
                    if let Some((old, mut file)) = current.take()
                        && let Err(err) = file.flush()
                    {
                        failure = Some((old, describe_write_error(&err)));
                    }
                    current = Some((seg, BufWriter::with_capacity(64 * 1024, file)));
                    done = seq;
                }
                Job::Write { seg, bytes, seq } => {
                    // A segment whose write failed takes nothing more: its
                    // offsets no longer match the reservations.
                    if let Some((open, file)) = &mut current
                        && *open == seg
                    {
                        match file.write_all(&bytes) {
                            Ok(()) => wrote = true,
                            Err(err) => {
                                failure = Some((seg, describe_write_error(&err)));
                                current = None;
                            }
                        }
                    }
                    done = seq;
                }
                Job::Close { seq } => {
                    if let Some((seg, mut file)) = current.take()
                        && let Err(err) = file.flush()
                    {
                        failure = Some((seg, describe_write_error(&err)));
                    }
                    done = seq;
                }
            }
            job = jobs.try_recv().ok();
        }
        if let Some((seg, file)) = &mut current
            && let Err(err) = file.flush()
        {
            failure = Some((*seg, describe_write_error(&err)));
            current = None;
        }
        let mut progress = state.progress.lock();
        progress.done = progress.done.max(done);
        match failure.take() {
            Some((seg, error)) => {
                progress.error = Some(error);
                progress.broken = Some(seg);
                state.has_broken.store(true, std::sync::atomic::Ordering::Release);
            }
            None if wrote => progress.error = None,
            None => {}
        }
        drop(progress);
        state.advanced.notify_all();
    }
    state.progress.lock().stopped = true;
    state.advanced.notify_all();
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
    fn prefix_reads_stop_inside_the_extent_that_holds_the_end() {
        let root = temp_root();
        let store = SegmentStore::new(&root);
        let extents: Vec<Extent> = [&b"hello "[..], b"big ", b"world"]
            .iter()
            .map(|chunk| store.append(0, chunk).unwrap())
            .collect();
        let mut out = Vec::new();
        store.read_prefix_into(&extents, 8, &mut out).unwrap();
        assert_eq!(out, b"hello bi");
        out.clear();
        store.read_prefix_into(&extents, 100, &mut out).unwrap();
        assert_eq!(out, b"hello big world");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn queued_writes_land_before_reads_and_clears() {
        let root = temp_root();
        let store = SegmentStore::new(&root);
        let chunk = vec![9u8; 256 * 1024];
        // More than the queue holds: appends wait for the writer, never fail.
        let extents: Vec<Extent> = (0..(QUEUE as u32 * 2))
            .map(|id| store.append(id, &chunk).unwrap())
            .collect();
        assert_eq!(store.read(*extents.last().unwrap()).unwrap(), chunk);
        store.sync();
        assert_eq!(store.error(), None);
        store.clear();
        let left: Vec<_> = fs::read_dir(store.dir())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name() != LOCK_FILE)
            .collect();
        assert!(left.is_empty(), "{left:?}");
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
