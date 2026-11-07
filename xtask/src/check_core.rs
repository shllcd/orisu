use std::fs;
use std::path::Path;

use crate::harness::root;

const ADAPTERS: &[&str] = &["orisu_compat_tailwind", "orisu_lsp", "orisu_napi"];

const FORBIDDEN: &[&str] = &["RefCell", "OnceCell", "Cell<", "Cell::", "Rc<", "Rc::", "std::rc", "std::cell"];

/// Fails when a core crate uses `RefCell`, `Cell`, `OnceCell` or `Rc`.
pub fn run() -> Result<(), String> {
    let root = root();
    let mut files = Vec::new();
    for entry in fs::read_dir(root.join("crates")).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !ADAPTERS.iter().any(|name| path.ends_with(name)) {
            collect(&path.join("src"), &mut files)?;
        }
    }
    files.sort();

    let mut errors = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let found = source.lines().filter(|line| FORBIDDEN.iter().any(|pattern| line.contains(pattern))).count();
        let relative = file.strip_prefix(root).unwrap_or(file).to_string_lossy().replace('\\', "/");
        if found > 0 {
            errors.push(format!("{relative}: {found} lines use interior mutability or Rc"));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}

fn collect(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    Ok(())
}
