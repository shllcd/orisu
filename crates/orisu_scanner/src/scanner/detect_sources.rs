use crate::GlobEntry;
use crate::scanner::auto_source_detection::IGNORED_CONTENT_DIRS;
use globwalk::DirEntry;
use rustc_hash::FxHashSet;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync;
use walkdir::WalkDir;

static KNOWN_EXTENSIONS: sync::LazyLock<Vec<&'static str>> = sync::LazyLock::new(|| {
    include_str!("fixtures/template-extensions.txt").trim().lines().filter(|x| !x.starts_with('#')).filter(|x| !x.is_empty()).collect()
});

fn sort_by_dir_and_name(a: &DirEntry, z: &DirEntry) -> Ordering {
    match (a.file_type().is_dir(), z.file_type().is_dir()) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => a.file_name().cmp(z.file_name()),
    }
}

pub fn resolve_globs(base: PathBuf, dirs: &FxHashSet<PathBuf>, extensions: &FxHashSet<String>) -> Vec<GlobEntry> {
    let mut found_extensions: FxHashSet<String> = FxHashSet::from_iter(KNOWN_EXTENSIONS.iter().map(|x| x.to_string()));
    found_extensions.extend(extensions.iter().cloned());

    let forced_static_directories: FxHashSet<PathBuf> = FxHashSet::from_iter(vec![base.join("public")]);

    let mut deep_globable_directories: FxHashSet<PathBuf> = Default::default();

    let mut shallow_globable_directories: FxHashSet<PathBuf> = Default::default();

    let mut it = WalkDir::new(&base).sort_by(sort_by_dir_and_name).into_iter();

    while let Some(Ok(entry)) = it.next() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        if !dirs.contains(path) {
            let mut path = path;
            while let Some(parent) = path.parent() {
                if parent == base {
                    break;
                }

                shallow_globable_directories.insert(parent.to_path_buf());
                path = parent
            }

            it.skip_current_dir();
        }
    }

    let mut it = WalkDir::new(&base).sort_by(sort_by_dir_and_name).into_iter();

    while let Some(Ok(entry)) = it.next() {
        let path = entry.path();
        if path.is_file() {
            continue;
        }

        if path == base {
            continue;
        }

        if IGNORED_CONTENT_DIRS.iter().any(|dir| match path.file_name() {
            Some(name) => name == *dir,
            None => false,
        }) {
            it.skip_current_dir();
            continue;
        }

        if !dirs.contains(path) {
            continue;
        }

        if shallow_globable_directories.contains(path) {
            continue;
        }

        if forced_static_directories.contains(path) {
            it.skip_current_dir();
            continue;
        }

        deep_globable_directories.insert(path.to_path_buf());
        it.skip_current_dir();
    }

    let mut extension_list = found_extensions.clone().into_iter().collect::<Vec<_>>();

    extension_list.sort();

    let extension_list = extension_list.join(",");

    let shallow_globs = shallow_globable_directories
        .iter()
        .map(|path| GlobEntry { base: path.display().to_string(), pattern: format!("*/*.{{{}}}", extension_list) });

    let deep_globs = deep_globable_directories
        .iter()
        .map(|path| GlobEntry { base: path.display().to_string(), pattern: format!("**/*.{{{}}}", extension_list) });

    shallow_globs.chain(deep_globs).chain(vec![GlobEntry { base: base.to_string_lossy().into(), pattern: "*".into() }]).collect::<Vec<_>>()
}
