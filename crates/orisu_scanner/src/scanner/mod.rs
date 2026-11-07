pub mod auto_source_detection;
mod detect_sources;
pub mod sources;

use crate::GlobEntry;
use crate::attributify;
use crate::cache::{self, Cache, Guard, Stat};
use crate::extractor::pre_processors::pre_process;
use crate::extractor::{Extracted, Extractor};
use crate::glob::optimize_patterns;
use crate::manifest;
use crate::scanner::detect_sources::resolve_globs;
use crate::scanner::sources::{PublicSourceEntry, SourceEntry, inside_git_repo, public_source_entries_to_private_source_entries};
use auto_source_detection::BINARY_EXTENSIONS_GLOB;
use bstr::ByteSlice;
use fast_glob::glob_match;
use orisu_candidate::attributify::Attributify;
use orisu_ignore::{WalkBuilder, git_exclude_files, gitignore::GitignoreBuilder, is_normal_path, path_starts_with};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A file with its extension, or content with the extension of the file it stands for.
#[derive(Debug, Clone)]
pub enum ChangedContent {
    File(PathBuf, String),
    Content(String, String),
}

#[derive(Debug, Clone, Default)]
pub struct Scanner {
    sources: Vec<SourceEntry>,
    walker: Option<WalkBuilder>,
    /// Manifests of installed libraries whose sources are not scanned.
    manifests: Vec<PathBuf>,
    attributify: Option<Attributify>,
    cache: Option<Cache>,
    /// Whether `files`, `dirs` and `extensions` hold the result of a walk.
    walked: bool,
    files: FxHashSet<PathBuf>,
    dirs: FxHashSet<PathBuf>,
    extensions: FxHashSet<String>,
    /// The globs for watchers, computed once per walk.
    globs: Option<Vec<GlobEntry>>,
    candidates: FxHashSet<String>,
    /// The stats files had when last scanned, so that later scans skip unchanged files.
    stats: FxHashMap<PathBuf, Stat>,
    /// The files the last scan read.
    scanned_files: Vec<String>,
    /// Whether a scan ran; the first one prunes the cache.
    scanned: bool,
}

impl Scanner {
    pub fn new(sources: Vec<PublicSourceEntry>) -> Self {
        let mut manifests = Vec::new();
        let sources = sources
            .into_iter()
            .filter(|source| match manifest::for_source(source) {
                Some(manifest) => {
                    if !manifests.contains(&manifest) {
                        manifests.push(manifest);
                    }
                    false
                }
                None => true,
            })
            .collect();
        let sources = public_source_entries_to_private_source_entries(sources);
        let walker = create_walker(&sources, None);
        Self { sources, walker, manifests, ..Default::default() }
    }

    /// Also extracts markup attributes as attributify candidates.
    pub fn with_attributify(mut self, attributify: Attributify) -> Self {
        self.attributify = Some(attributify);
        self
    }

    /// Keeps the candidates of every scanned file in `dir`, keyed by content, so unchanged files are not extracted again
    /// by this or any later scanner with the same sources, and the files the walk found.
    pub fn with_cache(mut self, dir: impl Into<PathBuf>) -> Self {
        let options = format!("{:?}\n{:?}", self.attributify, self.sources);
        let name = format!("scanner-{}.bin", &blake3::hash(options.as_bytes()).to_hex()[..16]);
        self.cache = Some(Cache::open(dir.into().join(name), &options));
        self
    }

    pub fn scan(&mut self) -> Vec<String> {
        self.scan_new();
        let mut result = self.candidates.iter().cloned().collect::<Vec<_>>();
        result.par_sort_unstable();
        result
    }

    /// Scans like [`Scanner::scan`], returning only the candidates that no earlier scan found.
    pub fn scan_new(&mut self) -> Vec<String> {
        let mut found = FxHashSet::default();
        let mut content_files = vec![];
        let mut css_files = vec![];
        let mut changed_files = vec![];
        for entry in self.walk() {
            let changed = match entry.stat {
                Some(stat) => self.stats.insert(entry.path.clone(), stat) != Some(stat),
                None => true,
            };
            if !changed {
                continue;
            }
            if let Some(file) = entry.path.to_str() {
                changed_files.push(file.to_owned());
            }
            match entry.kind {
                EntryKind::Manifest => found.extend(manifest::read(&entry.path)),
                _ => match extension(&entry.path) {
                    "css" => css_files.push(entry.path),
                    extension => {
                        let extension = extension.to_owned();
                        content_files.push((entry.path, extension, entry.stat));
                    }
                },
            }
        }
        self.stats.retain(|path, _| self.files.contains(path));

        let attributify = self.attributify.as_ref();
        found.extend(match &mut self.cache {
            Some(cache) => extract_cached(cache, content_files, attributify),
            None => extract_all(
                content_files.into_iter().map(|(path, extension, _)| ChangedContent::File(path, extension)).collect(),
                attributify,
            ),
        });
        found.extend(extract_css_variables(css_files));

        let fresh = self.add_candidates(found);
        changed_files.par_sort_unstable();
        self.scanned_files = changed_files;
        if let Some(cache) = &mut self.cache {
            let _ = cache.save(!self.scanned);
        }
        self.scanned = true;
        fresh
    }

    pub fn scan_content(&mut self, changed_content: Vec<ChangedContent>) -> Vec<String> {
        let mut content_to_scan = vec![];
        let mut unknown_files = vec![];
        for changed in changed_content {
            match changed {
                ChangedContent::File(file, extension) => {
                    let Ok(file) = dunce::canonicalize(file) else {
                        continue;
                    };
                    if self.files.contains(&file) {
                        content_to_scan.push(ChangedContent::File(file, extension));
                    } else {
                        unknown_files.push((file, extension));
                    }
                }
                content => content_to_scan.push(content),
            }
        }

        if !unknown_files.is_empty()
            && let Some(walker) = create_walker(&self.sources, Some(unknown_files.iter().map(|(file, _)| file.clone()).collect()))
        {
            for entry in walker.build().filter_map(Result::ok) {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                unknown_files.retain(|(file, extension)| {
                    if file != path {
                        return true;
                    }
                    self.files.insert(file.clone());
                    content_to_scan.push(ChangedContent::File(file.clone(), extension.clone()));
                    false
                });
                if unknown_files.is_empty() {
                    break;
                }
            }
        }

        let found = extract_all(content_to_scan, self.attributify.as_ref());
        self.add_candidates(found)
    }

    /// Keeps the candidates in `found` and returns the ones not seen before, sorted.
    fn add_candidates(&mut self, mut found: FxHashSet<String>) -> Vec<String> {
        found.retain(|candidate| !self.candidates.contains(candidate));
        self.candidates.extend(found.iter().cloned());
        let mut result: Vec<String> = found.into_iter().collect();
        result.par_sort_unstable();
        result
    }

    pub fn get_files(&mut self) -> Vec<String> {
        if !self.walked {
            self.walk();
        }
        self.files.par_iter().filter_map(|x| x.clone().into_os_string().into_string().ok()).collect()
    }

    pub fn get_scanned_files(&self) -> Vec<String> {
        self.scanned_files.clone()
    }

    pub fn get_globs(&mut self) -> Vec<GlobEntry> {
        if !self.walked {
            self.walk();
        }
        if let Some(globs) = &self.globs {
            return globs.clone();
        }

        let mut globs = vec![];
        for source in &self.sources {
            match source {
                SourceEntry::Auto { base } | SourceEntry::External { base } => {
                    globs.extend(resolve_globs(base.to_path_buf(), &self.dirs, &self.extensions));
                }
                SourceEntry::Pattern { .. } => globs.push(source.into()),
                SourceEntry::Ignored { .. } => {}
            }
        }
        let globs = optimize_patterns(&globs);
        self.globs = Some(globs.clone());
        globs
    }

    pub fn get_normalized_sources(&self) -> Vec<GlobEntry> {
        self.sources.iter().filter(|source| !matches!(source, SourceEntry::Ignored { .. })).map(GlobEntry::from).collect()
    }

    /// The candidates of one file with their byte offsets in it, before variant group expansion.
    pub fn get_candidates_with_positions(&self, changed_content: ChangedContent) -> Vec<(String, usize)> {
        /// Stands in for `-[]`, which ends no candidate, so that editors get the position of a candidate like `bg-[]`
        /// while its value is being typed. It has the same length, so the offsets stay valid.
        const EMPTY_ARBITRARY_PLACEHOLDER: &str = "XYZ";

        let original =
            read_changed_content(changed_content).map(|(content, extension)| pre_process(content, &extension)).unwrap_or_default();
        let content = original.replace("-[]", EMPTY_ARBITRARY_PLACEHOLDER);
        let offset = content.as_ptr() as usize;

        Extractor::new(&content[..])
            .extract()
            .into_par_iter()
            .flat_map(|extracted| match extracted {
                Extracted::Candidate(s) => {
                    let i = s.as_ptr() as usize - offset;
                    let written = &original[i..i + s.len()];
                    let bytes = if written.contains_str("-[]") { written } else { s };
                    Some((std::str::from_utf8(bytes).ok()?.to_owned(), i))
                }
                Extracted::CssVariable(_) => None,
            })
            .collect()
    }

    /// Walks the sources into `files`, `dirs` and `extensions`, returning the file and manifest entries.
    fn walk(&mut self) -> Vec<WalkEntry> {
        let mut entries = match (&mut self.cache, &mut self.walker) {
            (Some(cache), Some(walker)) => walk_cached(cache, walker, &self.sources),
            (None, Some(walker)) => walk_parallel(walker),
            (_, None) => vec![],
        };
        entries.extend(self.manifests.iter().map(|path| WalkEntry { path: path.clone(), kind: EntryKind::Manifest, stat: Stat::at(path) }));

        self.walked = true;
        self.files.clear();
        self.dirs.clear();
        self.extensions.clear();
        self.globs = None;

        let mut files = vec![];
        for entry in entries {
            match entry.kind {
                EntryKind::Dir => {
                    self.dirs.insert(entry.path);
                }
                _ if !self.files.insert(entry.path.clone()) => {}
                kind => {
                    if kind == EntryKind::File {
                        self.extensions.insert(extension(&entry.path).to_owned());
                    }
                    files.push(entry);
                }
            }
        }
        files
    }
}

/// The candidates the scanner finds in one file, without attributify.
pub fn file_candidates(content: Vec<u8>, extension: &str) -> FxHashSet<String> {
    let mut out = FxHashSet::default();
    extract_file(content, extension, None, &mut out, &mut FxHashSet::default());
    out
}

/// Adds the candidates of a file to `out`, skipping lines in `seen`, the lines already extracted.
fn extract_file(
    content: Vec<u8>,
    extension: &str,
    attributify: Option<&Attributify>,
    out: &mut FxHashSet<String>,
    seen: &mut FxHashSet<Box<[u8]>>,
) {
    let content = pre_process(content, extension);
    let content = orisu_candidate::group::expand(&content).unwrap_or(content);
    for line in content.split(|x| *x == b'\n').filter(|line| !line.is_empty()) {
        if seen.contains(line) {
            continue;
        }
        seen.insert(line.into());
        for extracted in Extractor::new(line).extract() {
            let (Extracted::Candidate(bytes) | Extracted::CssVariable(bytes)) = extracted;
            let Ok(candidate) = std::str::from_utf8(bytes) else {
                continue;
            };
            if !out.contains(candidate) {
                out.insert(candidate.to_owned());
            }
        }
    }
    if let Some(attributify) = attributify {
        attributify::extract(attributify, &content, out);
    }
}

/// Candidates of `contents`, each read, extracted and dropped in one pass so that only the files in flight are in memory.
fn extract_all(contents: Vec<ChangedContent>, attributify: Option<&Attributify>) -> FxHashSet<String> {
    contents
        .into_par_iter()
        .fold(Default::default, |(mut out, mut seen): (FxHashSet<String>, FxHashSet<Box<[u8]>>), changed| {
            if let Some((content, extension)) = read_changed_content(changed) {
                extract_file(content, &extension, attributify, &mut out, &mut seen);
            }
            (out, seen)
        })
        .map(|(out, _)| out)
        .reduce(FxHashSet::default, union)
}

fn union(mut a: FxHashSet<String>, mut b: FxHashSet<String>) -> FxHashSet<String> {
    if a.len() < b.len() {
        std::mem::swap(&mut a, &mut b);
    }
    a.extend(b);
    a
}

/// Candidates of `files`: cached ones by stat or content, the rest extracted one file at a time and added to `cache`.
fn extract_cached(cache: &mut Cache, files: Vec<(PathBuf, String, Option<Stat>)>, attributify: Option<&Attributify>) -> FxHashSet<String> {
    let looked_up: Vec<_> = files
        .into_par_iter()
        .filter_map(|(path, extension, stat)| match cache.known(&path, stat) {
            Some(key) => Some((key, None)),
            None => {
                let content = std::fs::read(&path).ok()?;
                Some((cache.key(&extension, &content), Some((content, extension, path, stat))))
            }
        })
        .collect();
    let mut ids = FxHashSet::default();
    let mut misses = vec![];
    for (key, read) in looked_up {
        let hit = cache.get(&key).map(|candidates| ids.extend(candidates.iter().copied())).is_some();
        match read {
            Some((_, _, path, stat)) if hit => cache.remember(path, stat, key),
            Some((content, extension, path, stat)) => misses.push((key, content, extension, path, stat)),
            None => {}
        }
    }
    let mut out: FxHashSet<String> = ids.into_iter().map(|id| cache.str(id).to_owned()).collect();
    let fresh: Vec<_> = misses
        .into_par_iter()
        .map(|(key, content, extension, path, stat)| {
            let mut candidates = FxHashSet::default();
            extract_file(content, &extension, attributify, &mut candidates, &mut FxHashSet::default());
            let mut candidates: Vec<String> = candidates.into_iter().collect();
            candidates.sort_unstable();
            (key, candidates, path, stat)
        })
        .collect();
    for (key, candidates, path, stat) in fresh {
        out.extend(candidates.iter().cloned());
        cache.insert(key, candidates);
        cache.remember(path, stat, key);
    }
    out
}

fn read_changed_content(changed: ChangedContent) -> Option<(Vec<u8>, String)> {
    match changed {
        ChangedContent::File(file, extension) => Some((std::fs::read(&file).ok()?, extension)),
        ChangedContent::Content(contents, extension) => Some((contents.into_bytes(), extension)),
    }
}

/// The CSS variables that `files` read with `var(…)`.
fn extract_css_variables(files: Vec<PathBuf>) -> FxHashSet<String> {
    files
        .into_par_iter()
        .filter_map(|file| std::fs::read(file).ok())
        .map(|content| orisu_candidate::group::expand(&content).unwrap_or(content))
        .fold(FxHashSet::default, |mut out, content| {
            for line in content.split(|x| *x == b'\n').filter(|line| !line.is_empty()) {
                for extracted in Extractor::new(line).extract_variables_from_css() {
                    let (Extracted::Candidate(bytes) | Extracted::CssVariable(bytes)) = extracted;
                    if let Ok(variable) = std::str::from_utf8(bytes) {
                        out.insert(variable.to_owned());
                    }
                }
            }
            out
        })
        .reduce(FxHashSet::default, union)
}

fn extension(path: &Path) -> &str {
    path.extension().and_then(|x| x.to_str()).unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Dir,
    File,
    /// A manifest standing in for the sources of an installed library.
    Manifest,
}

#[derive(Debug)]
struct WalkEntry {
    path: PathBuf,
    kind: EntryKind,
    /// `None` for directories and files whose metadata can't be read.
    stat: Option<Stat>,
}

impl WalkEntry {
    fn new(path: PathBuf, is_dir: bool) -> Self {
        match is_dir {
            true => Self { path, kind: EntryKind::Dir, stat: None },
            false => {
                let stat = Stat::at(&path);
                Self { path, kind: EntryKind::File, stat }
            }
        }
    }
}

/// The stats a directory's listing and ignore rules depend on: its own, those of its ignore files and those of the git
/// exclude files. A missing `.gitignore` or `.ignore` needs no guard, since creating one changes the directory.
fn dir_guards(dir: &Path) -> Vec<Guard> {
    let mut guards = vec![(dir.to_path_buf(), Stat::at(dir))];
    for name in [".gitignore", ".ignore"] {
        let path = dir.join(name);
        if let Some(stat) = Stat::at(&path) {
            guards.push((path, Some(stat)));
        }
    }
    guards.extend(git_exclude_files(dir).into_iter().map(|path| {
        let stat = Stat::at(&path);
        (path, stat)
    }));
    guards
}

/// Reuses the entries of the cached walk while no directory it entered and no ignore file it read changed; otherwise
/// walks and caches the result. Guards are taken after the walk, so one that changed during it is too fresh to keep.
fn walk_cached(cache: &mut Cache, walker: &mut WalkBuilder, sources: &[SourceEntry]) -> Vec<WalkEntry> {
    if let Some(entries) = cache.walked() {
        return entries.par_iter().map(|(path, is_dir)| WalkEntry::new(path.clone(), *is_dir)).collect();
    }
    let start = cache::now();
    let entries = walk_parallel(walker);
    let mut dirs: FxHashSet<&Path> =
        entries.iter().filter(|entry| entry.kind == EntryKind::Dir).map(|entry| entry.path.as_path()).collect();
    for source in sources {
        if let SourceEntry::Auto { base } | SourceEntry::Pattern { base, .. } | SourceEntry::External { base } = source {
            dirs.extend(base.ancestors());
        }
    }
    let guards = dirs.into_par_iter().flat_map_iter(dir_guards).collect();
    cache.set_walk(guards, entries.iter().map(|entry| (entry.path.clone(), entry.kind == EntryKind::Dir)).collect(), start);
    entries
}

fn walk_parallel(walker: &mut WalkBuilder) -> Vec<WalkEntry> {
    struct FlushOnDrop {
        local: Vec<WalkEntry>,
        shared: Arc<Mutex<Vec<WalkEntry>>>,
    }

    impl Drop for FlushOnDrop {
        fn drop(&mut self) {
            if !self.local.is_empty() {
                self.shared.lock().unwrap().append(&mut self.local);
            }
        }
    }

    let collected: Arc<Mutex<Vec<WalkEntry>>> = Arc::new(Mutex::new(vec![]));

    walker.build_parallel().run(|| {
        let mut buf = FlushOnDrop { local: Vec::with_capacity(256), shared: collected.clone() };

        Box::new(move |entry| {
            let Ok(entry) = entry else {
                return orisu_ignore::WalkState::Continue;
            };

            let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
            buf.local.push(WalkEntry::new(entry.into_path(), is_dir));

            if buf.local.len() >= 256 {
                buf.shared.lock().unwrap().append(&mut buf.local);
            }

            orisu_ignore::WalkState::Continue
        })
    });

    Arc::try_unwrap(collected).unwrap().into_inner().unwrap()
}

/// Sets up a walker over all source roots with the default ignore rules, the `.gitignore` files and the source patterns.
/// With `targets`, the walk only enters directories that contain one of them.
fn create_walker(sources: &[SourceEntry], targets: Option<Vec<PathBuf>>) -> Option<WalkBuilder> {
    let mut roots: Vec<&PathBuf> = vec![];
    let mut ignores: Vec<(&PathBuf, Vec<String>)> = vec![];
    let mut emit = |base, pattern| match ignores.last_mut() {
        Some((prev_base, patterns)) if *prev_base == base => patterns.push(pattern),
        _ => ignores.push((base, vec![pattern])),
    };

    for source in sources {
        match source {
            SourceEntry::Auto { base } => roots.push(base),
            SourceEntry::Pattern { base, pattern } => {
                roots.push(base);
                if !pattern.contains("**") {
                    emit(base, format!("!{pattern}"));
                } else if let Some(extension) = Path::new(pattern).extension() {
                    emit(base, format!("!*.{}", extension.to_string_lossy()));
                }
            }
            SourceEntry::Ignored { base, pattern } => emit(base, pattern.to_owned()),
            SourceEntry::External { base } => {
                roots.push(base);
                emit(base, "!/**/*".to_owned());
                emit(base, BINARY_EXTENSIONS_GLOB.clone());
            }
        }
    }

    let (first_root, other_roots) = roots.split_first()?;
    let mut builder = WalkBuilder::new(first_root);
    builder.follow_links(true).hidden(false).git_global(false).require_git(inside_git_repo(first_root));
    let mut added = FxHashSet::default();
    for root in other_roots {
        if added.insert(root) {
            builder.add(root);
        }
    }

    for ignore in auto_source_detection::RULES.iter() {
        builder.add_gitignore(ignore.clone());
    }

    for (base, patterns) in ignores {
        let mut ignore_builder = GitignoreBuilder::new(base);
        for pattern in patterns {
            let _ = ignore_builder.add_line(None, &pattern);
        }
        if let Ok(ignore) = ignore_builder.build() {
            builder.add_gitignore(ignore);
        }
    }

    let auto_bases: Vec<(PathBuf, bool)> = sources
        .iter()
        .filter_map(|source| match source {
            SourceEntry::Auto { base } | SourceEntry::External { base } => Some((base.clone(), is_normal_path(base))),
            _ => None,
        })
        .collect();

    let pattern_sources: Vec<(PathBuf, String)> = sources
        .iter()
        .filter_map(|source| match source {
            SourceEntry::Pattern { base, pattern } => Some((base.into(), pattern.into())),
            _ => None,
        })
        .collect();

    builder.filter_entry(move |entry| {
        let path = entry.path();

        if let Some(targets) = &targets
            && !targets.iter().any(|target| target.starts_with(path))
        {
            return false;
        }

        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            return true;
        }

        let normal = is_normal_path(path);
        auto_bases.iter().any(|(base, base_is_normal)| path_starts_with(path, base, normal && *base_is_normal))
            || pattern_sources.iter().any(|(base, pattern)| {
                path.strip_prefix(base).is_ok_and(|remainder| {
                    let mut path_str = remainder.to_string_lossy().to_string();
                    if !path_str.starts_with("/") {
                        path_str = format!("/{path_str}");
                    }
                    glob_match(pattern, path_str.as_bytes())
                })
            })
    });

    Some(builder)
}
