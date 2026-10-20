use orisu_allocator::Allocator;
use orisu_canonical::{CanonicalizeOptions, Canonicalizer};
use orisu_compat_tailwind::Tailwind;
use orisu_compiler::compiler::{Options, Stylesheet, compile};
use orisu_compiler::intellisense;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Compiles every `<name>.css` in `dir` with the candidates from `<name>.txt` through orisu and Tailwind, then compares the outputs.
pub fn run(args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("missing <dir>")?;
    let dir = fs::canonicalize(dir).map_err(|e| format!("{dir}: {e}"))?;
    let tailwind = match args.iter().position(|a| a == "--tailwind") {
        Some(i) => PathBuf::from(args.get(i + 1).ok_or("missing value for --tailwind")?),
        None => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".bun/install/global/node_modules"),
    };

    let mut names: Vec<String> = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|file| file.strip_suffix(".css").filter(|name| !name.contains(".out")).map(str::to_owned))
        .collect();
    names.sort();

    for name in &names {
        let (css, extra) = compile_one(&dir, name);
        fs::write(dir.join(format!("{name}.orisu.out")), css).map_err(|e| e.to_string())?;
        for (ext, out) in extra {
            fs::write(dir.join(format!("{name}.{ext}.orisu.out")), out).map_err(|e| e.to_string())?;
        }
    }

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("conformance.mjs");
    let status = Command::new("node").arg(script).arg(&dir).arg(&tailwind).status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("conformance.mjs failed".to_owned());
    }

    let mut outputs: Vec<String> = names.clone();
    for ext in EXTRA {
        outputs.extend(names.iter().filter(|name| dir.join(format!("{name}.{ext}")).exists()).map(|name| format!("{name}.{ext}")));
    }
    let mut failed = 0;
    for name in &outputs {
        let ours = fs::read(dir.join(format!("{name}.orisu.out"))).map_err(|e| e.to_string())?;
        let theirs = fs::read(dir.join(format!("{name}.tailwind.out"))).map_err(|e| e.to_string())?;
        if ours == theirs {
            println!("ok    {name}");
        } else {
            failed += 1;
            let at = ours.iter().zip(&theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
            println!("FAIL  {name} (first difference at byte {at})");
        }
    }
    println!("{} passed, {failed} failed", outputs.len() - failed);
    if failed > 0 { Err(format!("{failed} case(s) differ")) } else { Ok(()) }
}

/// Collapsing is exponential in the number of candidates, in Tailwind as well, so only this many take part.
const COLLAPSED: usize = 32;

/// Files next to `<name>.css` that turn on an extra comparison: `.canon` lists candidates to canonicalize, `.classes` compares
/// the class list and variants.
const EXTRA: [&str; 2] = ["canon", "classes"];

/// The stylesheet and the outputs of the comparisons [`EXTRA`] turns on.
fn compile_one(dir: &Path, name: &str) -> (String, std::vec::Vec<(&'static str, String)>) {
    let read = |ext: &str| fs::read_to_string(dir.join(format!("{name}.{ext}"))).unwrap_or_default();
    let css = read("css");
    let candidates = read("txt");
    let alloc = Allocator::new();
    let base = dir.to_string_lossy().into_owned();
    let options =
        Options { base: &base, load_stylesheet: Some(Box::new(load_stylesheet)), extension: Some(&Tailwind), ..Options::default() };
    let mut compiler = match compile(&css, &alloc, options) {
        Ok(compiler) => compiler,
        Err(error) => return (format!("error: {error}"), std::vec::Vec::new()),
    };
    let output = compiler.build(candidates.split_whitespace()).to_owned();
    let mut extra = std::vec::Vec::new();
    if let Ok(list) = fs::read_to_string(dir.join(format!("{name}.canon"))) {
        let mut canonicalizer = Canonicalizer::new(&compiler.design_system, compiler.alloc());
        let list: Vec<&str> = list.split_whitespace().collect();
        let mut out = String::new();
        for options in [CanonicalizeOptions::default(), CanonicalizeOptions { rem: Some(16.0), ..CanonicalizeOptions::default() }] {
            for candidate in &list {
                out.push_str(&canonicalizer.canonicalize(&[candidate], &options).join(" "));
                out.push('\n');
            }
        }
        let collapse = CanonicalizeOptions { rem: Some(16.0), collapse: true, ..CanonicalizeOptions::default() };
        out.push_str(&canonicalizer.canonicalize(&list[..list.len().min(COLLAPSED)], &collapse).join(" "));
        out.push('\n');
        extra.push(("canon", out));
    }
    if dir.join(format!("{name}.classes")).exists() {
        extra.push(("classes", classes(&compiler.design_system)));
    }
    (output, extra)
}

/// One line per class with its modifiers, then one line per variant, in the format `conformance.mjs` prints.
fn classes(ds: &orisu_compiler::design_system::DesignSystem<'_>) -> String {
    let mut out = String::new();
    for entry in intellisense::class_list(ds) {
        out.push_str(&entry.name);
        for modifier in &entry.modifiers {
            out.push(' ');
            out.push_str(modifier);
        }
        out.push('\n');
    }
    for variant in intellisense::variants(ds) {
        out.push_str(&format!("@{} {} {}", variant.name, variant.is_arbitrary, variant.has_dash));
        for value in &variant.values {
            out.push(' ');
            out.push_str(value);
        }
        out.push('\n');
    }
    out
}

pub(crate) fn load_stylesheet(uri: &str, base: &str) -> Result<Stylesheet, String> {
    let path = Path::new(base).join(uri);
    let content = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let base = path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Stylesheet { path: path.to_string_lossy().into_owned(), base, content })
}
