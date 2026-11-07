use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};

/// The sources extraction output depends on; their hash keys the scanner cache.
const SOURCES: [&str; 2] = ["src", "../orisu_candidate/src"];

fn main() {
    let mut files = Vec::new();
    for dir in SOURCES {
        println!("cargo:rerun-if-changed={dir}");
        collect(Path::new(dir), &mut files);
    }
    files.sort();
    let mut hasher = DefaultHasher::new();
    for file in files {
        hasher.write(file.to_string_lossy().as_bytes());
        hasher.write(&std::fs::read(&file).unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display())));
    }
    println!("cargo:rustc-env=ORISU_SCANNER_SOURCE_HASH={:016x}", hasher.finish());
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()));
    for entry in entries {
        let path = entry.unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display())).path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}
