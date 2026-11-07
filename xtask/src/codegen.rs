use std::fmt::Write;
use std::fs;
use std::path::PathBuf;

use orisu_compiler::compiler::features;
use orisu_diagnostics::CATALOG;
use orisu_optimizer::polyfills;

use crate::harness::root;

/// Writes every generated file, or with `--check` fails when one is stale.
pub fn run(args: &[String]) -> Result<(), String> {
    let check = args.iter().any(|arg| arg == "--check");
    let root = root();
    let outputs = [
        (root.join("packages/orisu/skills/orisu-diagnostics/SKILL.md"), orisu_diagnostics::render_skill()),
        (root.join("packages/orisu/src/codes.ts"), codes()),
        (root.join("packages/orisu/src/flags.ts"), flags()),
        (root.join("packages/orisu/src/merge-keys.ts"), merge_keys()),
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

fn flags() -> String {
    let mut out = String::new();
    flag_object(
        &mut out,
        "What a compiled stylesheet uses, from `orisu_compiler::compiler::features`.",
        "Features",
        &[
            ("None", features::NONE),
            ("AtApply", features::AT_APPLY),
            ("AtImport", features::AT_IMPORT),
            ("ThemeFunction", features::THEME_FUNCTION),
            ("Utilities", features::UTILITIES),
            ("Variants", features::VARIANTS),
            ("AtTheme", features::AT_THEME),
        ],
    );
    out.push('\n');
    flag_object(
        &mut out,
        "Polyfills the compiler emits, from `orisu_optimizer::polyfills`.",
        "Polyfills",
        &[("None", polyfills::NONE), ("AtProperty", polyfills::AT_PROPERTY), ("ColorMix", polyfills::COLOR_MIX), ("All", polyfills::ALL)],
    );
    out
}

fn merge_keys() -> String {
    let mut out = String::from(
        "/** Variants whose relative order changes the generated selector, from `orisu_merge::ORDER_SENSITIVE`. */\nexport const ORDER_SENSITIVE: ReadonlySet<string> = new Set([\n",
    );
    for variant in orisu_merge::ORDER_SENSITIVE {
        let _ = writeln!(out, "  {variant:?},");
    }
    out.push_str("]);\n");
    let alphabet = std::str::from_utf8(orisu_mangle::names::ALPHABET).expect("the alphabet is ASCII");
    let _ = write!(
        out,
        "\n/** The characters of short names, from `orisu_mangle::names::ALPHABET`; the first {first} can start one. */\nexport const NAME_ALPHABET = {alphabet:?};\nexport const NAME_FIRST = {first};\n",
        first = orisu_mangle::names::FIRST,
    );
    out
}

fn flag_object(out: &mut String, doc: &str, name: &str, flags: &[(&str, u8)]) {
    let _ = writeln!(out, "/** {doc} */\nexport const {name} = {{");
    for (flag, value) in flags {
        let _ = writeln!(out, "  {flag}: {value},");
    }
    out.push_str("} as const;\n");
}
