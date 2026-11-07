use orisu_ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::ffi::OsStr;
use std::sync;

/// The entries of a fixture list, one per line, skipping blank lines and `#` comments.
pub fn list(text: &'static str) -> impl Iterator<Item = &'static str> {
    text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#'))
}

fn joined(text: &'static str) -> String {
    list(text).collect::<Vec<_>>().join(",")
}

/// The default rules of auto source detection: the walk skips these directories, extensions and files.
pub static RULES: sync::LazyLock<Vec<Gitignore>> = sync::LazyLock::new(|| {
    let mut builder = GitignoreBuilder::new("");
    builder.add_line(None, &format!("{{{}}}/", IGNORED_CONTENT_DIRS.join(","))).unwrap();
    builder.add_line(None, &format!("*.{{{}}}", joined(include_str!("fixtures/ignored-extensions.txt")))).unwrap();
    builder.add_line(None, &format!("{{{}}}", joined(include_str!("fixtures/ignored-files.txt")))).unwrap();

    let mut file_only_builder = GitignoreBuilder::new("");
    file_only_builder.only_on_files(true).add_line(None, &BINARY_EXTENSIONS_GLOB).unwrap();

    vec![builder.build().unwrap(), file_only_builder.build().unwrap()]
});

static IGNORED_CONTENT_DIRS: sync::LazyLock<Vec<&'static str>> =
    sync::LazyLock::new(|| list(include_str!("fixtures/ignored-content-dirs.txt")).collect());

/// Whether a directory named `name` holds dependencies, build output or version control data rather than sources.
pub fn is_ignored_content_dir(name: &OsStr) -> bool {
    IGNORED_CONTENT_DIRS.iter().any(|dir| name == *dir)
}

pub static BINARY_EXTENSIONS_GLOB: sync::LazyLock<String> =
    sync::LazyLock::new(|| format!("*.{{{}}}", joined(include_str!("fixtures/binary-extensions.txt"))));
