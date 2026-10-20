use std::fs::{self, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rustc_hash::{FxHashMap, FxHashSet};

const MAGIC: &[u8; 8] = b"ORISUSC\x02";

pub type Key = [u8; 32];

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

    /// Whether the file was written so recently that a later write may keep the same mtime.
    fn is_racy(self, now: u64) -> bool {
        self.0[0] + 2 > now
    }
}

/// Candidates per file, keyed by blake3 of the scanner version, the extraction options, the extension and the
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
    dirty: bool,
}

impl Cache {
    /// Starts empty when the file is missing, corrupt or written by another version or with other `options`.
    pub fn open(path: PathBuf, options: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(env!("CARGO_PKG_VERSION").as_bytes()).update(&[0]).update(options.as_bytes());
        let salt = *hasher.finalize().as_bytes();
        let decoded = fs::read(&path).ok().and_then(|bytes| decode(&bytes, &salt));
        let (text, ends, entries, stats) = decoded.unwrap_or_default();
        Self { path, salt, text, ends, ids: FxHashMap::default(), entries, stats, used: FxHashSet::default(), dirty: false }
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
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
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
    /// Only the candidates of kept entries are written.
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
        let tmp = self.path.with_extension("tmp");
        let mut file = io::BufWriter::new(fs::File::create(&tmp)?);
        file.write_all(MAGIC)?;
        file.write_all(&self.salt)?;
        file.write_all(&(order.len() as u32).to_le_bytes())?;
        for &id in &order {
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
        file.into_inner().map_err(io::IntoInnerError::into_error)?.sync_all()?;
        fs::rename(&tmp, &self.path)?;
        self.dirty = false;
        Ok(())
    }
}

type Decoded = (String, Vec<u32>, FxHashMap<Key, Box<[u32]>>, FxHashMap<PathBuf, (Stat, Key)>);

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
        let len = reader.u32()? as usize;
        let path = PathBuf::from(std::str::from_utf8(reader.take(len)?).ok()?);
        let mut stat = [0; 4];
        for field in &mut stat {
            *field = reader.u64()?;
        }
        let key: Key = reader.take(32)?.try_into().ok()?;
        stats.insert(path, (Stat(stat), key));
    }
    reader.bytes.is_empty().then_some((text, ends, entries, stats))
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
}
