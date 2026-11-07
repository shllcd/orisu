/*!
A parallel recursive directory walker that respects `.gitignore`, `.ignore`
and hidden-file rules, vendored from ripgrep's `ignore` crate and trimmed to
what the scanner uses. Matching rules and precedence are documented on
`WalkBuilder`.
*/

#![allow(clippy::all, reason = "vendored upstream code, kept close to ripgrep for easy syncing")]
#![deny(missing_docs)]

use std::path::{Path, PathBuf};

pub use crate::walk::{DirEntry, WalkBuilder, WalkState};

mod dir;
pub mod gitignore;
mod pathutil;
pub use crate::dir::git_exclude_files;
pub use crate::pathutil::{is_normal_path, path_starts_with};
mod walk;

/// Represents an error that can occur when parsing a gitignore file.
#[derive(Debug)]
pub enum Error {
    /// A collection of "soft" errors. These occur when adding an ignore
    /// file partially succeeded.
    Partial(Vec<Error>),
    /// An error associated with a specific line number.
    WithLineNumber {
        /// The line number.
        line: u64,
        /// The underlying error.
        err: Box<Error>,
    },
    /// An error associated with a particular file path.
    WithPath {
        /// The file path.
        path: PathBuf,
        /// The underlying error.
        err: Box<Error>,
    },
    /// An error associated with a particular directory depth when recursively
    /// walking a directory.
    WithDepth {
        /// The directory depth.
        depth: usize,
        /// The underlying error.
        err: Box<Error>,
    },
    /// An error that occurs when a file loop is detected when traversing
    /// symbolic links.
    Loop {
        /// The ancestor file path in the loop.
        ancestor: PathBuf,
        /// The child file path in the loop.
        child: PathBuf,
    },
    /// An error that occurs when doing I/O, such as reading an ignore file.
    Io(std::io::Error),
    /// An error that occurs when trying to parse a glob.
    Glob {
        /// The original glob that caused this error. This glob, when
        /// available, always corresponds to the glob provided by an end user.
        /// e.g., It is the glob as written in a `.gitignore` file.
        ///
        /// (This glob may be distinct from the glob that is actually
        /// compiled, after accounting for `gitignore` semantics.)
        glob: Option<String>,
        /// The underlying glob error as a string.
        err: String,
    },
}

impl Clone for Error {
    fn clone(&self) -> Error {
        match *self {
            Error::Partial(ref errs) => Error::Partial(errs.clone()),
            Error::WithLineNumber { line, ref err } => Error::WithLineNumber { line, err: err.clone() },
            Error::WithPath { ref path, ref err } => Error::WithPath { path: path.clone(), err: err.clone() },
            Error::WithDepth { depth, ref err } => Error::WithDepth { depth, err: err.clone() },
            Error::Loop { ref ancestor, ref child } => Error::Loop { ancestor: ancestor.clone(), child: child.clone() },
            Error::Io(ref err) => match err.raw_os_error() {
                Some(e) => Error::Io(std::io::Error::from_raw_os_error(e)),
                None => Error::Io(std::io::Error::new(err.kind(), err.to_string())),
            },
            Error::Glob { ref glob, ref err } => Error::Glob { glob: glob.clone(), err: err.clone() },
        }
    }
}

impl Error {
    /// Returns true if this error is exclusively an I/O error.
    fn is_io(&self) -> bool {
        match *self {
            Error::Partial(ref errs) => errs.len() == 1 && errs[0].is_io(),
            Error::WithLineNumber { ref err, .. } => err.is_io(),
            Error::WithPath { ref err, .. } => err.is_io(),
            Error::WithDepth { ref err, .. } => err.is_io(),
            Error::Loop { .. } => false,
            Error::Io(_) => true,
            Error::Glob { .. } => false,
        }
    }

    /// Returns a depth associated with recursively walking a directory (if
    /// this error was generated from a recursive directory iterator).
    pub fn depth(&self) -> Option<usize> {
        match *self {
            Error::WithPath { ref err, .. } => err.depth(),
            Error::WithDepth { depth, .. } => Some(depth),
            _ => None,
        }
    }

    /// Turn an error into a tagged error with the given file path.
    fn with_path<P: AsRef<Path>>(self, path: P) -> Error {
        Error::WithPath { path: path.as_ref().to_path_buf(), err: Box::new(self) }
    }

    /// Turn an error into a tagged error with the given depth.
    fn with_depth(self, depth: usize) -> Error {
        Error::WithDepth { depth, err: Box::new(self) }
    }

    /// Turn an error into a tagged error with the given file path and line
    /// number. If path is empty, then it is omitted from the error.
    fn tagged<P: AsRef<Path>>(self, path: P, lineno: u64) -> Error {
        let errline = Error::WithLineNumber { line: lineno, err: Box::new(self) };
        if path.as_ref().as_os_str().is_empty() {
            return errline;
        }
        errline.with_path(path)
    }

    /// Build an error from a walkdir error.
    fn from_walkdir(err: walkdir::Error) -> Error {
        let depth = err.depth();
        if let (Some(anc), Some(child)) = (err.loop_ancestor(), err.path()) {
            return Error::WithDepth { depth, err: Box::new(Error::Loop { ancestor: anc.to_path_buf(), child: child.to_path_buf() }) };
        }
        let path = err.path().map(|p| p.to_path_buf());
        let mut ig_err = Error::Io(std::io::Error::from(err));
        if let Some(path) = path {
            ig_err = Error::WithPath { path, err: Box::new(ig_err) };
        }
        ig_err
    }
}

impl std::error::Error for Error {
    #[allow(deprecated)]
    fn description(&self) -> &str {
        match *self {
            Error::Partial(_) => "partial error",
            Error::WithLineNumber { ref err, .. } => err.description(),
            Error::WithPath { ref err, .. } => err.description(),
            Error::WithDepth { ref err, .. } => err.description(),
            Error::Loop { .. } => "file system loop found",
            Error::Io(ref err) => err.description(),
            Error::Glob { ref err, .. } => err,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Error::Partial(ref errs) => {
                let msgs: Vec<String> = errs.iter().map(|err| err.to_string()).collect();
                write!(f, "{}", msgs.join("\n"))
            }
            Error::WithLineNumber { line, ref err } => {
                write!(f, "line {}: {}", line, err)
            }
            Error::WithPath { ref path, ref err } => {
                write!(f, "{}: {}", path.display(), err)
            }
            Error::WithDepth { ref err, .. } => err.fmt(f),
            Error::Loop { ref ancestor, ref child } => write!(
                f,
                "File system loop found: \
                           {} points to an ancestor {}",
                child.display(),
                ancestor.display()
            ),
            Error::Io(ref err) => err.fmt(f),
            Error::Glob { glob: None, ref err } => write!(f, "{}", err),
            Error::Glob { glob: Some(ref glob), ref err } => {
                write!(f, "error parsing glob '{}': {}", glob, err)
            }
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Error {
        Error::Io(err)
    }
}

#[derive(Debug, Default)]
struct PartialErrorBuilder(Vec<Error>);

impl PartialErrorBuilder {
    fn push(&mut self, err: Error) {
        self.0.push(err);
    }

    fn push_ignore_io(&mut self, err: Error) {
        if !err.is_io() {
            self.push(err);
        }
    }

    fn maybe_push(&mut self, err: Option<Error>) {
        if let Some(err) = err {
            self.push(err);
        }
    }

    fn maybe_push_ignore_io(&mut self, err: Option<Error>) {
        if let Some(err) = err {
            self.push_ignore_io(err);
        }
    }

    fn into_error_option(mut self) -> Option<Error> {
        if self.0.is_empty() {
            None
        } else if self.0.len() == 1 {
            Some(self.0.pop().unwrap())
        } else {
            Some(Error::Partial(self.0))
        }
    }
}

/// The result of a glob match.
///
/// The type parameter `T` typically refers to a type that provides more
/// information about a particular match. For example, it might identify
/// the specific gitignore file and the specific glob pattern that caused
/// the match.
#[derive(Clone, Debug)]
pub enum Match<T> {
    /// The path didn't match any glob.
    None,
    /// The highest precedent glob matched indicates the path should be
    /// ignored.
    Ignore(T),
    /// The highest precedent glob matched indicates the path should be
    /// whitelisted.
    Whitelist(T),
}

impl<T> Match<T> {
    /// Returns true if the match result didn't match any globs.
    pub fn is_none(&self) -> bool {
        match *self {
            Match::None => true,
            Match::Ignore(_) | Match::Whitelist(_) => false,
        }
    }

    /// Returns true if the match result implies the path should be ignored.
    pub fn is_ignore(&self) -> bool {
        match *self {
            Match::Ignore(_) => true,
            Match::None | Match::Whitelist(_) => false,
        }
    }

    /// Returns true if the match result implies the path should be
    /// whitelisted.
    pub fn is_whitelist(&self) -> bool {
        match *self {
            Match::Whitelist(_) => true,
            Match::None | Match::Ignore(_) => false,
        }
    }

    /// Apply the given function to the value inside this match.
    ///
    /// If the match has no value, then return the match unchanged.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Match<U> {
        match self {
            Match::None => Match::None,
            Match::Ignore(t) => Match::Ignore(f(t)),
            Match::Whitelist(t) => Match::Whitelist(f(t)),
        }
    }
}
