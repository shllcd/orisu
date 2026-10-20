use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use orisu_diagnostics::CATALOG;

/// Writes every generated file, or with `--check` fails when one is stale.
pub fn run(args: &[String]) -> Result<(), String> {
    let check = args.iter().any(|arg| arg == "--check");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace root");
    let outputs = [
        (root.join("packages/orisu/skills/orisu-diagnostics/SKILL.md"), orisu_diagnostics::render_skill()),
        (root.join("packages/orisu/src/codes.ts"), codes()),
    ];
    let mut stale: Vec<&PathBuf> = Vec::new();
    for (path, content) in &outputs {
        if fs::read_to_string(path).ok().as_deref() == Some(content.as_str()) {
            continue;
        }
        if check {
            stale.push(path);
            continue;
        }
        fs::create_dir_all(path.parent().expect("outputs live in directories")).map_err(|e| e.to_string())?;
        fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", path.strip_prefix(root).unwrap_or(path).display());
    }
    if stale.is_empty() {
        return Ok(());
    }
    let list: Vec<String> = stale.iter().map(|path| path.strip_prefix(root).unwrap_or(path).display().to_string()).collect();
    Err(format!("stale, run `cargo xtask codegen`: {}", list.join(", ")))
}

fn codes() -> String {
    let mut out =
        String::from("/** Every orisu diagnostic code, from the catalog in `orisu_diagnostics`. */\nexport type DiagnosticCode =\n");
    for entry in CATALOG {
        let _ = writeln!(out, "  | {:?}", entry.name);
    }
    out.pop();
    out.push_str(";\n");
    out
}
