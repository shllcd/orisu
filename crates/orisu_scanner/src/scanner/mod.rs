pub mod auto_source_detection;
pub mod detect_sources;
pub mod sources;

use crate::GlobEntry;
use crate::attributify::Attributify;
use crate::cache::{Cache, Stat};
use crate::extractor::{Extracted, Extractor};
use crate::glob::optimize_patterns;
use crate::manifest;
use crate::scanner::detect_sources::resolve_globs;
use crate::scanner::sources::{PublicSourceEntry, SourceEntry, Sources, public_source_entries_to_private_source_entries};
use auto_source_detection::BINARY_EXTENSIONS_GLOB;
use bstr::ByteSlice;
use fast_glob::glob_match;
use orisu_ignore::{WalkBuilder, gitignore::GitignoreBuilder};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub enum ChangedContent {
    File(PathBuf, String),
    Content(String, String),
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Base path to start scanning from
    pub base: Option<String>,

    /// Glob sources
    pub sources: Vec<GlobEntry>,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub candidates: Vec<String>,
    pub files: Vec<String>,
    pub globs: Vec<GlobEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct Scanner {
    /// Content sources
    sources: Sources,

    /// The walker to detect all files that we have to scan
    walker: Option<WalkBuilder>,

    /// All found extensions
    extensions: FxHashSet<String>,

    /// All files that we have to scan
    files: FxHashSet<PathBuf>,

    /// All directories, sub-directories, etc… we saw during source detection
    dirs: FxHashSet<PathBuf>,

    /// All generated globs, used for setting up watchers
    globs: Option<Vec<GlobEntry>>,

    /// Track unique set of candidates
    candidates: FxHashSet<String>,

    /// Track mtimes for files so incremental scans can skip unchanged files.
    mtimes: FxHashMap<PathBuf, Stat>,

    /// Files that were scanned during the last `scan()` call.
    scanned_files: Vec<String>,

    /// Whether we've completed at least one full scan. When false, we skip
    /// mtime tracking entirely so the initial build stays fast.
    has_scanned_once: bool,

    /// Whether sources have been scanned since the last `scan()` call
    sources_scanned: bool,

    attributify: Option<Attributify>,

    cache: Option<Cache>,

    /// Manifests of installed libraries whose sources are not scanned.
    manifests: Vec<PathBuf>,
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
        let sources = Sources::new(public_source_entries_to_private_source_entries(sources));

        let walker = create_walker(&sources);

        Self { sources, walker, manifests, ..Default::default() }
    }

    /// Also extracts markup attributes as attributify candidates.
    pub fn with_attributify(mut self, attributify: Attributify) -> Self {
        self.attributify = Some(attributify);
        self
    }

    /// Keeps the candidates of every scanned file in `dir`, keyed by content, so unchanged files are not extracted again
    /// by this or any later scanner.
    pub fn with_cache(mut self, dir: impl Into<PathBuf>) -> Self {
        let options = format!("{:?}", self.attributify);
        self.cache = Some(Cache::open(dir.into().join("scanner.bin"), &options));
        self
    }

    pub fn scan(&mut self) -> Vec<String> {
        self.sources_scanned = false;

        let prune = !self.has_scanned_once;
        let (scanned_blobs, css_files, files, extracted) = self.discover_sources();

        self.extract_candidates(scanned_blobs, css_files, extracted);
        self.scanned_files = files;
        if let Some(cache) = &mut self.cache {
            let _ = cache.save(prune);
        }

        let mut result = self.candidates.iter().cloned().collect::<Vec<_>>();
        result.par_sort_unstable();
        result
    }

    pub fn scan_content(&mut self, changed_content: Vec<ChangedContent>) -> Vec<String> {
        let (changed_files, changed_contents) = changed_content.into_iter().partition::<Vec<_>, _>(|x| match x {
            ChangedContent::File(_, _) => true,
            ChangedContent::Content(_, _) => false,
        });

        let mut content_to_scan: Vec<ChangedContent> = changed_contents;

        let changed_files = changed_files
            .into_iter()
            .filter_map(|changed_content| match changed_content {
                ChangedContent::File(file, extension) => {
                    let Ok(file) = dunce::canonicalize(file) else {
                        return None;
                    };
                    Some(ChangedContent::File(file, extension))
                }
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();

        let (known_files, mut new_unknown_files) = changed_files.into_iter().partition::<Vec<_>, _>(|changed_file| match changed_file {
            ChangedContent::Content(_, _) => unreachable!(),
            ChangedContent::File(file, _) => self.files.contains(file),
        });

        content_to_scan.extend(known_files);

        if !new_unknown_files.is_empty()
            && let Some(walk_builder) = &mut self.walker
        {
            for entry in walk_builder.build().filter_map(Result::ok) {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                let mut drop_file_indexes = vec![];
                for (idx, changed_file) in new_unknown_files.iter().enumerate().rev() {
                    let ChangedContent::File(file, _) = changed_file else {
                        continue;
                    };

                    if file == path {
                        self.files.insert(path.to_path_buf());
                        content_to_scan.push(changed_file.clone());
                        drop_file_indexes.push(idx);
                    }
                }

                if !drop_file_indexes.is_empty() {
                    drop_file_indexes.into_iter().for_each(|idx| {
                        new_unknown_files.remove(idx);
                    });
                }

                if new_unknown_files.is_empty() {
                    break;
                }
            }
        }

        let blobs = read_all_files(content_to_scan);
        self.extract_candidates(blobs, vec![], FxHashSet::default())
    }

    fn extract_candidates(&mut self, blobs: Vec<Vec<u8>>, css_files: Vec<PathBuf>, extracted: FxHashSet<String>) -> Vec<String> {
        let attributes = self.attributify.as_ref().map(|attributify| {
            blobs
                .par_iter()
                .fold(FxHashSet::default, |mut out, blob| {
                    attributify.extract(blob, &mut out);
                    out
                })
                .reduce(FxHashSet::default, |mut a, b| {
                    a.extend(b);
                    a
                })
        });
        let mut new_candidates = parse_all_blobs(&blobs);
        new_candidates.extend(attributes.into_iter().flatten());
        new_candidates.extend(extracted);

        if !css_files.is_empty() {
            let css_variables = extract_css_variables(&read_all_files(
                css_files.into_iter().map(|file| ChangedContent::File(file, "css".into())).collect(),
            ));

            new_candidates.extend(css_variables);
        }

        for existing in self.candidates.iter() {
            new_candidates.remove(existing);
        }

        self.candidates.extend(new_candidates.iter().cloned());

        let mut result: Vec<String> = new_candidates.into_iter().collect();
        result.par_sort_unstable();

        result
    }

    pub fn get_files(&mut self) -> Vec<String> {
        let _ = self.discover_sources();

        self.files.par_iter().filter_map(|x| x.clone().into_os_string().into_string().ok()).collect()
    }

    pub fn get_scanned_files(&self) -> Vec<String> {
        self.scanned_files.clone()
    }

    pub fn get_globs(&mut self) -> Vec<GlobEntry> {
        if let Some(globs) = &self.globs {
            return globs.clone();
        }

        let _ = self.discover_sources();

        let mut globs = vec![];
        for source in self.sources.iter() {
            match source {
                SourceEntry::Auto { base } | SourceEntry::External { base } => {
                    globs.extend(resolve_globs(base.to_path_buf(), &self.dirs, &self.extensions));
                }
                SourceEntry::Pattern { base, pattern } => {
                    globs.push(GlobEntry { base: base.to_string_lossy().to_string(), pattern: pattern.to_string() });
                }
                _ => {}
            }
        }

        globs = optimize_patterns(&globs);

        self.globs = Some(globs.clone());

        globs
    }

    pub fn get_normalized_sources(&self) -> Vec<GlobEntry> {
        self.sources
            .iter()
            .filter_map(|source| match source {
                SourceEntry::Auto { base } | SourceEntry::External { base } => {
                    Some(GlobEntry { base: base.to_string_lossy().to_string(), pattern: "**/*".to_string() })
                }
                SourceEntry::Pattern { base, pattern } => {
                    Some(GlobEntry { base: base.to_string_lossy().to_string(), pattern: pattern.to_string() })
                }
                _ => None,
            })
            .collect()
    }

    pub fn get_candidates_with_positions(&mut self, changed_content: ChangedContent) -> Vec<(String, usize)> {
        let content = read_changed_content(changed_content).unwrap_or_default();
        let original_content = &content;

        let content = content.replace("-[]", "XYZ");
        let offset = content.as_ptr() as usize;

        let mut extractor = Extractor::new(&content[..]);

        extractor
            .extract()
            .into_par_iter()
            .flat_map(|extracted| match extracted {
                Extracted::Candidate(s) => {
                    let i = s.as_ptr() as usize - offset;
                    let original = &original_content[i..i + s.len()];
                    if original.contains_str("-[]") {
                        return Some(unsafe { (String::from_utf8_unchecked(original.to_vec()), i) });
                    }

                    // SAFETY: When we parsed the candidates, we already guaranteed that the byte
                    Some(unsafe { (String::from_utf8_unchecked(s.to_vec()), i) })
                }

                _ => None,
            })
            .collect()
    }

    /// Content blobs to extract, CSS files, the changed files and candidates already extracted from caches and manifests.
    fn discover_sources(&mut self) -> (Vec<Vec<u8>>, Vec<PathBuf>, Vec<String>, FxHashSet<String>) {
        if self.sources_scanned {
            return Default::default();
        }
        self.sources_scanned = true;

        let mut all_entries = self.walker.as_mut().map(walk_parallel).unwrap_or_default();
        for manifest in &self.manifests {
            let mtime = manifest.metadata().ok().as_ref().and_then(Stat::of);
            all_entries.push((manifest.clone(), false, MANIFEST.to_owned(), mtime));
        }

        let mut css_files: Vec<PathBuf> = vec![];
        let mut content_paths: Vec<(PathBuf, String, Option<Stat>)> = vec![];
        let mut changed_files = vec![];
        let mut manifest_paths: Vec<PathBuf> = vec![];

        self.files.clear();
        self.dirs.clear();
        self.extensions.clear();
        self.globs = None;

        for (path, is_dir, extension, mtime) in all_entries {
            if is_dir {
                self.dirs.insert(path);
            } else {
                if !self.files.insert(path.clone()) {
                    continue;
                }
                self.extensions.insert(extension.clone());

                let changed = if self.has_scanned_once {
                    match mtime {
                        Some(mtime) => {
                            let prev = self.mtimes.insert(path.clone(), mtime);
                            prev.is_none_or(|prev| prev != mtime)
                        }
                        None => true,
                    }
                } else {
                    if let Some(mtime) = mtime {
                        self.mtimes.insert(path.clone(), mtime);
                    }

                    true
                };

                if !changed {
                    continue;
                }

                if let Ok(file) = path.clone().into_os_string().into_string() {
                    changed_files.push(file);
                }

                match extension.as_str() {
                    "css" => css_files.push(path),
                    MANIFEST => manifest_paths.push(path),
                    _ => content_paths.push((path, extension, mtime)),
                }
            }
        }

        self.mtimes.retain(|path, _| self.files.contains(path));

        let mut extracted: FxHashSet<String> = manifest_paths.iter().flat_map(|path| manifest::read(path)).collect();
        let scanned_blobs: Vec<Vec<u8>> = match &mut self.cache {
            Some(cache) => {
                extracted.extend(extract_cached(cache, content_paths, self.attributify.as_ref()));
                vec![]
            }
            None => content_paths
                .into_par_iter()
                .filter_map(|(path, ext, _)| {
                    let content = std::fs::read(&path).ok()?;
                    let processed = expand_variant_groups(pre_process_input(content, &ext));
                    if processed.is_empty() { None } else { Some(processed) }
                })
                .collect(),
        };

        if !self.has_scanned_once {
            self.has_scanned_once = true;
        }

        changed_files.par_sort_unstable();

        (scanned_blobs, css_files, changed_files, extracted)
    }
}

/// The extension that marks manifest entries among the walked files.
const MANIFEST: &str = "\0manifest";

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
            let blob = expand_variant_groups(pre_process_input(content, &extension));
            let mut candidates = parse_all_blobs(std::slice::from_ref(&blob));
            if let Some(attributify) = attributify {
                attributify.extract(&blob, &mut candidates);
            }
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

fn read_changed_content(c: ChangedContent) -> Option<Vec<u8>> {
    let (content, extension) = match c {
        ChangedContent::File(file, extension) => (std::fs::read(&file).ok()?, extension),

        ChangedContent::Content(contents, extension) => (contents.into_bytes(), extension),
    };

    Some(pre_process_input(content, &extension))
}

pub fn pre_process_input(content: Vec<u8>, extension: &str) -> Vec<u8> {
    use crate::extractor::pre_processors::*;

    match extension {
        "clj" | "cljs" | "cljc" => Clojure.process(&content),
        "heex" | "eex" | "ex" | "exs" => Elixir.process(&content),
        "cshtml" | "razor" => Razor.process(&content),
        "haml" => Haml.process(&content),
        "json" | "jsonl" | "ndjson" => Json.process(&content),
        "md" | "mdx" => Markdown.process(&content),
        "pug" => Pug.process(&content),
        "rb" | "erb" => Ruby.process(&content),
        "slim" | "slang" => Slim.process(&content),
        "svelte" => Svelte.process(&content),
        "rs" => Rust.process(&content),
        "tt" | "tt2" | "tx" => TemplateToolkit.process(&content),
        "twig" => Twig.process(&content),
        "vue" => Vue.process(&content),
        _ => content,
    }
}

fn read_all_files(changed_content: Vec<ChangedContent>) -> Vec<Vec<u8>> {
    changed_content.into_par_iter().filter_map(read_changed_content).map(expand_variant_groups).collect()
}

fn expand_variant_groups(content: Vec<u8>) -> Vec<u8> {
    orisu_candidate::group::expand(&content).unwrap_or(content)
}

fn extract_css_variables(blobs: &[Vec<u8>]) -> FxHashSet<String> {
    extract(blobs, |mut extractor| extractor.extract_variables_from_css())
}

fn parse_all_blobs(blobs: &[Vec<u8>]) -> FxHashSet<String> {
    extract(blobs, |mut extractor| extractor.extract())
}

fn extract<H>(blobs: &[Vec<u8>], handle: H) -> FxHashSet<String>
where
    H: Fn(Extractor) -> Vec<Extracted> + std::marker::Sync,
{
    blobs
        .par_iter()
        .flat_map(|blob| blob.par_split(|x| *x == b'\n'))
        .filter_map(|blob| {
            if blob.is_empty() {
                return None;
            }

            let extracted = handle(crate::extractor::Extractor::new(blob));
            if extracted.is_empty() {
                return None;
            }

            Some(FxHashSet::from_iter(extracted.into_iter().map(|x| match x {
                Extracted::Candidate(bytes) => bytes,
                Extracted::CssVariable(bytes) => bytes,
            })))
        })
        .reduce(Default::default, |mut a, b| {
            a.extend(b);
            a
        })
        .into_iter()
        .map(|s| unsafe { String::from_utf8_unchecked(s.to_vec()) })
        .collect()
}

type WalkEntry = (PathBuf, bool, String, Option<Stat>);

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

            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            let path = entry.into_path();

            if is_dir {
                buf.local.push((path, true, String::new(), None));
            } else {
                let ext = path.extension().and_then(|x| x.to_str()).unwrap_or_default().to_owned();
                let mtime = path.metadata().ok().as_ref().and_then(Stat::of);
                buf.local.push((path, false, ext, mtime));
            }

            if buf.local.len() >= 256 {
                buf.shared.lock().unwrap().append(&mut buf.local);
            }

            orisu_ignore::WalkState::Continue
        })
    });

    Arc::try_unwrap(collected).unwrap().into_inner().unwrap()
}

/// Sets up a WalkBuilder with all source roots, gitignore rules, and source pattern matching.
fn create_walker(sources: &Sources) -> Option<WalkBuilder> {
    let mut other_roots: FxHashSet<&PathBuf> = FxHashSet::default();
    let mut first_root: Option<&PathBuf> = None;

    let mut ignores: Vec<(&PathBuf, Vec<String>)> = Default::default();
    let mut emit = |base, pattern| match ignores.last_mut() {
        Some((prev_base, patterns)) if *prev_base == base => {
            patterns.push(pattern);
        }
        _ => {
            ignores.push((base, vec![pattern]));
        }
    };

    for source in sources.iter() {
        match source {
            SourceEntry::Auto { base } => {
                if first_root.is_none() {
                    first_root = Some(base);
                } else {
                    other_roots.insert(base);
                }
            }
            SourceEntry::Pattern { base, pattern } => {
                let pattern = pattern.to_owned();

                if first_root.is_none() {
                    first_root = Some(base);
                } else {
                    other_roots.insert(base);
                }

                if !pattern.contains("**") {
                    emit(base, format!("!{}", pattern));
                } else {
                    if let Some(extension) = Path::new(&pattern).extension() {
                        emit(base, format!("!*.{}", extension.to_string_lossy()));
                    }
                }
            }
            SourceEntry::Ignored { base, pattern } => {
                emit(base, pattern.to_owned());
            }
            SourceEntry::External { base } => {
                if first_root.is_none() {
                    first_root = Some(base);
                } else {
                    other_roots.insert(base);
                }

                emit(base, "!/**/*".to_owned());

                emit(base, BINARY_EXTENSIONS_GLOB.clone());
            }
        }
    }

    let mut builder = WalkBuilder::new(first_root?);

    builder.follow_links(true);

    builder.hidden(false);

    builder.git_global(false);

    builder.require_git(false);

    for parent in first_root?.ancestors() {
        if parent.join(".git").exists() {
            builder.require_git(true);
            break;
        }
    }

    for root in other_roots {
        builder.add(root);
    }

    for ignore in auto_source_detection::RULES.iter() {
        builder.add_gitignore(ignore.clone());
    }

    for (base, patterns) in ignores {
        let mut ignore_builder = GitignoreBuilder::new(base);
        for pattern in patterns {
            ignore_builder.add_line(None, &pattern).unwrap();
        }
        let ignore = ignore_builder.build().unwrap();
        builder.add_gitignore(ignore);
    }

    let auto_bases: Vec<PathBuf> = sources
        .iter()
        .filter_map(|source| match source {
            SourceEntry::Auto { base } | SourceEntry::External { base } => Some(base.clone()),
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

        if entry.file_type().is_some_and(|ft| ft.is_file()) {
            let mut matches = false;

            for base in &auto_bases {
                if path.starts_with(base) {
                    matches = true;
                    break;
                }
            }

            if !matches {
                for (base, pattern) in &pattern_sources {
                    let remainder = path.strip_prefix(base);
                    if remainder.is_ok_and(|remainder| {
                        let mut path_str = remainder.to_string_lossy().to_string();
                        if !path_str.starts_with("/") {
                            path_str = format!("/{path_str}");
                        }
                        glob_match(pattern, path_str.as_bytes())
                    }) {
                        matches = true;
                        break;
                    }
                }
            }

            if !matches {
                return false;
            }
        }

        true
    });

    Some(builder)
}
