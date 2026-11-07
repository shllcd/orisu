mod css;
pub mod names;
mod rewrite;

use indexmap::IndexMap;
use orisu_analyze::{Fragment, Token};
use orisu_diagnostics::Code;
use orisu_merge::{MergeTable, Merger};
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

const JS_EXTENSIONS: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts"];

#[derive(Clone, Debug, Default)]
pub struct Options {
    prefix: String,
    keep: Vec<String>,
    positions: orisu_analyze::Options,
    variant_groups: bool,
}

impl Options {
    /// `prefix` starts every short name and must keep each one a CSS identifier without escapes; `variant_groups` tells
    /// whether variant groups in source strings are expanded before the build sees them.
    pub fn new(prefix: String, keep: Vec<String>, positions: orisu_analyze::Options, variant_groups: bool) -> Result<Self, String> {
        if !names::valid_name_prefix(&prefix) {
            return Err(format!("`mangle.prefix` must be a CSS identifier start of letters, digits, `_` and `-`, got `{prefix}`"));
        }
        Ok(Self { prefix, keep, positions, variant_groups })
    }
}

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
    names: IndexMap<String, String, FxBuildHasher>,
    shorts: FxHashSet<String>,
    table: MergeTable,
    name_table: NameTable,
}

/// The classes of a JavaScript module, read from its source after its variant groups are expanded when asked.
struct Analyzed {
    /// The source the tokens point into when expanding changed it.
    expanded: Option<String>,
    tokens: Vec<Token>,
    pinned: FxHashSet<String>,
    fragments: Vec<Fragment>,
    name_merger: Option<String>,
}

impl Analyzed {
    fn source<'s>(&'s self, original: &'s str) -> &'s str {
        self.expanded.as_deref().unwrap_or(original)
    }
}

/// The class positions of a JavaScript module, or every candidate in it pinned when it does not parse.
fn analyze_js(source: &str, path: &Path, options: &Options) -> Result<Analyzed, FxHashSet<String>> {
    let expanded = if options.variant_groups { orisu_candidate::group::expand_in_strings(source) } else { None };
    let Some(analysis) = orisu_analyze::analyze(expanded.as_deref().unwrap_or(source), path, &options.positions) else {
        return Err(orisu_scanner::file_candidates(source.as_bytes().to_vec(), extension(path)));
    };
    let mut pinned: FxHashSet<String> = analysis.pinned.into_iter().collect();
    for string in &analysis.strings {
        pinned.extend(orisu_scanner::file_candidates(string.as_bytes().to_vec(), "js"));
    }
    Ok(Analyzed { expanded, tokens: analysis.tokens, pinned, fragments: analysis.fragments, name_merger: analysis.name_merger })
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

fn file_facts(path: &Path, entry: &Path, options: &Options) -> Result<Facts, String> {
    let content = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ext = extension(path);
    if ext == "css" {
        let pinned = if path == entry { FxHashSet::default() } else { css::classes(&String::from_utf8_lossy(&content)) };
        return Ok(Facts { pinned, ..Facts::default() });
    }
    if JS_EXTENSIONS.contains(&ext)
        && !in_node_modules(path)
        && let Ok(source) = std::str::from_utf8(&content)
    {
        return Ok(match analyze_js(source, path, options) {
            Ok(analyzed) => {
                let source = analyzed.source(source);
                let tokens = analyzed.tokens.iter().map(|t| source[t.start as usize..t.end as usize].to_owned()).collect();
                Facts { tokens, pinned: analyzed.pinned, fragments: analyzed.fragments }
            }
            Err(pinned) => Facts { pinned, ..Facts::default() },
        });
    }
    Ok(Facts { pinned: orisu_scanner::file_candidates(content, ext), ..Facts::default() })
}

/// Rewrites the class positions of a module into placeholders, expanding its variant groups first when asked. A module
/// that does not parse keeps its code and pins every candidate in it.
pub fn module(source: &str, path: &Path, options: &Options) -> Result<Module, String> {
    if source.contains(rewrite::MARKER) {
        return Err(format!("{} already contains `{}`, which `mangle` reserves for its placeholders.", path.display(), rewrite::MARKER));
    }
    let analyzed = match analyze_js(source, path, options) {
        Ok(analyzed) => analyzed,
        Err(pinned) => return Ok(Module { code: None, pinned: pinned.into_iter().collect(), fragments: Vec::new(), name_merger: None }),
    };
    let source = analyzed.source(source);
    let edits: Vec<_> = analyzed
        .tokens
        .iter()
        .map(|t| {
            let class = &source[t.start as usize..t.end as usize];
            let text = if t.quote { format!("\"{}\"", rewrite::placeholder(class)) } else { rewrite::placeholder(class) };
            (t.start as usize..t.end as usize, text)
        })
        .collect();
    let rewrite = !edits.is_empty() || analyzed.expanded.is_some();
    let code = rewrite.then(|| rewrite::apply(source, &path.to_string_lossy(), &edits));
    Ok(Module { code, pinned: analyzed.pinned.into_iter().collect(), fragments: analyzed.fragments, name_merger: analyzed.name_merger })
}

/// Picks the classes to rename and their short names; fails when a scanned file cannot be read.
pub fn plan(merger: &mut Merger<'_, '_>, input: &Input<'_>, options: &Options) -> Result<Plan, String> {
    let facts =
        input.files.par_iter().map(|path| file_facts(path, input.entry, options)).try_reduce(Facts::default, |a, b| Ok(a.join(b)))?;
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
    let (table, kept_groups) = merger.table(input.candidates.iter().map(String::as_str).filter(|c| !renamed_set.contains(c)));

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
    reserved.extend(table.keys.iter().map(String::as_str));
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
    Ok(Plan::new(names, table, name_table))
}

/// The saved form of a [`Plan`].
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlanJson {
    /// Each renamed class followed by its short name.
    names: Vec<String>,
    keys: Vec<String>,
    values: Vec<u32>,
    erases: Vec<Vec<u32>>,
    name_prefix: String,
    groups: Vec<String>,
    opaque: u32,
    runs: Vec<(u32, Vec<u32>)>,
    holes: Vec<u64>,
}

impl Plan {
    fn new(names: IndexMap<String, String, FxBuildHasher>, table: MergeTable, name_table: NameTable) -> Self {
        let shorts = names.values().cloned().collect();
        Self { names, shorts, table, name_table }
    }

    /// Renamed classes and their short names, in naming order.
    pub fn names(&self) -> &IndexMap<String, String, FxBuildHasher> {
        &self.names
    }

    /// Shapes of the classes that keep their names.
    pub fn table(&self) -> &MergeTable {
        &self.table
    }

    pub fn name_table(&self) -> &NameTable {
        &self.name_table
    }

    /// The plan's fields, which `from_json` reads back.
    pub fn to_json(&self) -> Map<String, Value> {
        let names = &self.name_table;
        let json = PlanJson {
            names: self.names.iter().flat_map(|(class, short)| [class.clone(), short.clone()]).collect(),
            keys: self.table.keys.clone(),
            values: self.table.values.clone(),
            erases: self.table.erases.clone(),
            name_prefix: names.prefix.clone(),
            groups: names.groups.clone(),
            opaque: names.opaque,
            runs: names.runs.clone(),
            holes: names.holes.clone(),
        };
        match serde_json::to_value(json) {
            Ok(Value::Object(map)) => map,
            _ => unreachable!("a struct serializes to an object"),
        }
    }

    pub fn from_json(value: &Value) -> Result<Plan, String> {
        let invalid = |what: &str| format!("the saved mangle plan is invalid: {what}");
        let json = PlanJson::deserialize(value).map_err(|e| invalid(&e.to_string()))?;
        if json.names.len() % 2 != 0 {
            return Err(invalid("`names` has an odd length"));
        }
        if json.keys.len() != json.values.len() {
            return Err(invalid("`keys` and `values` differ in length"));
        }
        let mut pairs = json.names.into_iter();
        let names = std::iter::from_fn(|| Some((pairs.next()?, pairs.next()?))).collect();
        let table = MergeTable { keys: json.keys, values: json.values, erases: json.erases };
        let name_table =
            NameTable { prefix: json.name_prefix, groups: json.groups, opaque: json.opaque, runs: json.runs, holes: json.holes };
        Ok(Plan::new(names, table, name_table))
    }

    /// `css` with the renamed classes of its selectors replaced by their short names.
    pub fn rename_css(&self, css: &str) -> String {
        css::rename(css, &self.names)
    }

    /// `code` with each placeholder replaced by its class's short name, or by the class when it keeps its name, and
    /// a JSON source map back to `code`; `None` without placeholders.
    pub fn fill(&self, code: &str, name: &str) -> Option<(String, String)> {
        fill(code, name, |class| self.names.get(&class).cloned().unwrap_or(class))
    }

    /// The error of a built module that uses renamed classes where they cannot be renamed.
    pub fn check_module(&self, file: &str, module: &Module) -> Option<String> {
        if let Some(import) = &module.name_merger {
            return Some(orisu_diagnostics::message(Code::MangleNameMerger, &[("file", file.to_owned()), ("module", import.clone())]));
        }
        let mut pinned: Vec<&String> = module.pinned.iter().filter(|class| self.names.contains_key(*class)).collect();
        pinned.sort_unstable();
        let class = pinned
            .first()
            .map(|c| c.as_str())
            .or_else(|| self.names.keys().map(String::as_str).find(|class| module.fragments.iter().any(|f| f.matches(class))))?;
        Some(orisu_diagnostics::message(Code::MangleUntrackedClass, &[("class", class.to_owned()), ("file", file.to_owned())]))
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
            Some(orisu_diagnostics::message(
                Code::MangleClassCollision,
                &[("file", file.to_owned()), ("class", class), ("reason", reason.to_owned())],
            ))
        })
    }
}

/// `code` with each placeholder replaced by its class, for a build that renames nothing, and a JSON source map back to
/// `code`; `None` without placeholders.
pub fn fill_placeholders(code: &str, name: &str) -> Option<(String, String)> {
    fill(code, name, |class| class)
}

fn fill(code: &str, name: &str, short: impl Fn(String) -> String) -> Option<(String, String)> {
    let edits: Vec<_> = rewrite::placeholders(code).into_iter().map(|(range, class)| (range, short(class))).collect();
    (!edits.is_empty()).then(|| rewrite::apply(code, name, &edits))
}
