use crate::GlobEntry;
use bexpand::Expression;
use orisu_ignore::gitignore::Gitignore;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Component, Path, PathBuf};

use super::auto_source_detection::IGNORED_CONTENT_DIRS;

#[derive(Debug, Clone)]
pub struct PublicSourceEntry {
    /// Base path of the glob
    pub base: String,

    /// Glob pattern
    pub pattern: String,

    /// Negated flag
    pub negated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourceEntry {
    /// Auto source detection
    ///
    /// Represented by:
    ///
    /// ```css
    /// @source "src";`
    /// @source "src/**/*";`
    /// ```
    Auto { base: PathBuf },

    /// Explicit source pattern regardless of any auto source detection rules
    ///
    /// Represented by:
    ///
    /// ```css
    /// @source "src/**/*.html";`
    /// ```
    Pattern { base: PathBuf, pattern: String },

    /// Ignored pattern
    ///
    /// Represented by:
    ///
    /// ```css
    /// @source not "src";`
    /// @source not "src/**/*.html";`
    /// ```
    Ignored { base: PathBuf, pattern: String },

    /// External sources are directories that are ignored (by us or .gitignore rules), but should be
    /// included bypassing the default ignore rules.
    ///
    /// Represented by:
    ///
    /// ```css
    /// @source "../node_modules/my-lib";`
    /// @source "../node_modules/my-lib/**/*";`
    /// ```
    External { base: PathBuf },
}

#[derive(Debug, Clone, Default)]
pub struct Sources {
    sources: Vec<SourceEntry>,
}

impl Sources {
    pub fn new(sources: Vec<SourceEntry>) -> Self {
        Self { sources }
    }

    pub fn iter(&self) -> impl Iterator<Item = &SourceEntry> {
        self.sources.iter()
    }
}

/// When dealing with a pattern, then it could be that we end up with:
///
/// ```json
/// { base: '/some/folder', pattern: 'foo.ts' }
/// ```
///
/// If we just emit `!foo.ts` for the `/some/folder` path, then _everything_ else in that folder
/// would still be walked (but the result will be ignored).
///
/// Instead, we have to ensure that we ignore everything in that folder _except_ for the `foo.ts`
/// pattern.
///
/// This should be equivalent to:
/// ```gitignore
/// *
/// !foo.ts
/// ```
///
/// However, we have to be careful that we don't start ignoring files/folders that already exist.
/// ```css
/// @source "./some/folder/foo.ts";
/// @source "./some/folder/bar.ts";
/// ```
/// Would result in:
/// ```json
/// { base: '/some/folder', pattern: 'foo.ts' }
/// { base: '/some/folder', pattern: 'bar.ts' }
/// ```
///
/// If we were to blindly emit `*` for each pattern, then the `.gitignore` equivalent would look like
/// this:
/// ```gitignore
/// *
/// !foo.ts
/// *
/// !bar.ts
/// ```
///
/// This would result in ignoring the `foo.ts` file as well. Therefore we only want to insert
/// this `*` pattern when nothing else exists yet.
///
/// There is another problem that we need to solve. Let's say you have a pattern that contains a `*`
/// in the pattern:
/// ```css
/// @source './src/ba*/*.html';
/// ```
///
/// This would result in
/// ```json
/// { base: '/src', pattern: '/ba*/*.html' }
/// ```
///
/// If we now inject the `*` pattern for the `/src` folder, then we wouldn't scan any `ba*` folders
/// (e.g. `bar` or `baz`). For this, we have to make sure that we add inverse patterns for these
/// folders. This would essentially result in:
/// ```gitignore
/// *                     ← ignore everything
/// !/ba*/                ← except for the `ba*/` pattern, so we scan these folders
/// !/ba*/*.html          ← then ensure we scan the `*.html` files in it as well
/// ```
///
fn expand_restricted_patterns(sources: Vec<SourceEntry>) -> Vec<SourceEntry> {
    let unrestricted_roots = sources
        .iter()
        .filter_map(|source| match source {
            SourceEntry::Auto { base } | SourceEntry::External { base } => Some(base.clone()),
            SourceEntry::Pattern { base, pattern } if pattern.contains("**") => Some(base.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    let pattern_roots = sources
        .iter()
        .filter_map(|source| match source {
            SourceEntry::Pattern { base, .. } => Some(base.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    let mut restricted_roots: FxHashSet<PathBuf> = FxHashSet::default();
    let mut expanded = vec![];

    for source in sources {
        let SourceEntry::Pattern { base, pattern } = &source else {
            expanded.push(source);
            continue;
        };

        if unrestricted_roots.iter().any(|root| base.starts_with(root)) {
            expanded.push(source);
            continue;
        }

        if restricted_roots.insert(base.clone()) {
            let has_nested_root = unrestricted_roots.iter().any(|root| root.starts_with(base))
                || pattern_roots.iter().any(|root| root != base && root.starts_with(base));

            let pattern = if has_nested_root { "/*" } else { "*" };

            expanded.push(SourceEntry::Ignored { base: base.clone(), pattern: pattern.to_owned() });
        }

        {
            let mut dir = PathBuf::new();
            let mut components = Path::new(pattern).components().peekable();

            while let Some(component) = components.next() {
                if components.peek().is_none() {
                    break;
                }

                match component {
                    Component::Prefix(_) | Component::RootDir | Component::CurDir => continue,
                    Component::ParentDir | Component::Normal(_) => dir.push(component),
                }

                expanded.push(SourceEntry::Ignored {
                    base: base.clone(),
                    pattern: format!("!/{}/", path_to_posix_string(&dir).trim_start_matches('/')),
                });
            }
        }

        expanded.push(source);
    }

    expanded
}

impl PublicSourceEntry {
    /// Optimize the PublicSourceEntry by trying to move all the static parts of the pattern to the
    /// base of the PublicSourceEntry.
    ///
    /// ```diff
    /// - { base: '/', pattern: 'src/**/*.html'}
    /// + { base: '/src', pattern: '**/*.html'}
    /// ```
    ///
    /// A file stays in the `pattern` part, because the `base` should only be a directory.
    ///
    /// ```diff
    /// - { base: '/', pattern: 'src/examples/index.html'}
    /// + { base: '/src/examples', pattern: 'index.html'}
    /// ```
    ///
    /// A folder will be moved to the `base` part, and the `pattern` will be set to `**/*`.
    ///
    /// ```diff
    /// - { base: '/', pattern: 'src/examples'}
    /// + { base: '/src/examples', pattern: '**/*'}
    /// ```
    ///
    /// In addition, we will canonicalize the base path so we always work with the correctly
    /// resolved path.
    pub fn optimize(&mut self) {
        let Ok(mut base) = dunce::canonicalize(&self.base) else {
            return;
        };

        let mut new_pattern = PathBuf::new();
        enum ComponentStage {
            Base,
            Pattern,
        }
        let mut stage = ComponentStage::Base;

        let mut components = Path::new(&self.pattern).components().peekable();
        while let Some(component) = components.next() {
            match stage {
                ComponentStage::Base => match component {
                    Component::CurDir => {}

                    Component::ParentDir => {
                        base.pop();
                    }

                    Component::Normal(part) if part.to_string_lossy().contains("*") => {
                        new_pattern.push(component);
                        stage = ComponentStage::Pattern;
                    }

                    Component::Normal(part) if components.peek().is_some() => {
                        base.push(part);
                    }

                    Component::Normal(part) => {
                        let full_path = base.join(part);
                        if full_path.is_dir() {
                            base.push(part);
                        } else {
                            new_pattern.push(part);
                        }
                    }

                    Component::Prefix(_) => {
                        base.clear();
                        base.push(component);
                    }
                    Component::RootDir => {
                        #[cfg(not(windows))]
                        base.clear();
                        base.push(component);
                    }
                },
                ComponentStage::Pattern => {
                    new_pattern.push(component);
                }
            }
        }

        self.base = base.to_string_lossy().to_string();
        self.pattern = path_to_posix_string(&new_pattern);

        if self.pattern.is_empty() {
            self.pattern = "/**/*".to_owned();
        } else if !self.pattern.starts_with("/") {
            self.pattern = format!("/{}", self.pattern);
        }
    }
}

fn path_to_posix_string(path: &Path) -> String {
    let mut parts = Vec::new();
    let mut is_rooted = false;

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                parts.push(prefix.as_os_str().to_string_lossy().to_string());
            }
            Component::RootDir => {
                is_rooted = true;
                if parts.is_empty() {
                    parts.push(String::new());
                }
            }
            Component::CurDir => {
                parts.push(".".to_string());
            }
            Component::ParentDir => {
                parts.push("..".to_string());
            }
            Component::Normal(part) => {
                parts.push(part.to_string_lossy().to_string());
            }
        }
    }

    let result = parts.join("/");
    if result.is_empty() && is_rooted { "/".to_string() } else { result }
}

/// For each public source entry:
///
/// 1. Perform brace expansion
///
/// ```diff
/// - { base: '/', pattern: 'src/{foo,bar}.html'}
/// + { base: '/', pattern: 'src/foo.html'}
/// + { base: '/', pattern: 'src/bar.html'}
/// ```
///
/// 2. Hoist static parts, e.g.:
///
/// ```diff
/// - { base: '/', pattern: 'src/**/*.html'}
/// + { base: '/src', pattern: '**/*.html'}
/// ```
///
/// 3. Convert to private SourceEntry
///
pub fn public_source_entries_to_private_source_entries(sources: Vec<PublicSourceEntry>) -> Vec<SourceEntry> {
    let expanded_globs = sources
        .into_iter()
        .flat_map(|source| {
            let expression: Result<Expression, _> = source.pattern[..].try_into();
            let Ok(expression) = expression else {
                return vec![source];
            };

            expression
                .into_iter()
                .filter_map(Result::ok)
                .map(move |pattern| PublicSourceEntry { base: source.base.clone(), pattern: pattern.into(), negated: source.negated })
                .collect::<Vec<_>>()
        })
        .map(|mut public_source| {
            public_source.optimize();
            public_source
        })
        .collect::<Vec<_>>();

    let mut gitignores: FxHashMap<PathBuf, Option<Gitignore>> = FxHashMap::default();

    let cwd = std::env::current_dir().map(|cwd| dunce::canonicalize(&cwd).unwrap_or(cwd)).ok();

    let sources = expanded_globs
        .into_iter()
        .map(|public_source| {
            let mut source: SourceEntry = public_source.into();

            if let SourceEntry::Auto { ref base } = source {
                let inside_git_repo = base.ancestors().any(|dir| dir.join(".git").exists());

                for dir in base.ancestors() {
                    let gitignore = gitignores.entry(dir.to_path_buf()).or_insert_with(|| {
                        let path = dir.join(".gitignore");

                        path.is_file().then(|| Gitignore::new(&path).0)
                    });

                    if let Some(gitignore) = gitignore
                        && gitignore.matched_path_or_any_parents(base, true).is_ignore()
                    {
                        source = SourceEntry::External { base: base.into() };
                        break;
                    }

                    if dir.join(".git").exists() {
                        break;
                    }

                    if !inside_git_repo && cwd.as_ref().is_some_and(|cwd| cwd.starts_with(dir)) {
                        break;
                    }
                }
            }

            source
        })
        .collect::<Vec<SourceEntry>>();

    expand_restricted_patterns(sources)
}

/// Convert a public source entry to a source entry
impl From<PublicSourceEntry> for SourceEntry {
    fn from(value: PublicSourceEntry) -> Self {
        if value.negated {
            return SourceEntry::Ignored { base: value.base.into(), pattern: value.pattern };
        }

        let auto = value.pattern == "/**/*" || PathBuf::from(&value.base).join(&value.pattern).is_dir();

        if !auto {
            return SourceEntry::Pattern { base: value.base.into(), pattern: value.pattern };
        }

        let inside_ignored_content_dir = IGNORED_CONTENT_DIRS.iter().any(|dir| {
            value.base.contains(&format!("{}{}{}", std::path::MAIN_SEPARATOR, dir, std::path::MAIN_SEPARATOR))
                || value.base.ends_with(&format!("{}{}", std::path::MAIN_SEPARATOR, dir))
        });

        match inside_ignored_content_dir {
            false => SourceEntry::Auto { base: value.base.into() },
            true => SourceEntry::External { base: value.base.into() },
        }
    }
}

impl From<GlobEntry> for SourceEntry {
    fn from(value: GlobEntry) -> Self {
        SourceEntry::Pattern { base: PathBuf::from(value.base), pattern: value.pattern }
    }
}

impl From<SourceEntry> for GlobEntry {
    fn from(value: SourceEntry) -> Self {
        match value {
            SourceEntry::Auto { base } | SourceEntry::External { base } => {
                GlobEntry { base: base.to_string_lossy().into(), pattern: "**/*".into() }
            }
            SourceEntry::Pattern { base, pattern } => GlobEntry { base: base.to_string_lossy().into(), pattern: pattern.clone() },
            SourceEntry::Ignored { base, pattern } => GlobEntry { base: base.to_string_lossy().into(), pattern: pattern.clone() },
        }
    }
}

impl From<&SourceEntry> for GlobEntry {
    fn from(value: &SourceEntry) -> Self {
        match value {
            SourceEntry::Auto { base } | SourceEntry::External { base } => {
                GlobEntry { base: base.to_string_lossy().into(), pattern: "**/*".into() }
            }
            SourceEntry::Pattern { base, pattern } => GlobEntry { base: base.to_string_lossy().into(), pattern: pattern.clone() },
            SourceEntry::Ignored { base, pattern } => GlobEntry { base: base.to_string_lossy().into(), pattern: pattern.clone() },
        }
    }
}
