pub mod css;
pub mod names;
pub mod rewrite;

use indexmap::IndexMap;
use orisu_analyze::Fragment;
use orisu_diagnostics::{Code, Report, Span};
use orisu_merge::{MergeTable, Merger};
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

const JS_EXTENSIONS: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts"];

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Starts every short name.
    pub prefix: String,
    /// Classes that keep their names.
    pub keep: Vec<String>,
    pub positions: orisu_analyze::Options,
    /// Whether variant groups are expanded in source strings before the build sees them.
    pub variant_groups: bool,
}

/// What the plan is computed from.
pub struct Input<'i> {
    /// Every scanned file.
    pub files: &'i [PathBuf],
    /// The stylesheet the plan renames, which is not scanned for foreign classes.
    pub entry: &'i Path,
    pub candidates: &'i [String],
    /// The stylesheet built from `candidates`.
    pub css: &'i str,
    /// Classes the stylesheet asks for by name.
    pub inline: &'i [&'i str],
}

/// A module of the build, rewritten so its classes can take their short names once the plan is known.
pub struct Module {
    /// The rewritten code and its JSON source map, when there was something to rewrite.
    pub code: Option<(String, String)>,
    /// Classes the module uses where they cannot be renamed.
    pub pinned: Vec<String>,
    pub fragments: Vec<Fragment>,
    /// The import that merges classes by name, if any.
    pub name_merger: Option<String>,
}

/// Short names of the merging classes, in the form the JS `cn` runtime rebuilds them from.
#[derive(Debug, Default)]
pub struct NameTable {
    pub prefix: String,
    /// Variant groups shared with classes that keep their names, as the runtime spells them; group `i` has id `i + 1`.
    pub groups: Vec<String>,
    /// How many group ids after the shared ones only renamed classes use.
    pub opaque: u32,
    /// For each run of names, its group id and the shapes of its names.
    pub runs: Vec<(u32, Vec<u32>)>,
    /// Name indices skipped because the name is taken.
    pub holes: Vec<u64>,
}

pub struct Plan {
    /// Renamed classes and their short names.
    pub names: IndexMap<String, String, FxBuildHasher>,
    pub shorts: FxHashSet<String>,
    /// Shapes of the classes that keep their names.
    pub table: MergeTable,
    pub name_table: NameTable,
}

#[derive(Default)]
struct Facts {
    tokens: FxHashSet<String>,
    pinned: FxHashSet<String>,
    fragments: Vec<Fragment>,
}

impl Facts {
    fn join(mut self, other: Facts) -> Facts {
        self.tokens.extend(other.tokens);
        self.pinned.extend(other.pinned);
        self.fragments.extend(other.fragments);
        self
    }
}

fn extension(path: &Path) -> &str {
    path.extension().and_then(|e| e.to_str()).unwrap_or("")
}

fn in_node_modules(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "node_modules")
}

fn string_candidates(strings: &[String], out: &mut FxHashSet<String>) {
    for string in strings {
        out.extend(orisu_scanner::scanner::file_candidates(string.as_bytes().to_vec(), "js"));
    }
}

fn file_facts(path: &Path, entry: &Path, options: &Options) -> Facts {
    let mut facts = Facts::default();
    let Ok(content) = std::fs::read(path) else { return facts };
    let ext = extension(path);
    if ext == "css" {
        if path != entry {
            facts.pinned = css::classes(&String::from_utf8_lossy(&content));
        }
        return facts;
    }
    if JS_EXTENSIONS.contains(&ext)
        && !in_node_modules(path)
        && let Ok(source) = std::str::from_utf8(&content)
        && let expanded = options.variant_groups.then(|| orisu_candidate::group::expand_in_strings(source)).flatten()
        && let source = expanded.as_deref().unwrap_or(source)
        && let Some(analysis) = orisu_analyze::analyze(source, path, &options.positions)
    {
        facts.tokens = analysis.tokens.iter().map(|t| source[t.start as usize..t.end as usize].to_owned()).collect();
        facts.pinned.extend(analysis.pinned);
        string_candidates(&analysis.strings, &mut facts.pinned);
        facts.fragments = analysis.fragments;
        return facts;
    }
    facts.pinned = orisu_scanner::scanner::file_candidates(content, ext);
    facts
}

/// Rewrites the class positions of a module into placeholders. A module that does not parse keeps its code and pins
/// every candidate in it.
pub fn module(source: &str, path: &Path, options: &Options) -> Result<Module, String> {
    if source.contains(rewrite::MARKER) {
        return Err(format!("{} already contains `{}`, which `mangle` reserves for its placeholders.", path.display(), rewrite::MARKER));
    }
    let Some(analysis) = orisu_analyze::analyze(source, path, &options.positions) else {
        let pinned = orisu_scanner::scanner::file_candidates(source.as_bytes().to_vec(), extension(path));
        return Ok(Module { code: None, pinned: pinned.into_iter().collect(), fragments: Vec::new(), name_merger: None });
    };
    let edits: Vec<_> = analysis
        .tokens
        .iter()
        .map(|t| {
            let class = &source[t.start as usize..t.end as usize];
            let text = if t.quote { format!("\"{}\"", rewrite::placeholder(class)) } else { rewrite::placeholder(class) };
            (t.start as usize..t.end as usize, text)
        })
        .collect();
    let code = (!edits.is_empty()).then(|| rewrite::apply(source, &path.to_string_lossy(), &edits));
    let mut pinned: FxHashSet<String> = analysis.pinned.into_iter().collect();
    string_candidates(&analysis.strings, &mut pinned);
    Ok(Module { code, pinned: pinned.into_iter().collect(), fragments: analysis.fragments, name_merger: analysis.name_merger })
}

/// Picks the classes to rename and their short names.
pub fn plan(merger: &mut Merger<'_, '_>, input: &Input<'_>, options: &Options) -> Plan {
    let facts = input.files.par_iter().map(|path| file_facts(path, input.entry, options)).reduce(Facts::default, Facts::join);
    let selector_classes = css::classes(input.css);
    let kept: FxHashSet<&str> = options.keep.iter().map(String::as_str).chain(input.inline.iter().copied()).collect();
    let mut renamed: Vec<&str> = facts
        .tokens
        .iter()
        .map(String::as_str)
        .filter(|class| {
            selector_classes.contains(*class)
                && !facts.pinned.contains(*class)
                && !kept.contains(class)
                && !facts.fragments.iter().any(|f| f.matches(class))
        })
        .collect();
    renamed.sort_unstable();

    let mut shaped: Vec<(String, Option<u32>, &str)> =
        renamed.iter().map(|&class| merger.class_shape(class)).zip(&renamed).map(|((g, s), c)| (g, s, *c)).collect();
    let renamed_set: FxHashSet<&str> = renamed.iter().copied().collect();
    let kept_classes: Vec<&str> = input.candidates.iter().map(String::as_str).filter(|c| !renamed_set.contains(c)).collect();
    let table = merger.table(kept_classes.iter().copied());

    let mut kept_groups = FxHashSet::default();
    for &class in &kept_classes {
        if let (group, Some(_)) = merger.class_shape(class) {
            kept_groups.insert(group);
        }
    }
    let mut groups: Vec<String> = shaped.iter().filter(|(g, s, _)| s.is_some() && !g.is_empty()).map(|(g, _, _)| g.clone()).collect();
    groups.sort_unstable();
    groups.dedup();
    let (shared, opaque): (Vec<String>, Vec<String>) = groups.into_iter().partition(|g| kept_groups.contains(g));
    let mut ids: FxHashMap<String, u32> = FxHashMap::default();
    ids.insert(String::new(), 0);
    for (i, group) in shared.iter().chain(&opaque).enumerate() {
        ids.insert(group.clone(), i as u32 + 1);
    }
    shaped.sort_unstable_by_key(|(g, s, c)| (s.is_none(), s.map(|_| ids[g.as_str()]), *s, *c));

    let mut reserved: FxHashSet<&str> = facts.tokens.iter().map(String::as_str).collect();
    reserved.extend(table.entries.keys().map(String::as_str));
    reserved.extend(selector_classes.iter().map(String::as_str));
    reserved.extend(kept.iter().copied());
    reserved.extend(facts.pinned.iter().map(String::as_str));

    let mut names = IndexMap::default();
    let mut name_table = NameTable { prefix: options.prefix.clone(), groups: shared, opaque: opaque.len() as u32, ..NameTable::default() };
    let mut k = 0;
    for (group, shape, class) in &shaped {
        let short = loop {
            let short = format!("{}{}", options.prefix, names::name(k));
            if !reserved.contains(short.as_str()) {
                break short;
            }
            if shape.is_some() {
                name_table.holes.push(k);
            }
            k += 1;
        };
        k += 1;
        if let Some(shape) = shape {
            let id = ids[group.as_str()];
            match name_table.runs.last_mut() {
                Some((last, shapes)) if *last == id => shapes.push(*shape),
                _ => name_table.runs.push((id, vec![*shape])),
            }
        }
        names.insert((*class).to_owned(), short);
    }
    let shorts = names.values().cloned().collect();
    Plan { names, shorts, table, name_table }
}

impl Plan {
    pub fn rename_css(&self, css: &str) -> String {
        css::rename(css, &self.names)
    }

    /// `code` with each placeholder replaced by its class's short name, or by the class when it keeps its name, and
    /// a JSON source map back to `code`; `None` without placeholders.
    pub fn fill(&self, code: &str, name: &str) -> Option<(String, String)> {
        let edits: Vec<_> = rewrite::placeholders(code)
            .into_iter()
            .map(|(range, class)| {
                let text = self.names.get(&class).cloned().unwrap_or(class);
                (range, text)
            })
            .collect();
        (!edits.is_empty()).then(|| rewrite::apply(code, name, &edits))
    }

    /// The error of a built module that uses renamed classes where they cannot be renamed.
    pub fn check_module(&self, file: &str, module: &Module) -> Option<String> {
        if let Some(import) = &module.name_merger {
            return Some(name_merger_message(file, import));
        }
        let mut pinned: Vec<&String> = module.pinned.iter().filter(|class| self.names.contains_key(*class)).collect();
        pinned.sort_unstable();
        let class = pinned
            .first()
            .map(|c| c.as_str())
            .or_else(|| self.names.keys().map(String::as_str).find(|class| module.fragments.iter().any(|f| f.matches(class))))?;
        Some(Report::new(Code::MangleUntrackedClass, Span::new(0, 0)).arg("class", class).arg("file", file).message())
    }

    /// The error of a stylesheet outside the build whose classes the plan renames or reuses.
    pub fn check_css(&self, file: &str, css: &str) -> Option<String> {
        let mut classes: Vec<String> = css::classes(css).into_iter().collect();
        classes.sort_unstable();
        classes.into_iter().find_map(|class| {
            let reason = if self.names.contains_key(&class) {
                "renames"
            } else if self.shorts.contains(&class) {
                "gives as a short name to another class"
            } else {
                return None;
            };
            Some(
                Report::new(Code::MangleClassCollision, Span::new(0, 0))
                    .arg("file", file)
                    .arg("class", class)
                    .arg("reason", reason)
                    .message(),
            )
        })
    }
}

pub fn name_merger_message(file: &str, module: &str) -> String {
    Report::new(Code::MangleNameMerger, Span::new(0, 0)).arg("file", file).arg("module", module).message()
}
