use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs::{File, FileType},
    io::{self, BufRead},
    path::{Path, PathBuf},
    sync::{Arc, RwLock, Weak},
};

use crate::{
    gitignore::{self, Gitignore, GitignoreBuilder},
    overrides::{self, Override},
    pathutil::{is_hidden, strip_prefix},
    types::{self, Types},
    walk::DirEntry,
    {Error, Match, PartialErrorBuilder},
};

/// IgnoreMatch represents information about where a match came from when using
/// the `Ignore` matcher.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct IgnoreMatch<'a>(IgnoreMatchInner<'a>);

/// IgnoreMatchInner describes precisely where the match information came from.
/// This is private to allow expansion to more matchers in the future.
#[derive(Clone, Debug)]
#[allow(dead_code)]
enum IgnoreMatchInner<'a> {
    Override(overrides::Glob<'a>),
    Gitignore(&'a gitignore::Glob),
    Types(types::Glob<'a>),
    Hidden,
}

impl<'a> IgnoreMatch<'a> {
    fn overrides(x: overrides::Glob<'a>) -> IgnoreMatch<'a> {
        IgnoreMatch(IgnoreMatchInner::Override(x))
    }

    fn gitignore(x: &'a gitignore::Glob) -> IgnoreMatch<'a> {
        IgnoreMatch(IgnoreMatchInner::Gitignore(x))
    }

    fn types(x: types::Glob<'a>) -> IgnoreMatch<'a> {
        IgnoreMatch(IgnoreMatchInner::Types(x))
    }

    fn hidden() -> IgnoreMatch<'static> {
        IgnoreMatch(IgnoreMatchInner::Hidden)
    }
}

/// Options for the ignore matcher, shared between the matcher itself and the
/// builder.
#[derive(Clone, Copy, Debug)]
struct IgnoreOptions {
    /// Whether to ignore hidden file paths or not.
    hidden: bool,
    /// Whether to read .ignore files.
    ignore: bool,
    /// Whether to respect any ignore files in parent directories.
    parents: bool,
    /// Whether to read git's global gitignore file.
    git_global: bool,
    /// Whether to read .gitignore files.
    git_ignore: bool,
    /// Whether to read .git/info/exclude files.
    git_exclude: bool,
    /// Whether to ignore files case insensitively
    ignore_case_insensitive: bool,
    /// Whether a git repository must be present in order to apply any
    /// git-related ignore rules.
    require_git: bool,
}

/// Ignore is a matcher useful for recursively walking one or more directories.
#[derive(Clone, Debug)]
pub(crate) struct Ignore(Arc<IgnoreInner>);

#[derive(Clone, Debug)]
struct IgnoreInner {
    /// A map of all existing directories that have already been
    /// compiled into matchers.
    ///
    /// Note that this is never used during matching, only when adding new
    /// parent directory matchers. This avoids needing to rebuild glob sets for
    /// parent directories if many paths are being searched.
    compiled: Arc<RwLock<HashMap<OsString, Weak<IgnoreInner>>>>,
    /// The path to the directory that this matcher was built from.
    dir: PathBuf,
    /// An override matcher (default is empty).
    overrides: Arc<Override>,
    /// A file type matcher.
    types: Arc<Types>,
    /// The parent directory to match next.
    ///
    /// If this is the root directory or there are otherwise no more
    /// directories to match, then `parent` is `None`.
    parent: Option<Ignore>,
    /// Whether this is an absolute parent matcher, as added by add_parent.
    is_absolute_parent: bool,
    /// The absolute base path of this matcher. Populated only if parent
    /// directories are added.
    absolute_base: Option<Arc<PathBuf>>,
    /// The directory that gitignores should be interpreted relative to.
    ///
    /// Usually this is the directory containing the gitignore file. But in
    /// some cases, like for global gitignores or for gitignores specified
    /// explicitly, this should generally be set to the current working
    /// directory. This is only used for global gitignores or "explicit"
    /// gitignores.
    ///
    /// When `None`, this means the CWD could not be determined or is unknown.
    /// In this case, global gitignore files are ignored because they otherwise
    /// cannot be matched correctly.
    global_gitignores_relative_to: Option<PathBuf>,
    /// Explicit global ignore matchers specified by the caller.
    explicit_ignores: Arc<Vec<Gitignore>>,
    /// Ignore files used in addition to `.ignore`
    custom_ignore_filenames: Arc<Vec<OsString>>,
    /// The matcher for custom ignore files
    custom_ignore_matcher: Gitignore,
    /// The matcher for .ignore files.
    ignore_matcher: Gitignore,
    /// A global gitignore matcher, usually from $XDG_CONFIG_HOME/git/ignore.
    git_global_matcher: Arc<Gitignore>,
    /// The matcher for .gitignore files.
    git_ignore_matcher: Gitignore,
    /// Special matcher for `.git/info/exclude` files.
    git_exclude_matcher: Gitignore,
    /// Whether this directory contains a .git sub-directory.
    has_git: bool,
    /// Ignore config.
    opts: IgnoreOptions,
}

impl Ignore {
    /// Return the directory path of this matcher.
    pub(crate) fn path(&self) -> &Path {
        &self.0.dir
    }

    /// Return true if this matcher has no parent.
    pub(crate) fn is_root(&self) -> bool {
        self.0.parent.is_none()
    }

    /// Returns true if this matcher was added via the `add_parents` method.
    pub(crate) fn is_absolute_parent(&self) -> bool {
        self.0.is_absolute_parent
    }

    /// Return this matcher's parent, if one exists.
    pub(crate) fn parent(&self) -> Option<Ignore> {
        self.0.parent.clone()
    }

    /// Create a new `Ignore` matcher with the parent directories of `dir`.
    ///
    /// Note that this can only be called on an `Ignore` matcher with no
    /// parents (i.e., `is_root` returns `true`). This will panic otherwise.
    pub(crate) fn add_parents<P: AsRef<Path>>(&self, path: P) -> (Ignore, Option<Error>) {
        if !self.0.opts.parents && !self.0.opts.git_ignore && !self.0.opts.git_exclude && !self.0.opts.git_global {
            return (self.clone(), None);
        }
        if !self.is_root() {
            panic!("Ignore::add_parents called on non-root matcher");
        }
        let absolute_base = match dunce::canonicalize(path.as_ref()) {
            Ok(path) => Arc::new(path),
            Err(_) => {
                return (self.clone(), None);
            }
        };
        let mut parents = vec![];
        let mut path = &**absolute_base;
        while let Some(parent) = path.parent() {
            parents.push(parent);
            path = parent;
        }
        let mut errs = PartialErrorBuilder::default();
        let mut ig = self.clone();
        for parent in parents.into_iter().rev() {
            let mut compiled = self.0.compiled.write().unwrap();
            if let Some(weak) = compiled.get(parent.as_os_str()) {
                if let Some(prebuilt) = weak.upgrade() {
                    ig = Ignore(prebuilt);
                    continue;
                }
            }
            let (mut igtmp, err) = ig.add_child_path(parent);
            errs.maybe_push(err);
            igtmp.is_absolute_parent = true;
            igtmp.absolute_base = Some(absolute_base.clone());
            igtmp.has_git = if self.0.opts.require_git && self.0.opts.git_ignore {
                parent.join(".git").exists() || parent.join(".jj").exists()
            } else {
                false
            };
            let ig_arc = Arc::new(igtmp);
            ig = Ignore(ig_arc.clone());
            compiled.insert(parent.as_os_str().to_os_string(), Arc::downgrade(&ig_arc));
        }
        (ig, errs.into_error_option())
    }

    /// Create a new `Ignore` matcher for the given child directory.
    ///
    /// Since building the matcher may require reading from multiple
    /// files, it's possible that this method partially succeeds. Therefore,
    /// a matcher is always returned (which may match nothing) and an error is
    /// returned if it exists.
    ///
    /// Note that all I/O errors are completely ignored.
    pub(crate) fn add_child<P: AsRef<Path>>(&self, dir: P) -> (Ignore, Option<Error>) {
        let (ig, err) = self.add_child_path(dir.as_ref());
        (Ignore(Arc::new(ig)), err)
    }

    /// Like add_child, but takes a full path and returns an IgnoreInner.
    fn add_child_path(&self, dir: &Path) -> (IgnoreInner, Option<Error>) {
        let git_type = if self.0.opts.require_git && (self.0.opts.git_ignore || self.0.opts.git_exclude) {
            dir.join(".git").metadata().ok().map(|md| md.file_type())
        } else {
            None
        };
        let has_git = git_type.is_some() || dir.join(".jj").exists();

        let mut errs = PartialErrorBuilder::default();
        let custom_ig_matcher = if self.0.custom_ignore_filenames.is_empty() {
            Gitignore::empty()
        } else {
            let (m, err) = create_gitignore(&dir, &dir, &self.0.custom_ignore_filenames, self.0.opts.ignore_case_insensitive);
            errs.maybe_push(err);
            m
        };
        let ig_matcher = if !self.0.opts.ignore {
            Gitignore::empty()
        } else {
            let (m, err) = create_gitignore(&dir, &dir, &[".ignore"], self.0.opts.ignore_case_insensitive);
            errs.maybe_push(err);
            m
        };
        let gi_matcher = if !self.0.opts.git_ignore {
            Gitignore::empty()
        } else {
            let (m, err) = create_gitignore(&dir, &dir, &[".gitignore"], self.0.opts.ignore_case_insensitive);
            errs.maybe_push(err);
            m
        };

        let gi_exclude_matcher = if !self.0.opts.git_exclude {
            Gitignore::empty()
        } else {
            match resolve_git_commondir(dir, git_type) {
                Ok(git_dir) => {
                    let (m, err) = create_gitignore(&dir, &git_dir, &["info/exclude"], self.0.opts.ignore_case_insensitive);
                    errs.maybe_push(err);
                    m
                }
                Err(err) => {
                    errs.maybe_push(err);
                    Gitignore::empty()
                }
            }
        };
        let ig = IgnoreInner {
            compiled: self.0.compiled.clone(),
            dir: dir.to_path_buf(),
            overrides: self.0.overrides.clone(),
            types: self.0.types.clone(),
            parent: Some(self.clone()),
            is_absolute_parent: false,
            absolute_base: self.0.absolute_base.clone(),
            global_gitignores_relative_to: self.0.global_gitignores_relative_to.clone(),
            explicit_ignores: self.0.explicit_ignores.clone(),
            custom_ignore_filenames: self.0.custom_ignore_filenames.clone(),
            custom_ignore_matcher: custom_ig_matcher,
            ignore_matcher: ig_matcher,
            git_global_matcher: self.0.git_global_matcher.clone(),
            git_ignore_matcher: gi_matcher,
            git_exclude_matcher: gi_exclude_matcher,
            has_git,
            opts: self.0.opts,
        };
        (ig, errs.into_error_option())
    }

    /// Returns true if at least one type of ignore rule should be matched.
    fn has_any_ignore_rules(&self) -> bool {
        let opts = self.0.opts;
        let has_custom_ignore_files = !self.0.custom_ignore_filenames.is_empty();
        let has_explicit_ignores = !self.0.explicit_ignores.is_empty();

        opts.ignore || opts.git_global || opts.git_ignore || opts.git_exclude || has_custom_ignore_files || has_explicit_ignores
    }

    /// Like `matched`, but works with a directory entry instead.
    pub(crate) fn matched_dir_entry<'a>(&'a self, dent: &DirEntry) -> Match<IgnoreMatch<'a>> {
        let m = self.matched(dent.path(), dent.is_dir());
        if m.is_none() && self.0.opts.hidden && is_hidden(dent) {
            return Match::Ignore(IgnoreMatch::hidden());
        }
        m
    }

    /// Returns a match indicating whether the given file path should be
    /// ignored or not.
    ///
    /// The match contains information about its origin.
    fn matched<'a, P: AsRef<Path>>(&'a self, path: P, is_dir: bool) -> Match<IgnoreMatch<'a>> {
        let mut path = path.as_ref();
        if let Some(p) = strip_prefix("./", path) {
            path = p;
        }
        if !self.0.overrides.is_empty() {
            let mat = self.0.overrides.matched(path, is_dir).map(IgnoreMatch::overrides);
            if !mat.is_none() {
                return mat;
            }
        }
        let mut whitelisted = Match::None;
        if self.has_any_ignore_rules() {
            let mat = self.matched_ignore(path, is_dir);
            if mat.is_ignore() {
                return mat;
            } else if mat.is_whitelist() {
                whitelisted = mat;
            }
        }
        if !self.0.types.is_empty() {
            let mat = self.0.types.matched(path, is_dir).map(IgnoreMatch::types);
            if mat.is_ignore() {
                return mat;
            } else if mat.is_whitelist() {
                whitelisted = mat;
            }
        }
        whitelisted
    }

    /// Performs matching only on the ignore files for this directory and
    /// all parent directories.
    fn matched_ignore<'a>(&'a self, path: &Path, is_dir: bool) -> Match<IgnoreMatch<'a>> {
        let (mut m_custom_ignore, mut m_ignore, mut m_gi, mut m_gi_exclude, mut m_explicit) =
            (Match::None, Match::None, Match::None, Match::None, Match::None);
        let any_git = !self.0.opts.require_git || self.parents().any(|ig| ig.0.has_git);
        let mut saw_git = false;
        for ig in self.parents().take_while(|ig| !ig.0.is_absolute_parent) {
            if m_custom_ignore.is_none() {
                m_custom_ignore = ig.0.custom_ignore_matcher.matched(path, is_dir).map(IgnoreMatch::gitignore);
            }
            if m_ignore.is_none() {
                m_ignore = ig.0.ignore_matcher.matched(path, is_dir).map(IgnoreMatch::gitignore);
            }
            if any_git && !saw_git && m_gi.is_none() {
                m_gi = ig.0.git_ignore_matcher.matched(path, is_dir).map(IgnoreMatch::gitignore);
            }
            if any_git && !saw_git && m_gi_exclude.is_none() {
                m_gi_exclude = ig.0.git_exclude_matcher.matched(path, is_dir).map(IgnoreMatch::gitignore);
            }
            saw_git = saw_git || ig.0.has_git;
        }
        if self.0.opts.parents {
            if let Some(_) = self.absolute_base() {
                for ig in self.parents().skip_while(|ig| !ig.0.is_absolute_parent) {
                    if m_custom_ignore.is_none() {
                        m_custom_ignore = ig.0.custom_ignore_matcher.matched(&path, is_dir).map(IgnoreMatch::gitignore);
                    }
                    if m_ignore.is_none() {
                        m_ignore = ig.0.ignore_matcher.matched(&path, is_dir).map(IgnoreMatch::gitignore);
                    }
                    if any_git && !saw_git && m_gi.is_none() {
                        m_gi = ig.0.git_ignore_matcher.matched(&path, is_dir).map(IgnoreMatch::gitignore);
                    }
                    if any_git && !saw_git && m_gi_exclude.is_none() {
                        m_gi_exclude = ig.0.git_exclude_matcher.matched(&path, is_dir).map(IgnoreMatch::gitignore);
                    }
                    saw_git = saw_git || ig.0.has_git;
                }
            }
        }
        for gi in self.0.explicit_ignores.iter().rev() {
            if !path.starts_with(gi.path()) {
                continue;
            }
            if !m_explicit.is_none() {
                break;
            }
            m_explicit = gi.matched(&path, is_dir).map(IgnoreMatch::gitignore);
        }
        let m_global = if any_git { self.0.git_global_matcher.matched(&path, is_dir).map(IgnoreMatch::gitignore) } else { Match::None };

        let order = [&m_explicit, &m_custom_ignore, &m_ignore, &m_gi, &m_gi_exclude, &m_global];

        for check in order.into_iter() {
            if check.is_none() {
                continue;
            }

            return check.clone();
        }

        m_explicit
    }

    /// Returns an iterator over parent ignore matchers, including this one.
    pub(crate) fn parents(&self) -> Parents<'_> {
        Parents(Some(self))
    }

    /// Returns the first absolute path of the first absolute parent, if
    /// one exists.
    fn absolute_base(&self) -> Option<&Path> {
        self.0.absolute_base.as_ref().map(|p| &***p)
    }
}

/// An iterator over all parents of an ignore matcher, including itself.
///
/// The lifetime `'a` refers to the lifetime of the initial `Ignore` matcher.
pub(crate) struct Parents<'a>(Option<&'a Ignore>);

impl<'a> Iterator for Parents<'a> {
    type Item = &'a Ignore;

    fn next(&mut self) -> Option<&'a Ignore> {
        match self.0.take() {
            None => None,
            Some(ig) => {
                self.0 = ig.0.parent.as_ref();
                Some(ig)
            }
        }
    }
}

/// A builder for creating an Ignore matcher.
#[derive(Clone, Debug)]
pub(crate) struct IgnoreBuilder {
    /// The root directory path for this ignore matcher.
    dir: PathBuf,
    /// An override matcher (default is empty).
    overrides: Arc<Override>,
    /// A type matcher (default is empty).
    types: Arc<Types>,
    /// Explicit global ignore matchers.
    explicit_ignores: Vec<Gitignore>,
    /// Ignore files in addition to .ignore.
    custom_ignore_filenames: Vec<OsString>,
    /// The directory that gitignores should be interpreted relative to.
    ///
    /// Usually this is the directory containing the gitignore file. But in
    /// some cases, like for global gitignores or for gitignores specified
    /// explicitly, this should generally be set to the current working
    /// directory. This is only used for global gitignores or "explicit"
    /// gitignores.
    ///
    /// When `None`, global gitignores are ignored.
    global_gitignores_relative_to: Option<PathBuf>,
    /// Ignore config.
    opts: IgnoreOptions,
}

impl IgnoreBuilder {
    /// Create a new builder for an `Ignore` matcher.
    ///
    /// It is likely a bug to use this without also calling `current_dir()`
    /// outside of tests. This isn't made mandatory because this is an internal
    /// abstraction and it's annoying to update tests.
    pub(crate) fn new() -> IgnoreBuilder {
        IgnoreBuilder {
            dir: Path::new("").to_path_buf(),
            overrides: Arc::new(Override::empty()),
            types: Arc::new(Types::empty()),
            explicit_ignores: vec![],
            custom_ignore_filenames: vec![],
            global_gitignores_relative_to: None,
            opts: IgnoreOptions {
                hidden: true,
                ignore: true,
                parents: true,
                git_global: true,
                git_ignore: true,
                git_exclude: true,
                ignore_case_insensitive: false,
                require_git: true,
            },
        }
    }

    /// Builds a new `Ignore` matcher.
    ///
    /// The matcher returned won't match anything until ignore rules from
    /// directories are added to it.
    pub(crate) fn build(&self) -> Ignore {
        self.build_with_cwd(None)
    }

    /// Builds a new `Ignore` matcher using the given CWD directory.
    ///
    /// The matcher returned won't match anything until ignore rules from
    /// directories are added to it.
    pub(crate) fn build_with_cwd(&self, cwd: Option<PathBuf>) -> Ignore {
        let global_gitignores_relative_to = cwd.or_else(|| self.global_gitignores_relative_to.clone());
        let git_global_matcher = if !self.opts.git_global {
            Gitignore::empty()
        } else if let Some(ref cwd) = global_gitignores_relative_to {
            let mut builder = GitignoreBuilder::new(cwd);
            builder.case_insensitive(self.opts.ignore_case_insensitive).unwrap();
            let (gi, err) = builder.build_global();
            if let Some(err) = err {
                log::debug!("{}", err);
            }
            gi
        } else {
            log::debug!("ignoring global gitignore file because CWD is not known");
            Gitignore::empty()
        };

        Ignore(Arc::new(IgnoreInner {
            compiled: Arc::new(RwLock::new(HashMap::new())),
            dir: self.dir.clone(),
            overrides: self.overrides.clone(),
            types: self.types.clone(),
            parent: None,
            is_absolute_parent: true,
            absolute_base: None,
            global_gitignores_relative_to,
            explicit_ignores: Arc::new(self.explicit_ignores.clone()),
            custom_ignore_filenames: Arc::new(self.custom_ignore_filenames.clone()),
            custom_ignore_matcher: Gitignore::empty(),
            ignore_matcher: Gitignore::empty(),
            git_global_matcher: Arc::new(git_global_matcher),
            git_ignore_matcher: Gitignore::empty(),
            git_exclude_matcher: Gitignore::empty(),
            has_git: false,
            opts: self.opts,
        }))
    }

    /// Set the current directory used for matching global gitignores.
    pub(crate) fn current_dir(&mut self, cwd: impl Into<PathBuf>) -> &mut IgnoreBuilder {
        self.global_gitignores_relative_to = Some(cwd.into());
        self
    }

    /// Add an override matcher.
    ///
    /// By default, no override matcher is used.
    ///
    /// This overrides any previous setting.
    pub(crate) fn overrides(&mut self, overrides: Override) -> &mut IgnoreBuilder {
        self.overrides = Arc::new(overrides);
        self
    }

    /// Add a file type matcher.
    ///
    /// By default, no file type matcher is used.
    ///
    /// This overrides any previous setting.
    pub(crate) fn types(&mut self, types: Types) -> &mut IgnoreBuilder {
        self.types = Arc::new(types);
        self
    }

    /// Adds a new global ignore matcher from the ignore file path given.
    pub(crate) fn add_ignore(&mut self, ig: Gitignore) -> &mut IgnoreBuilder {
        self.explicit_ignores.push(ig);
        self
    }

    /// Add a custom ignore file name
    ///
    /// These ignore files have higher precedence than all other ignore files.
    ///
    /// When specifying multiple names, earlier names have lower precedence than
    /// later names.
    pub(crate) fn add_custom_ignore_filename<S: AsRef<OsStr>>(&mut self, file_name: S) -> &mut IgnoreBuilder {
        self.custom_ignore_filenames.push(file_name.as_ref().to_os_string());
        self
    }

    /// Enables ignoring hidden files.
    ///
    /// This is enabled by default.
    pub(crate) fn hidden(&mut self, yes: bool) -> &mut IgnoreBuilder {
        self.opts.hidden = yes;
        self
    }

    /// Enables reading `.ignore` files.
    ///
    /// `.ignore` files have the same semantics as `gitignore` files and are
    /// supported by search tools such as ripgrep and The Silver Searcher.
    ///
    /// This is enabled by default.
    pub(crate) fn ignore(&mut self, yes: bool) -> &mut IgnoreBuilder {
        self.opts.ignore = yes;
        self
    }

    /// Enables reading ignore files from parent directories.
    ///
    /// If this is enabled, then .gitignore files in parent directories of each
    /// file path given are respected. Otherwise, they are ignored.
    ///
    /// This is enabled by default.
    pub(crate) fn parents(&mut self, yes: bool) -> &mut IgnoreBuilder {
        self.opts.parents = yes;
        self
    }

    /// Add a global gitignore matcher.
    ///
    /// Its precedence is lower than both normal `.gitignore` files and
    /// `.git/info/exclude` files.
    ///
    /// This overwrites any previous global gitignore setting.
    ///
    /// This is enabled by default.
    pub(crate) fn git_global(&mut self, yes: bool) -> &mut IgnoreBuilder {
        self.opts.git_global = yes;
        self
    }

    /// Enables reading `.gitignore` files.
    ///
    /// `.gitignore` files have match semantics as described in the `gitignore`
    /// man page.
    ///
    /// This is enabled by default.
    pub(crate) fn git_ignore(&mut self, yes: bool) -> &mut IgnoreBuilder {
        self.opts.git_ignore = yes;
        self
    }

    /// Enables reading `.git/info/exclude` files.
    ///
    /// `.git/info/exclude` files have match semantics as described in the
    /// `gitignore` man page.
    ///
    /// This is enabled by default.
    pub(crate) fn git_exclude(&mut self, yes: bool) -> &mut IgnoreBuilder {
        self.opts.git_exclude = yes;
        self
    }

    /// Whether a git repository is required to apply git-related ignore
    /// rules (global rules, .gitignore and local exclude rules).
    ///
    /// When disabled, git-related ignore rules are applied even when searching
    /// outside a git repository.
    pub(crate) fn require_git(&mut self, yes: bool) -> &mut IgnoreBuilder {
        self.opts.require_git = yes;
        self
    }

    /// Process ignore files case insensitively
    ///
    /// This is disabled by default.
    pub(crate) fn ignore_case_insensitive(&mut self, yes: bool) -> &mut IgnoreBuilder {
        self.opts.ignore_case_insensitive = yes;
        self
    }
}

/// Creates a new gitignore matcher for the directory given.
///
/// The matcher is meant to match files below `dir`.
/// Ignore globs are extracted from each of the file names relative to
/// `dir_for_ignorefile` in the order given (earlier names have lower
/// precedence than later names).
///
/// I/O errors are ignored.
pub(crate) fn create_gitignore<T: AsRef<OsStr>>(
    dir: &Path,
    dir_for_ignorefile: &Path,
    names: &[T],
    case_insensitive: bool,
) -> (Gitignore, Option<Error>) {
    let mut builder = GitignoreBuilder::new(dir);
    let mut errs = PartialErrorBuilder::default();
    builder.case_insensitive(case_insensitive).unwrap();
    for name in names {
        let gipath = dir_for_ignorefile.join(name.as_ref());
        if cfg!(windows) || gipath.exists() {
            errs.maybe_push_ignore_io(builder.add(gipath));
        }
    }
    let gi = match builder.build() {
        Ok(gi) => gi,
        Err(err) => {
            errs.push(err);
            GitignoreBuilder::new(dir).build().unwrap()
        }
    };
    (gi, errs.into_error_option())
}

/// Find the GIT_COMMON_DIR for the given git worktree.
///
/// This is the directory that may contain a private ignore file
/// "info/exclude". Unlike git, this function does *not* read environment
/// variables GIT_DIR and GIT_COMMON_DIR, because it is not clear how to use
/// them when multiple repositories are searched.
///
/// Some I/O errors are ignored.
fn resolve_git_commondir(dir: &Path, git_type: Option<FileType>) -> Result<PathBuf, Option<Error>> {
    let git_dir_path = || dir.join(".git");
    let git_dir = git_dir_path();
    if !git_type.map_or(false, |ft| ft.is_file()) {
        return Ok(git_dir);
    }
    let file = match File::open(git_dir) {
        Ok(file) => io::BufReader::new(file),
        Err(err) => {
            return Err(Some(Error::Io(err).with_path(git_dir_path())));
        }
    };
    let dot_git_line = match file.lines().next() {
        Some(Ok(line)) => line,
        Some(Err(err)) => {
            return Err(Some(Error::Io(err).with_path(git_dir_path())));
        }
        None => return Err(None),
    };
    if !dot_git_line.starts_with("gitdir: ") {
        return Err(None);
    }
    let real_git_dir = PathBuf::from(&dot_git_line["gitdir: ".len()..]);
    let git_commondir_file = || real_git_dir.join("commondir");
    let file = match File::open(git_commondir_file()) {
        Ok(file) => io::BufReader::new(file),
        Err(_) => return Err(None),
    };
    let commondir_line = match file.lines().next() {
        Some(Ok(line)) => line,
        Some(Err(err)) => {
            return Err(Some(Error::Io(err).with_path(git_commondir_file())));
        }
        None => return Err(None),
    };
    let commondir_abs = if commondir_line.starts_with(".") { real_git_dir.join(commondir_line) } else { PathBuf::from(commondir_line) };
    Ok(commondir_abs)
}
