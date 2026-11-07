use std::fs::{self, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

const MAGIC: &[u8; 8] = b"ORISUSC\x03";

pub type Key = [u8; 32];

/// A path whose stat, or absence, the walk depended on.
pub type Guard = (PathBuf, Option<Stat>);

/// The entries one walk found, valid while every guard keeps its stat.
#[derive(Debug, Clone)]
struct Walk {
    guards: Vec<Guard>,
    entries: Vec<(PathBuf, bool)>,
}

pub fn now() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
}

/// Modification time, size and inode of a file: equal stats mean the content is taken as unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat([u64; 4]);

impl Stat {
    pub fn of(meta: &Metadata) -> Option<Self> {
        let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        #[cfg(unix)]
        let ino = std::os::unix::fs::MetadataExt::ino(meta);
        #[cfg(not(unix))]
        let ino = 0;
        Some(Self([mtime.as_secs(), u64::from(mtime.subsec_nanos()), meta.len(), ino]))
    }

    pub fn at(path: &Path) -> Option<Self> {
        Self::of(&path.metadata().ok()?)
    }

    /// Whether the file was written after `now`, or so shortly before that a later write may keep the same mtime.
    /// Whole-second mtimes come from coarse file systems (down to 2 s on FAT); finer ones advance at least every clock
    /// tick (up to 15.6 ms on Windows).
    fn is_racy(self, now: Duration) -> bool {
        let window = if self.0[1] == 0 { Duration::from_secs(2) } else { Duration::from_millis(50) };
        Duration::new(self.0[0], self.0[1] as u32) + window > now
    }
}

/// Candidates per file, keyed by blake3 of the scanner sources, the extraction options, the extension and the
/// content, and persisted in one file so later processes skip extraction for unchanged files. Files whose stat
/// did not change since they were hashed are looked up without being read.
#[derive(Debug, Clone)]
pub struct Cache {
    path: PathBuf,
    salt: Key,
    text: String,
    ends: Vec<u32>,
    ids: FxHashMap<Box<str>, u32>,
    entries: FxHashMap<Key, Box<[u32]>>,
    stats: FxHashMap<PathBuf, (Stat, Key)>,
    used: FxHashSet<Key>,
    walk: Option<Walk>,
    dirty: bool,
}

impl Cache {
    /// Starts empty when the file is missing, corrupt or written by another version or with other `options`.
    pub fn open(path: PathBuf, options: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(env!("ORISU_SCANNER_SOURCE_HASH").as_bytes()).update(&[0]).update(options.as_bytes());
        let salt = *hasher.finalize().as_bytes();
        let decoded = fs::read(&path).ok().and_then(|bytes| decode(&bytes, &salt));
        let (text, ends, entries, stats, walk) = decoded.unwrap_or_default();
        Self { path, salt, text, ends, ids: FxHashMap::default(), entries, stats, used: FxHashSet::default(), walk, dirty: false }
    }

    pub fn key(&self, extension: &str, content: &[u8]) -> Key {
        let mut hasher = blake3::Hasher::new_keyed(&self.salt);
        hasher.update(extension.as_bytes()).update(&[0]).update(content);
        *hasher.finalize().as_bytes()
    }

    /// The key `path` had when it was last hashed with the same `stat`, if its candidates are still cached.
    pub fn known(&self, path: &Path, stat: Option<Stat>) -> Option<Key> {
        let (known, key) = self.stats.get(path)?;
        (Some(*known) == stat && self.entries.contains_key(key)).then_some(*key)
    }

    /// Remembers the key of `path` for its `stat`, unless the file is too fresh to trust its mtime.
    pub fn remember(&mut self, path: PathBuf, stat: Option<Stat>, key: Key) {
        let now = now();
        match stat.filter(|stat| !stat.is_racy(now)) {
            Some(stat) => {
                if self.stats.insert(path, (stat, key)) != Some((stat, key)) {
                    self.dirty = true;
                }
            }
            None => {
                if self.stats.remove(&path).is_some() {
                    self.dirty = true;
                }
            }
        }
    }

    /// The entries of the last walk, when none of its guards changed since.
    pub fn walked(&self) -> Option<&[(PathBuf, bool)]> {
        let walk = self.walk.as_ref()?;
        walk.guards.par_iter().all(|(path, stat)| Stat::at(path) == *stat).then_some(&walk.entries)
    }

    /// Remembers a walk that started at `start` with guards taken after it, unless a guard changed since shortly before
    /// the start or a path is not UTF-8.
    pub fn set_walk(&mut self, guards: Vec<Guard>, entries: Vec<(PathBuf, bool)>, start: Duration) {
        let valid = guards.iter().all(|(path, stat)| path.to_str().is_some() && !stat.is_some_and(|stat| stat.is_racy(start)))
            && entries.iter().all(|(path, _)| path.to_str().is_some());
        self.walk = valid.then_some(Walk { guards, entries });
        self.dirty = true;
    }

    pub fn get(&mut self, key: &Key) -> Option<&[u32]> {
        let candidates = self.entries.get(key)?;
        self.used.insert(*key);
        Some(candidates)
    }

    pub fn str(&self, id: u32) -> &str {
        let start = id.checked_sub(1).map_or(0, |prev| self.ends[prev as usize]);
        &self.text[start as usize..self.ends[id as usize] as usize]
    }

    pub fn insert(&mut self, key: Key, candidates: Vec<String>) {
        if self.ids.len() != self.ends.len() {
            self.ids = (0..self.ends.len() as u32).map(|id| (self.str(id).into(), id)).collect();
        }
        let ids = candidates
            .into_iter()
            .map(|candidate| {
                *self.ids.entry(candidate.into_boxed_str()).or_insert_with_key(|candidate| {
                    self.text.push_str(candidate);
                    self.ends.push(self.text.len() as u32);
                    self.ends.len() as u32 - 1
                })
            })
            .collect();
        self.used.insert(key);
        self.entries.insert(key, ids);
        self.dirty = true;
    }

    /// Writes the file when it changed; with `prune`, first drops the entries no lookup asked for since the last save.
    /// Only the candidates of kept entries are written. The cache only saves work, so a failed write costs the next run a
    /// rescan of the files and nothing else; the scanner ignores the error.
    pub fn save(&mut self, prune: bool) -> io::Result<()> {
        if prune && self.entries.len() != self.used.len() {
            let used = &self.used;
            self.entries.retain(|key, _| used.contains(key));
            self.stats.retain(|_, (_, key)| used.contains(key));
            self.dirty = true;
        }
        self.used.clear();
        if !self.dirty {
            return Ok(());
        }
        let mut remap: FxHashMap<u32, u32> = FxHashMap::default();
        let mut order = Vec::new();
        for ids in self.entries.values() {
            for &id in ids {
                remap.entry(id).or_insert_with(|| {
                    order.push(id);
                    order.len() as u32 - 1
                });
            }
        }
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        static WRITES: AtomicUsize = AtomicUsize::new(0);
        let tmp = self.path.with_extension(format!("{}-{}.tmp", std::process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
        let written = self.write(&tmp, &order, &remap).and_then(|()| fs::rename(&tmp, &self.path));
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written?;
        self.dirty = false;
        Ok(())
    }

    fn write(&self, tmp: &Path, order: &[u32], remap: &FxHashMap<u32, u32>) -> io::Result<()> {
        let mut file = io::BufWriter::new(fs::File::create(tmp)?);
        file.write_all(MAGIC)?;
        file.write_all(&self.salt)?;
        file.write_all(&(order.len() as u32).to_le_bytes())?;
        for &id in order {
            let candidate = self.str(id);
            file.write_all(&(candidate.len() as u32).to_le_bytes())?;
            file.write_all(candidate.as_bytes())?;
        }
        file.write_all(&(self.entries.len() as u32).to_le_bytes())?;
        for (key, ids) in &self.entries {
            file.write_all(key)?;
            file.write_all(&(ids.len() as u32).to_le_bytes())?;
            for id in ids {
                file.write_all(&remap[id].to_le_bytes())?;
            }
        }
        let stats: Vec<_> = self.stats.iter().filter_map(|(path, entry)| Some((path.to_str()?, entry))).collect();
        file.write_all(&(stats.len() as u32).to_le_bytes())?;
        for (path, (stat, key)) in stats {
            file.write_all(&(path.len() as u32).to_le_bytes())?;
            file.write_all(path.as_bytes())?;
            for field in stat.0 {
                file.write_all(&field.to_le_bytes())?;
            }
            file.write_all(key)?;
        }
        if let Some(walk) = &self.walk {
            file.write_all(&(walk.guards.len() as u32).to_le_bytes())?;
            for (path, stat) in &walk.guards {
                write_path(&mut file, path)?;
                file.write_all(&[u8::from(stat.is_some())])?;
                for field in stat.map_or([0; 4], |stat| stat.0) {
                    file.write_all(&field.to_le_bytes())?;
                }
            }
            file.write_all(&(walk.entries.len() as u32).to_le_bytes())?;
            for (path, is_dir) in &walk.entries {
                write_path(&mut file, path)?;
                file.write_all(&[u8::from(*is_dir)])?;
            }
        }
        file.flush()
    }
}

fn write_path(file: &mut impl Write, path: &Path) -> io::Result<()> {
    let path = path.to_str().unwrap_or_default();
    file.write_all(&(path.len() as u32).to_le_bytes())?;
    file.write_all(path.as_bytes())
}

type Decoded = (String, Vec<u32>, FxHashMap<Key, Box<[u32]>>, FxHashMap<PathBuf, (Stat, Key)>, Option<Walk>);

fn decode(bytes: &[u8], salt: &Key) -> Option<Decoded> {
    let mut reader = Reader { bytes };
    if reader.take(MAGIC.len())? != MAGIC || reader.take(32)? != salt {
        return None;
    }
    let count = reader.u32()?;
    let mut text = Vec::new();
    let mut ends = Vec::with_capacity(count.min(1 << 20) as usize);
    for _ in 0..count {
        let len = reader.u32()? as usize;
        text.extend_from_slice(reader.take(len)?);
        ends.push(u32::try_from(text.len()).ok()?);
    }
    let text = String::from_utf8(text).ok()?;
    if ends.iter().any(|&end| !text.is_char_boundary(end as usize)) {
        return None;
    }
    let count = reader.u32()?;
    let mut entries = FxHashMap::with_capacity_and_hasher(count.min(1 << 20) as usize, Default::default());
    for _ in 0..count {
        let key: Key = reader.take(32)?.try_into().ok()?;
        let len = reader.u32()?;
        let ids = (0..len).map(|_| reader.u32().filter(|&id| (id as usize) < ends.len())).collect::<Option<Box<[u32]>>>()?;
        entries.insert(key, ids);
    }
    let count = reader.u32()?;
    let mut stats = FxHashMap::with_capacity_and_hasher(count.min(1 << 20) as usize, Default::default());
    for _ in 0..count {
        let path = reader.path()?;
        let stat = reader.stat()?;
        let key: Key = reader.take(32)?.try_into().ok()?;
        stats.insert(path, (stat, key));
    }
    let walk = if reader.bytes.is_empty() {
        None
    } else {
        let count = reader.u32()?;
        let guards = (0..count)
            .map(|_| {
                let path = reader.path()?;
                let present = reader.take(1)?[0] == 1;
                let stat = reader.stat()?;
                Some((path, present.then_some(stat)))
            })
            .collect::<Option<_>>()?;
        let count = reader.u32()?;
        let entries = (0..count).map(|_| Some((reader.path()?, reader.take(1)?[0] == 1))).collect::<Option<_>>()?;
        Some(Walk { guards, entries })
    };
    reader.bytes.is_empty().then_some((text, ends, entries, stats, walk))
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.bytes.split_at_checked(len)?;
        self.bytes = rest;
        Some(head)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn path(&mut self) -> Option<PathBuf> {
        let len = self.u32()? as usize;
        Some(PathBuf::from(std::str::from_utf8(self.take(len)?).ok()?))
    }

    fn stat(&mut self) -> Option<Stat> {
        let mut stat = [0; 4];
        for field in &mut stat {
            *field = self.u64()?;
        }
        Some(Stat(stat))
    }
}
