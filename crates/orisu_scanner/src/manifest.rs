use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::scanner::sources::PublicSourceEntry;

/// The candidate manifest that replaces scanning `source`: an installed package (below `node_modules`) containing the
/// source declares one with `"orisu": { "manifest": "./orisu.json" }` in its `package.json`.
pub fn for_source(source: &PublicSourceEntry) -> Option<PathBuf> {
    if source.negated {
        return None;
    }
    let mut root = source.clone();
    root.optimize();
    let root = Path::new(&root.base);
    for dir in root.ancestors() {
        let package = dir.join("package.json");
        if !package.is_file() {
            continue;
        }
        if !dir.components().any(|component| component.as_os_str() == "node_modules") {
            return None;
        }
        let manifest: Value = serde_json::from_slice(&fs::read(&package).ok()?).ok()?;
        return Some(dir.join(manifest.get("orisu")?.get("manifest")?.as_str()?));
    }
    None
}

/// The `candidates` array of a manifest file; empty when it is missing or malformed.
pub fn read(path: &Path) -> Vec<String> {
    let Some(manifest) = fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()) else {
        return Vec::new();
    };
    let Some(candidates) = manifest.get("candidates").and_then(Value::as_array) else {
        return Vec::new();
    };
    candidates.iter().filter_map(Value::as_str).map(str::to_owned).collect()
}
