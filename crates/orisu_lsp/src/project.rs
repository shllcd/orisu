use crate::color::{self, Rgba};
use crate::resolve::resolve_css;
use orisu_allocator::Allocator;
use orisu_ast::{Node, to_css};
use orisu_canonical::{CanonicalizeOptions, Canonicalizer};
use orisu_compat_tailwind::Tailwind;
use orisu_compiler::compiler::{self, CompileError, Compiler, Options, Stylesheet};
use orisu_compiler::design_system::{Caches, DesignSystem, Session};
use orisu_compiler::intellisense::{self, ClassEntry};
use orisu_parser::segment::segment;
use orisu_scanner::attributify::Attributify;
use rustc_hash::FxHashMap;
use std::cell::{Cell, OnceCell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

pub struct ClassInfo {
    pub css: Option<String>,
    pub color: Option<Rgba>,
    /// Sorted properties the class declares, outside `@property` and `@keyframes`.
    pub properties: Vec<String>,
    /// Whether a class list shows the class's color: its utility names a theme color or takes an arbitrary value.
    pub swatch: bool,
    /// Length of the class's variant prefix, without the `:` before the utility.
    pub variants: usize,
}

/// What completing classes needs, worked out once.
pub struct ClassCompletions {
    /// Per class, in the order of `classes`.
    pub items: Vec<ClassCompletion>,
    /// Indices into `classes`, bucketed by the name's first byte.
    pub by_first_byte: Vec<Vec<u32>>,
}

pub struct ClassCompletion {
    pub sort_text: String,
    /// The hex of the theme color the class names.
    pub color: Option<String>,
}

pub struct VariantItem {
    pub name: String,
    pub values: Vec<String>,
    pub has_dash: bool,
    pub is_arbitrary: bool,
}

/// A CSS entry point and the design system compiled from it.
pub struct Project {
    pub entry: PathBuf,
    pub dir: PathBuf,
    pub dependencies: Vec<PathBuf>,
    pub attributify: Option<Attributify>,
    arena: Arena,
    variables: FxHashMap<String, String>,
    classes: OnceCell<Vec<ClassEntry>>,
    /// Indices into `classes`, bucketed by name length.
    by_length: OnceCell<Vec<Vec<u32>>>,
    loaded_bytes: usize,
    variants: OnceCell<Vec<VariantItem>>,
    theme_colors: OnceCell<FxHashMap<String, Rgba>>,
    completions: OnceCell<ClassCompletions>,
    infos: RefCell<FxHashMap<String, Rc<ClassInfo>>>,
    canonical: RefCell<FxHashMap<String, Option<String>>>,
    canonical_ready: Cell<bool>,
    missing_theme_keys: RefCell<FxHashMap<String, bool>>,
}

struct State<'a> {
    compiler: Compiler<'a>,
    caches: RefCell<Caches<'a>>,
    canonicalizer: RefCell<Option<Canonicalizer<'a>>>,
}

self_cell::self_cell!(
    struct Arena {
        owner: Allocator,
        #[not_covariant]
        dependent: State,
    }
);

#[derive(Default)]
struct Loaded {
    dependencies: Vec<PathBuf>,
    variables: FxHashMap<String, String>,
}

impl Project {
    pub fn load(entry: &Path) -> Result<Self, CompileError> {
        let css = std::fs::read_to_string(entry).map_err(|e| format!("Could not read `{}`: {e}", entry.display()))?;
        let dir = entry.parent().unwrap_or(Path::new("/")).to_path_buf();
        let loaded = Rc::new(RefCell::new(Loaded::default()));
        collect_variables(&css, &mut loaded.borrow_mut().variables);
        let sink = loaded.clone();
        let load = move |id: &str, base: &str| {
            let path = resolve_css(id, Path::new(base)).ok_or_else(|| format!("Could not resolve '{id}' from '{base}'"))?;
            let content = std::fs::read_to_string(&path).map_err(|e| format!("Could not read `{}`: {e}", path.display()))?;
            let mut sink = sink.borrow_mut();
            collect_variables(&content, &mut sink.variables);
            sink.dependencies.push(path.clone());
            let base = path.parent().unwrap_or(Path::new("/")).to_string_lossy().into_owned();
            Ok(Stylesheet { path: path.to_string_lossy().into_owned(), base, content })
        };
        let base = dir.to_string_lossy();
        let arena = Arena::try_new(Allocator::new(), |alloc| {
            let options = Options { base: &base, load_stylesheet: Some(Box::new(load)), extension: Some(&Tailwind), ..Options::default() };
            compiler::compile(alloc.alloc_str(&css), alloc, options).map(|compiler| State {
                compiler,
                caches: RefCell::default(),
                canonicalizer: RefCell::default(),
            })
        })?;
        let attributify = arena.with_dependent(|_, state| {
            state.compiler.design_system.attributify.map(|a| Attributify { prefix: a.prefix.to_owned(), prefixed_only: a.prefixed_only })
        });
        let Loaded { dependencies, variables } = std::mem::take(&mut *loaded.borrow_mut());
        let loaded_bytes = arena.borrow_owner().allocated_bytes();
        Ok(Self {
            entry: entry.to_path_buf(),
            dir,
            dependencies,
            attributify,
            arena,
            variables,
            classes: OnceCell::new(),
            by_length: OnceCell::new(),
            loaded_bytes,
            variants: OnceCell::new(),
            theme_colors: OnceCell::new(),
            completions: OnceCell::new(),
            infos: RefCell::default(),
            canonical: RefCell::default(),
            canonical_ready: Cell::new(false),
            missing_theme_keys: RefCell::default(),
        })
    }

    /// Whether the arena, which only grows as classes are looked up, has outgrown the freshly loaded one enough to reload.
    pub fn needs_reload(&self) -> bool {
        self.arena.borrow_owner().allocated_bytes() > (self.loaded_bytes * 4).max(64 << 20)
    }

    fn with_ds<T>(&self, f: impl for<'a> FnOnce(&DesignSystem<'a>) -> T) -> T {
        self.arena.with_dependent(|_, state| f(&state.compiler.design_system))
    }

    fn with_session<T>(&self, f: impl for<'s, 'a> FnOnce(&mut Session<'s, 'a>) -> T) -> T {
        self.arena.with_dependent(|alloc, state| f(&mut Session::new(&state.compiler.design_system, alloc, &mut state.caches.borrow_mut())))
    }

    pub fn classes(&self) -> &[ClassEntry] {
        self.classes.get_or_init(|| self.with_ds(intellisense::class_list))
    }

    pub fn variants(&self) -> &[VariantItem] {
        self.variants.get_or_init(|| {
            self.with_ds(|ds| {
                intellisense::variants(ds)
                    .into_iter()
                    .map(|v| VariantItem { name: v.name.to_owned(), values: v.values, has_dash: v.has_dash, is_arbitrary: v.is_arbitrary })
                    .collect()
            })
        })
    }

    pub fn completions(&self) -> &ClassCompletions {
        self.completions.get_or_init(|| {
            let classes = self.classes();
            let mut by_first_byte = vec![Vec::new(); 256];
            for (i, entry) in classes.iter().enumerate() {
                if let Some(&first) = entry.name.as_bytes().first() {
                    by_first_byte[first as usize].push(i as u32);
                }
            }
            let items = classes
                .iter()
                .enumerate()
                .map(|(i, entry)| ClassCompletion { sort_text: format!("a{i:05}"), color: self.names_color(&entry.name).map(|c| c.hex()) })
                .collect();
            ClassCompletions { items, by_first_byte }
        })
    }

    /// The theme color named by a dash-separated suffix of `class`, ignoring any `/modifier`.
    pub fn names_color(&self, class: &str) -> Option<Rgba> {
        let colors = self.theme_colors();
        let class = class.split('/').next().unwrap_or(class);
        class.match_indices('-').find_map(|(i, _)| colors.get(&class[i + 1..]).copied())
    }

    /// Theme colors by name, such as `red-500`; a class ending in one sets that color.
    fn theme_colors(&self) -> &FxHashMap<String, Rgba> {
        self.theme_colors.get_or_init(|| {
            self.with_ds(|ds| {
                let variable = |name: &str| self.variable(ds, name);
                ds.theme
                    .keys_in_namespaces(["--color"])
                    .into_iter()
                    .filter_map(|name| {
                        let value = ds.theme.get_value(&format!("--color-{name}"))?.value;
                        Some((name.to_owned(), color::parse(value, &variable)?))
                    })
                    .collect()
            })
        })
    }

    pub fn info(&self, class: &str) -> Rc<ClassInfo> {
        if let Some(info) = self.infos.borrow().get(class) {
            return info.clone();
        }
        let utility = segment(class, b':').last().copied().unwrap_or_default();
        let swatch = self.names_color(utility).is_some() || utility.contains("-[") || utility.contains("-(");
        let variants = (class.len() - utility.len()).saturating_sub(1);
        let info = Rc::new(self.with_session(|session| {
            let nodes = session.candidates_to_ast(&[class]).into_iter().next().filter(|nodes| !nodes.is_empty());
            let mut declarations = Vec::new();
            if let Some(nodes) = &nodes {
                collect_declarations(nodes, &mut declarations);
            }
            let variable = |name: &str| self.variable(session.ds, name);
            let color = declarations
                .iter()
                .filter(|(property, _)| is_color_property(property))
                .find_map(|(_, value)| color::parse(value, &variable));
            let mut properties: Vec<String> = declarations.iter().map(|(property, _)| (*property).to_owned()).collect();
            properties.sort_unstable();
            properties.dedup();
            ClassInfo { css: nodes.map(|nodes| to_css(&nodes)), color, properties, swatch, variants }
        }));
        self.infos.borrow_mut().insert(class.to_owned(), info.clone());
        info
    }

    fn variable<'s>(&'s self, ds: &'s DesignSystem<'_>, name: &str) -> Option<&'s str> {
        let unprefixed = ds.theme.prefix().and_then(|p| name.strip_prefix("--")?.strip_prefix(p)?.strip_prefix('-'));
        let key = unprefixed.map(|rest| format!("--{rest}"));
        let key = key.as_deref().unwrap_or(name);
        ds.theme.get_value(key).map(|v| v.value).or_else(|| self.variables.get(name).map(String::as_str))
    }

    /// The known class closest to `utility` within a few edits, for misspellings.
    pub fn suggestion(&self, utility: &str) -> Option<&str> {
        let limit = (utility.chars().count() / 4).clamp(1, 3);
        let classes = self.classes();
        let by_length = self.by_length.get_or_init(|| {
            let mut by_length = vec![Vec::new(); classes.iter().map(|entry| entry.name.len() + 1).max().unwrap_or(0)];
            for (i, entry) in classes.iter().enumerate() {
                by_length[entry.name.len()].push(i as u32);
            }
            by_length
        });
        let lengths = utility.len().saturating_sub(limit)..(utility.len() + limit + 1).min(by_length.len());
        by_length
            .get(lengths)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|&i| {
                let name = classes[i as usize].name.as_str();
                Some((distance(utility, name, limit)?, i, name))
            })
            .min_by_key(|&(d, i, _)| (d, i))
            .map(|(_, _, name)| name)
    }

    /// Whether `name` sits in a theme namespace, such as `--color-brnd`, yet neither the theme nor a stylesheet defines it.
    pub fn is_missing_theme_key(&self, name: &str) -> bool {
        let Some(rest) = name.strip_prefix("--").filter(|rest| !rest.starts_with("tw-")) else {
            return false;
        };
        let Some((namespace, _)) = rest.split_once('-') else {
            return false;
        };
        if let Some(&missing) = self.missing_theme_keys.borrow().get(name) {
            return missing;
        }
        let namespace = format!("--{namespace}");
        let missing = self.with_ds(|ds| self.variable(ds, name).is_none() && !ds.theme.keys_in_namespaces([namespace.as_str()]).is_empty());
        self.missing_theme_keys.borrow_mut().insert(name.to_owned(), missing);
        missing
    }

    /// The canonical spelling of `class`, `None` when it already is canonical or unknown.
    pub fn canonical(&self, class: &str, rem: f64) -> Option<String> {
        if let Some(canonical) = self.canonical.borrow().get(class) {
            return canonical.clone();
        }
        let options = CanonicalizeOptions { rem: Some(rem), ..CanonicalizeOptions::default() };
        let canonical = self.arena.with_dependent(|alloc, state| {
            let mut canonicalizer = state.canonicalizer.borrow_mut();
            let canonicalizer = canonicalizer.get_or_insert_with(|| Canonicalizer::new(alloc));
            canonicalizer.canonicalize(&state.compiler.design_system, &[class], &options).into_iter().next().filter(|c| c != class)
        });
        self.canonical.borrow_mut().insert(class.to_owned(), canonical.clone());
        self.canonical_ready.set(true);
        canonical
    }

    /// Whether canonicalizing has run once, building the tables that make later calls quick.
    pub fn canonical_ready(&self) -> bool {
        self.canonical_ready.get()
    }

    /// `classes` in Tailwind's output order, unknown classes first.
    pub fn sort<'c>(&self, classes: &[&'c str]) -> Vec<&'c str> {
        let mut ordered = self.with_session(|ds| ds.class_order(classes));
        ordered.sort_by_key(|&(_, order)| order.map_or((0, 0), |o| (1, o)));
        ordered.into_iter().map(|(class, _)| class).collect()
    }
}

/// Levenshtein distance of `a` and `b`, `None` when it exceeds `limit`.
fn distance(a: &str, b: &str, limit: usize) -> Option<usize> {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        let mut best = row[0];
        for (j, &cb) in b.iter().enumerate() {
            let substitute = previous + usize::from(ca != cb);
            previous = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(previous + 1);
            best = best.min(row[j + 1]);
        }
        if best > limit {
            return None;
        }
    }
    row.last().copied().filter(|&d| d <= limit)
}

fn collect_declarations<'a>(nodes: &[Node<'a>], out: &mut Vec<(&'a str, &'a str)>) {
    for node in nodes {
        match node {
            Node::Decl { property, value: Some(value), .. } => out.push((property, value)),
            Node::AtRule { name: "@property" | "@keyframes", .. } => {}
            Node::AtRule { name: "@layer", params: "properties", .. } => {}
            _ => {
                if let Some(children) = node.children() {
                    collect_declarations(children, out);
                }
            }
        }
    }
}

fn is_color_property(property: &str) -> bool {
    matches!(property, "color" | "fill" | "stroke" | "--tw-gradient-from" | "--tw-gradient-via" | "--tw-gradient-to")
        || property.ends_with("-color")
}

/// Records the first value of each custom property declared in `css`, such as shadcn's
/// `:root { --primary: … }`, so `var(--primary)` can be shown as a color.
fn collect_variables(css: &str, out: &mut FxHashMap<String, String>) {
    let bytes = css.as_bytes();
    for (at, _) in css.match_indices("--") {
        let before = bytes[..at].iter().rev().find(|b| !b.is_ascii_whitespace());
        if !matches!(before, Some(b'{' | b';')) {
            continue;
        }
        let name_end =
            bytes[at..].iter().position(|&b| !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_')).map_or(bytes.len(), |n| at + n);
        let rest = &css[name_end..];
        let Some(value) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let mut depth = 0usize;
        let end = value
            .char_indices()
            .find(|&(_, c)| match c {
                '(' => {
                    depth += 1;
                    false
                }
                ')' => {
                    depth = depth.saturating_sub(1);
                    false
                }
                ';' | '}' => depth == 0,
                _ => false,
            })
            .map_or(value.len(), |(i, _)| i);
        out.entry(css[at..name_end].to_owned()).or_insert_with(|| value[..end].trim().to_owned());
    }
}

/// CSS files under `roots` that import Tailwind, skipping `node_modules` and ignored files, sorted by path within each root.
pub fn discover(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    for root in roots {
        let found = Mutex::new(Vec::new());
        let walker = orisu_ignore::WalkBuilder::new(root).filter_entry(|e| e.file_name() != "node_modules").build_parallel();
        walker.run(|| {
            Box::new(|entry| {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "css") && std::fs::read_to_string(path).is_ok_and(|css| is_entry(&css)) {
                        found.lock().unwrap().push(entry.into_path());
                    }
                }
                orisu_ignore::WalkState::Continue
            })
        });
        let mut found = found.into_inner().unwrap();
        found.sort_unstable();
        entries.append(&mut found);
    }
    entries
}

pub fn is_entry(css: &str) -> bool {
    ["@import \"tailwindcss", "@import 'tailwindcss", "@import url(\"tailwindcss", "@tailwind utilities"]
        .iter()
        .any(|needle| css.contains(needle))
}
