/*!
The gitignore module provides a way to match globs from a gitignore file
against file paths.

Note that this module implements the specification as described in the
`gitignore` man page from scratch. That is, this module does *not* shell out to
the `git` command line tool.
*/

use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

use {
    globset::{Candidate, GlobBuilder, GlobSet, GlobSetBuilder},
    regex_automata::util::pool::Pool,
};

use crate::{
    Error, Match, PartialErrorBuilder,
    pathutil::{is_file_name, strip_prefix},
};

/// Glob represents a single glob in a gitignore file.
///
/// This is used to report information about the highest precedent glob that
/// matched in one or more gitignore files.
#[derive(Clone, Debug)]
pub struct Glob {
    /// The file path that this glob was extracted from.
    from: Option<PathBuf>,
    /// The original glob string.
    original: String,
    /// The actual glob string used to convert to a regex.
    actual: String,
    /// Whether this is a whitelisted glob or not.
    is_whitelist: bool,
    /// Whether this glob should only match directories or not.
    is_only_dir: bool,
}

impl Glob {
    /// Returns the file path that defined this glob.
    pub fn from(&self) -> Option<&Path> {
        self.from.as_ref().map(|p| &**p)
    }

    /// The original glob as it was defined in a gitignore file.
    pub fn original(&self) -> &str {
        &self.original
    }

    /// The actual glob that was compiled to respect gitignore
    /// semantics.
    pub fn actual(&self) -> &str {
        &self.actual
    }

    /// Whether this was a whitelisted glob or not.
    pub fn is_whitelist(&self) -> bool {
        self.is_whitelist
    }

    /// Whether this glob must match a directory or not.
    pub fn is_only_dir(&self) -> bool {
        self.is_only_dir
    }

    /// Returns true if and only if this glob has a `**/` prefix.
    fn has_doublestar_prefix(&self) -> bool {
        self.actual.starts_with("**/") || self.actual == "**"
    }
}

/// Gitignore is a matcher for the globs in one or more gitignore files
/// in the same directory.
#[derive(Clone, Debug)]
pub struct Gitignore {
    set: GlobSet,
    root: PathBuf,
    globs: Vec<Glob>,
    num_ignores: u64,
    num_whitelists: u64,
    matches: Option<Arc<Pool<Vec<usize>>>>,
    only_on_files: bool,
}

impl Gitignore {
    /// Creates a new gitignore matcher from the gitignore file path given.
    ///
    /// If it's desirable to include multiple gitignore files in a single
    /// matcher, or read gitignore globs from a different source, then
    /// use `GitignoreBuilder`.
    ///
    /// This always returns a valid matcher, even if it's empty. In particular,
    /// a Gitignore file can be partially valid, e.g., when one glob is invalid
    /// but the rest aren't.
    ///
    /// Note that I/O errors are ignored. For more granular control over
    /// errors, use `GitignoreBuilder`.
    pub fn new<P: AsRef<Path>>(gitignore_path: P) -> (Gitignore, Option<Error>) {
        let path = gitignore_path.as_ref();
        let parent = path.parent().unwrap_or(Path::new("/"));
        let mut builder = GitignoreBuilder::new(parent);
        let mut errs = PartialErrorBuilder::default();
        errs.maybe_push_ignore_io(builder.add(path));
        match builder.build() {
            Ok(gi) => (gi, errs.into_error_option()),
            Err(err) => {
                errs.push(err);
                (Gitignore::empty(), errs.into_error_option())
            }
        }
    }

    /// Creates a new gitignore matcher from the global ignore file, if one
    /// exists.
    ///
    /// The global config file path is specified by git's `core.excludesFile`
    /// config option.
    ///
    /// Git's config file location is `$HOME/.gitconfig`. If `$HOME/.gitconfig`
    /// does not exist or does not specify `core.excludesFile`, then
    /// `$XDG_CONFIG_HOME/git/ignore` is read. If `$XDG_CONFIG_HOME` is not
    /// set or is empty, then `$HOME/.config/git/ignore` is used instead.
    pub fn global() -> (Gitignore, Option<Error>) {
        match std::env::current_dir() {
            Ok(cwd) => GitignoreBuilder::new(cwd).build_global(),
            Err(err) => (Gitignore::empty(), Some(err.into())),
        }
    }

    /// Creates a new empty gitignore matcher that never matches anything.
    ///
    /// Its path is empty.
    pub fn empty() -> Gitignore {
        Gitignore {
            set: GlobSet::empty(),
            root: PathBuf::from(""),
            globs: vec![],
            num_ignores: 0,
            num_whitelists: 0,
            matches: None,
            only_on_files: false,
        }
    }

    /// Returns the directory containing this gitignore matcher.
    ///
    /// All matches are done relative to this path.
    pub fn path(&self) -> &Path {
        &*self.root
    }

    /// Returns true if and only if this gitignore has zero globs, and
    /// therefore never matches any file path.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Returns the total number of globs, which should be equivalent to
    /// `num_ignores + num_whitelists`.
    pub fn len(&self) -> usize {
        self.set.len()
    }

    /// Returns the total number of ignore globs.
    pub fn num_ignores(&self) -> u64 {
        self.num_ignores
    }

    /// Returns the total number of whitelisted globs.
    pub fn num_whitelists(&self) -> u64 {
        self.num_whitelists
    }

    /// Returns whether the given path (file or directory) matched a pattern in
    /// this gitignore matcher.
    ///
    /// `is_dir` should be true if the path refers to a directory and false
    /// otherwise.
    ///
    /// The given path is matched relative to the path given when building
    /// the matcher. Specifically, before matching `path`, its prefix (as
    /// determined by a common suffix of the directory containing this
    /// gitignore) is stripped. If there is no common suffix/prefix overlap,
    /// then `path` is assumed to be relative to this matcher.
    pub fn matched<P: AsRef<Path>>(&self, path: P, is_dir: bool) -> Match<&Glob> {
        if self.is_empty() {
            return Match::None;
        }
        self.matched_stripped(self.strip(path.as_ref()), is_dir)
    }

    /// Returns whether the given path (file or directory, and expected to be
    /// under the root) or any of its parent directories (up to the root)
    /// matched a pattern in this gitignore matcher.
    ///
    /// NOTE: This method is more expensive than walking the directory hierarchy
    /// top-to-bottom and matching the entries. But, is easier to use in cases
    /// when a list of paths are available without a hierarchy.
    ///
    /// `is_dir` should be true if the path refers to a directory and false
    /// otherwise.
    ///
    /// The given path is matched relative to the path given when building
    /// the matcher. Specifically, before matching `path`, its prefix (as
    /// determined by a common suffix of the directory containing this
    /// gitignore) is stripped. If there is no common suffix/prefix overlap,
    /// then `path` is assumed to be relative to this matcher.
    ///
    /// # Panics
    ///
    /// This method panics if the given file path is not under the root path
    /// of this matcher.
    pub fn matched_path_or_any_parents<P: AsRef<Path>>(&self, path: P, is_dir: bool) -> Match<&Glob> {
        if self.is_empty() {
            return Match::None;
        }
        let mut path = self.strip(path.as_ref());
        assert!(!path.has_root(), "path is expected to be under the root");

        match self.matched_stripped(path, is_dir) {
            Match::None => (),
            a_match => return a_match,
        }
        while let Some(parent) = path.parent() {
            match self.matched_stripped(parent, true) {
                Match::None => path = parent,
                a_match => return a_match,
            }
        }
        Match::None
    }

    /// Like matched, but takes a path that has already been stripped.
    fn matched_stripped<P: AsRef<Path>>(&self, path: P, is_dir: bool) -> Match<&Glob> {
        if self.is_empty() {
            return Match::None;
        }
        if self.only_on_files && is_dir {
            return Match::None;
        }
        let path = path.as_ref();
        let mut matches = self.matches.as_ref().unwrap().get();
        let candidate = Candidate::new(path);
        self.set.matches_candidate_into(&candidate, &mut *matches);
        for &i in matches.iter().rev() {
            let glob = &self.globs[i];
            if !glob.is_only_dir() || is_dir {
                return if glob.is_whitelist() { Match::Whitelist(glob) } else { Match::Ignore(glob) };
            }
        }
        Match::None
    }

    /// Strips the given path such that it's suitable for matching with this
    /// gitignore matcher.
    fn strip<'a, P: 'a + AsRef<Path> + ?Sized>(&'a self, path: &'a P) -> &'a Path {
        let mut path = path.as_ref();
        if let Some(p) = strip_prefix("./", path) {
            path = p;
        }
        if self.root != Path::new(".") && !is_file_name(path) {
            if let Some(p) = strip_prefix(&self.root, path) {
                path = p;
                if let Some(p) = strip_prefix("/", path) {
                    path = p;
                }
            }
        }
        path
    }
}

/// Builds a matcher for a single set of globs from a .gitignore file.
#[derive(Clone, Debug)]
pub struct GitignoreBuilder {
    builder: GlobSetBuilder,
    root: PathBuf,
    globs: Vec<Glob>,
    case_insensitive: bool,
    allow_unclosed_class: bool,
    only_on_files: bool,
}

impl GitignoreBuilder {
    /// Create a new builder for a gitignore file.
    ///
    /// The path given should be the path at which the globs for this gitignore
    /// file should be matched. Note that paths are always matched relative
    /// to the root path given here. Generally, the root path should correspond
    /// to the *directory* containing a `.gitignore` file.
    pub fn new<P: AsRef<Path>>(root: P) -> GitignoreBuilder {
        let root = root.as_ref();
        GitignoreBuilder {
            builder: GlobSetBuilder::new(),
            root: strip_prefix("./", root).unwrap_or(root).to_path_buf(),
            globs: vec![],
            case_insensitive: false,
            allow_unclosed_class: true,
            only_on_files: false,
        }
    }

    /// Builds a new matcher from the globs added so far.
    ///
    /// Once a matcher is built, no new globs can be added to it.
    pub fn build(&self) -> Result<Gitignore, Error> {
        let nignore = self.globs.iter().filter(|g| !g.is_whitelist()).count();
        let nwhite = self.globs.iter().filter(|g| g.is_whitelist()).count();
        let set = self.builder.build().map_err(|err| Error::Glob { glob: None, err: err.to_string() })?;
        Ok(Gitignore {
            set,
            root: self.root.clone(),
            globs: self.globs.clone(),
            num_ignores: nignore as u64,
            num_whitelists: nwhite as u64,
            matches: Some(Arc::new(Pool::new(|| vec![]))),
            only_on_files: self.only_on_files,
        })
    }

    /// Build a global gitignore matcher using the configuration in this
    /// builder.
    ///
    /// This consumes ownership of the builder unlike `build` because it
    /// must mutate the builder to add the global gitignore globs.
    ///
    /// Note that this ignores the path given to this builder's constructor
    /// and instead derives the path automatically from git's global
    /// configuration.
    pub fn build_global(mut self) -> (Gitignore, Option<Error>) {
        match gitconfig_excludes_path() {
            None => (Gitignore::empty(), None),
            Some(path) => {
                if !path.is_file() {
                    (Gitignore::empty(), None)
                } else {
                    let mut errs = PartialErrorBuilder::default();
                    errs.maybe_push_ignore_io(self.add(path));
                    match self.build() {
                        Ok(gi) => (gi, errs.into_error_option()),
                        Err(err) => {
                            errs.push(err);
                            (Gitignore::empty(), errs.into_error_option())
                        }
                    }
                }
            }
        }
    }

    /// Add each glob from the file path given.
    ///
    /// The file given should be formatted as a `gitignore` file.
    ///
    /// Note that partial errors can be returned. For example, if there was
    /// a problem adding one glob, an error for that will be returned, but
    /// all other valid globs will still be added.
    pub fn add<P: AsRef<Path>>(&mut self, path: P) -> Option<Error> {
        let path = path.as_ref();
        let file = match File::open(path) {
            Err(err) => return Some(Error::Io(err).with_path(path)),
            Ok(file) => file,
        };
        log::debug!("opened gitignore file: {}", path.display());
        let rdr = BufReader::new(file);
        let mut errs = PartialErrorBuilder::default();
        for (i, line) in rdr.lines().enumerate() {
            let lineno = (i + 1) as u64;
            let line = match line {
                Ok(line) => line,
                Err(err) => {
                    errs.push(Error::Io(err).tagged(path, lineno));
                    break;
                }
            };

            const UTF8_BOM: &str = "\u{feff}";
            let line = if i == 0 { line.trim_start_matches(UTF8_BOM) } else { &line };

            if let Err(err) = self.add_line(Some(path.to_path_buf()), &line) {
                errs.push(err.tagged(path, lineno));
            }
        }
        errs.into_error_option()
    }

    /// Add each glob line from the string given.
    ///
    /// If this string came from a particular `gitignore` file, then its path
    /// should be provided here.
    ///
    /// The string given should be formatted as a `gitignore` file.

    /// Add a line from a gitignore file to this builder.
    ///
    /// If this line came from a particular `gitignore` file, then its path
    /// should be provided here.
    ///
    /// If the line could not be parsed as a glob, then an error is returned.
    pub fn add_line(&mut self, from: Option<PathBuf>, mut line: &str) -> Result<&mut GitignoreBuilder, Error> {
        #![allow(deprecated)]

        if line.starts_with("#") {
            return Ok(self);
        }
        if !line.ends_with("\\ ") {
            line = line.trim_right();
        }
        if line.is_empty() {
            return Ok(self);
        }
        let mut glob = Glob { from, original: line.to_string(), actual: String::new(), is_whitelist: false, is_only_dir: false };
        let mut is_absolute = false;
        if line.starts_with("\\!") || line.starts_with("\\#") {
            line = &line[1..];
            is_absolute = line.chars().nth(0) == Some('/');
        } else {
            if line.starts_with("!") {
                glob.is_whitelist = true;
                line = &line[1..];
            }
            if line.starts_with("/") {
                line = &line[1..];
                is_absolute = true;
            }
        }
        if line.as_bytes().last() == Some(&b'/') {
            glob.is_only_dir = true;
            line = &line[..line.len() - 1];
            if line.as_bytes().last() == Some(&b'\\') {
                line = &line[..line.len() - 1];
            }
        }
        glob.actual = line.to_string();
        if !is_absolute && !line.chars().any(|c| c == '/') {
            if !glob.has_doublestar_prefix() {
                glob.actual = format!("**/{}", glob.actual);
            }
        }
        if glob.actual.ends_with("/**") {
            glob.actual = format!("{}/*", glob.actual);
        }
        let parsed = GlobBuilder::new(&glob.actual)
            .literal_separator(true)
            .case_insensitive(self.case_insensitive)
            .backslash_escape(true)
            .allow_unclosed_class(self.allow_unclosed_class)
            .build()
            .map_err(|err| Error::Glob { glob: Some(glob.original.clone()), err: err.kind().to_string() })?;
        self.builder.add(parsed);
        self.globs.push(glob);
        Ok(self)
    }

    /// Toggle whether the globs should be matched case insensitively or not.
    ///
    /// When this option is changed, only globs added after the change will be
    /// affected.
    ///
    /// This is disabled by default.
    pub fn case_insensitive(&mut self, yes: bool) -> Result<&mut GitignoreBuilder, Error> {
        self.case_insensitive = yes;
        Ok(self)
    }

    /// Toggle whether unclosed character classes are allowed. When allowed,
    /// a `[` without a matching `]` is treated literally instead of resulting
    /// in a parse error.
    ///
    /// For example, if this is set then the glob `[abc` will be treated as the
    /// literal string `[abc` instead of returning an error.
    ///
    /// By default, this is true in order to match established `gitignore`
    /// semantics. Generally speaking, enabling this leads to worse failure
    /// modes since the glob parser becomes more permissive. You might want to
    /// enable this when compatibility (e.g., with POSIX glob implementations)
    /// is more important than good error messages.
    pub fn allow_unclosed_class(&mut self, yes: bool) -> &mut GitignoreBuilder {
        self.allow_unclosed_class = yes;
        self
    }

    /// CHANGED: Add a flag to have Gitignore rules that apply only to files.
    ///
    /// If this is set, then the globs will only be matched against file paths.
    /// This will ensure that ignore rules like `*.pages` will _only_ ignore
    /// files ending in `.pages` and not folders ending in `.pages`.
    pub fn only_on_files(&mut self, yes: bool) -> &mut GitignoreBuilder {
        self.only_on_files = yes;
        self
    }
}

/// Return the file path of the current environment's global gitignore file.
///
/// Note that the file path returned may not exist.
pub fn gitconfig_excludes_path() -> Option<PathBuf> {
    match gitconfig_home_contents().and_then(|x| parse_excludes_file(&x)) {
        Some(path) => return Some(path),
        None => {}
    }
    match gitconfig_xdg_contents().and_then(|x| parse_excludes_file(&x)) {
        Some(path) => return Some(path),
        None => {}
    }
    excludes_file_default()
}

/// Returns the file contents of git's global config file, if one exists, in
/// the user's home directory.
fn gitconfig_home_contents() -> Option<Vec<u8>> {
    let home = match home_dir() {
        None => return None,
        Some(home) => home,
    };
    let mut file = match File::open(home.join(".gitconfig")) {
        Err(_) => return None,
        Ok(file) => BufReader::new(file),
    };
    let mut contents = vec![];
    file.read_to_end(&mut contents).ok().map(|_| contents)
}

/// Returns the file contents of git's global config file, if one exists, in
/// the user's XDG_CONFIG_HOME directory.
fn gitconfig_xdg_contents() -> Option<Vec<u8>> {
    let path = std::env::var_os("XDG_CONFIG_HOME")
        .and_then(|x| if x.is_empty() { None } else { Some(PathBuf::from(x)) })
        .or_else(|| home_dir().map(|p| p.join(".config")))
        .map(|x| x.join("git/config"));
    let mut file = match path.and_then(|p| File::open(p).ok()) {
        None => return None,
        Some(file) => BufReader::new(file),
    };
    let mut contents = vec![];
    file.read_to_end(&mut contents).ok().map(|_| contents)
}

/// Returns the default file path for a global .gitignore file.
///
/// Specifically, this respects XDG_CONFIG_HOME.
fn excludes_file_default() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .and_then(|x| if x.is_empty() { None } else { Some(PathBuf::from(x)) })
        .or_else(|| home_dir().map(|p| p.join(".config")))
        .map(|x| x.join("git/ignore"))
}

/// Extract git's `core.excludesfile` config setting from the raw file contents
/// given.
fn parse_excludes_file(data: &[u8]) -> Option<PathBuf> {
    use std::sync::OnceLock;

    use regex_automata::{meta::Regex, util::syntax};

    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::builder()
            .configure(Regex::config().utf8_empty(false))
            .syntax(syntax::Config::new().utf8(false))
            .build(r#"(?im-u)^\s*excludesfile\s*=\s*"?\s*(\S+?)\s*"?\s*$"#)
            .unwrap()
    });
    let mut caps = re.create_captures();
    re.captures(data, &mut caps);
    let span = caps.get_group(1)?;
    let candidate = &data[span];
    std::str::from_utf8(candidate).ok().map(|s| PathBuf::from(expand_tilde(s)))
}

/// Expands ~ in file paths to the value of $HOME.
fn expand_tilde(path: &str) -> String {
    let home = match home_dir() {
        None => return path.to_string(),
        Some(home) => home.to_string_lossy().into_owned(),
    };
    path.replace("~", &home)
}

/// Returns the location of the user's home directory.
fn home_dir() -> Option<PathBuf> {
    #![allow(deprecated)]
    std::env::home_dir()
}
