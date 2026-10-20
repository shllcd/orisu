use crate::color::{self, Rgba};
use crate::resolve::resolve_css;
use orisu_allocator::Allocator;
use orisu_ast::{Node, to_css};
use orisu_canonical::{CanonicalizeOptions, Canonicalizer};
use orisu_compat_tailwind::Tailwind;
use orisu_compiler::compiler::{self, Compiler, Options, Stylesheet};
use orisu_compiler::design_system::{Caches, DesignSystem, Session};
use orisu_compiler::intellisense::{self, ClassEntry};
use orisu_scanner::attributify::Attributify;
use rustc_hash::FxHashMap;
use std::cell::{Cell, OnceCell, RefCell};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct ClassInfo {
    pub css: Option<String>,
    pub color: Option<Rgba>,
    /// Sorted properties the class declares, outside `@property` and `@keyframes`.
    pub properties: Vec<String>,
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
    canonicalizer: ManuallyDrop<RefCell<Option<Canonicalizer<'static, 'static>>>>,
    compiler: ManuallyDrop<Box<Compiler<'static>>>,
    caches: RefCell<Caches<'static>>,
    alloc: *mut Allocator,
    variables: FxHashMap<String, String>,
    classes: OnceCell<Vec<ClassEntry>>,
    variants: OnceCell<Vec<VariantItem>>,
    theme_colors: OnceCell<FxHashMap<String, Rgba>>,
    infos: RefCell<FxHashMap<String, Rc<ClassInfo>>>,
    canonical: RefCell<FxHashMap<String, Option<String>>>,
    canonical_ready: Cell<bool>,
}

impl Drop for Project {
    fn drop(&mut self) {
        // SAFETY: `canonicalizer` borrows the boxed compiler and `compiler` borrows from `alloc`, so they are
        // dropped in that order and never used again; `alloc` came from `Box::into_raw` in `load` and is freed exactly once here.
        unsafe {
            ManuallyDrop::drop(&mut self.canonicalizer);
            ManuallyDrop::drop(&mut self.compiler);
            drop(Box::from_raw(self.alloc));
        }
    }
}

#[derive(Default)]
struct Loaded {
    dependencies: Vec<PathBuf>,
    variables: FxHashMap<String, String>,
}

impl Project {
    pub fn load(entry: &Path) -> Result<Self, String> {
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
        let options = Options { base: &base, load_stylesheet: Some(Box::new(load)), extension: Some(&Tailwind), ..Options::default() };
        let alloc = Box::into_raw(Box::new(Allocator::new()));
        // SAFETY: the arena lives until `Project::drop`, which drops the borrowing compiler first.
        let arena: &'static Allocator = unsafe { &*alloc };
        let compiler = match compiler::compile(arena.alloc_str(&css), arena, options) {
            Ok(compiler) => compiler,
            Err(err) => {
                // SAFETY: nothing borrows the arena once compilation failed.
                unsafe { drop(Box::from_raw(alloc)) };
                return Err(err);
            }
        };
        let attributify =
            compiler.design_system.attributify.map(|a| Attributify { prefix: a.prefix.to_owned(), prefixed_only: a.prefixed_only });
        let Loaded { dependencies, variables } = std::mem::take(&mut *loaded.borrow_mut());
        Ok(Self {
            entry: entry.to_path_buf(),
            dir,
            dependencies,
            attributify,
            canonicalizer: ManuallyDrop::new(RefCell::new(None)),
            compiler: ManuallyDrop::new(Box::new(compiler)),
            caches: RefCell::default(),
            alloc,
            variables,
            classes: OnceCell::new(),
            variants: OnceCell::new(),
            theme_colors: OnceCell::new(),
            infos: RefCell::default(),
            canonical: RefCell::default(),
            canonical_ready: Cell::new(false),
        })
    }

    pub fn ds(&self) -> &DesignSystem<'static> {
        &self.compiler.design_system
    }

    fn with_session<T>(&self, f: impl FnOnce(&mut Session<'_, 'static>) -> T) -> T {
        f(&mut Session::new(self.ds(), self.compiler.alloc(), &mut self.caches.borrow_mut()))
    }

    pub fn classes(&self) -> &[ClassEntry] {
        self.classes.get_or_init(|| intellisense::class_list(self.ds()))
    }

    pub fn variants(&self) -> &[VariantItem] {
        self.variants.get_or_init(|| {
            intellisense::variants(self.ds())
                .into_iter()
                .map(|v| VariantItem { name: v.name.to_owned(), values: v.values, has_dash: v.has_dash, is_arbitrary: v.is_arbitrary })
                .collect()
        })
    }

    /// Theme colors by name, such as `red-500`; a class ending in one sets that color.
    pub fn theme_colors(&self) -> &FxHashMap<String, Rgba> {
        self.theme_colors.get_or_init(|| {
            let variable = |name: &str| self.variable(name);
            let theme = &self.ds().theme;
            theme
                .keys_in_namespaces(["--color"])
                .into_iter()
                .filter_map(|name| {
                    let value = theme.get_value(&format!("--color-{name}"))?.value;
                    Some((name.to_owned(), color::parse(value, &variable)?))
                })
                .collect()
        })
    }

    pub fn info(&self, class: &str) -> Rc<ClassInfo> {
        if let Some(info) = self.infos.borrow().get(class) {
            return info.clone();
        }
        let nodes = self.with_session(|ds| ds.candidates_to_ast(&[class])).into_iter().next().filter(|nodes| !nodes.is_empty());
        let mut declarations = Vec::new();
        if let Some(nodes) = &nodes {
            collect_declarations(nodes, &mut declarations);
        }
        let variable = |name: &str| self.variable(name);
        let color =
            declarations.iter().filter(|(property, _)| is_color_property(property)).find_map(|(_, value)| color::parse(value, &variable));
        let mut properties: Vec<String> = declarations.iter().map(|(property, _)| (*property).to_owned()).collect();
        properties.sort_unstable();
        properties.dedup();
        let info = Rc::new(ClassInfo { css: nodes.map(|nodes| to_css(&nodes)), color, properties });
        self.infos.borrow_mut().insert(class.to_owned(), info.clone());
        info
    }

    fn variable(&self, name: &str) -> Option<&str> {
        let ds = self.ds();
        let unprefixed = ds.theme.prefix().and_then(|p| name.strip_prefix("--")?.strip_prefix(p)?.strip_prefix('-'));
        let key = unprefixed.map(|rest| format!("--{rest}"));
        let key = key.as_deref().unwrap_or(name);
        ds.theme.get_value(key).map(|v| v.value).or_else(|| self.variables.get(name).map(String::as_str))
    }

    /// The known class closest to `utility` within a few edits, for misspellings.
    pub fn suggestion(&self, utility: &str) -> Option<&str> {
        let limit = (utility.chars().count() / 4).clamp(1, 3);
        self.classes()
            .iter()
            .filter(|entry| entry.name.len().abs_diff(utility.len()) <= limit)
            .filter_map(|entry| Some((distance(utility, &entry.name, limit)?, entry.name.as_str())))
            .min_by_key(|&(d, _)| d)
            .map(|(_, name)| name)
    }

    /// Whether `name` sits in a theme namespace, such as `--color-brnd`, yet neither the theme nor a stylesheet defines it.
    pub fn is_missing_theme_key(&self, name: &str) -> bool {
        let Some(rest) = name.strip_prefix("--").filter(|rest| !rest.starts_with("tw-")) else {
            return false;
        };
        let Some((namespace, _)) = rest.split_once('-') else {
            return false;
        };
        let namespace = format!("--{namespace}");
        self.variable(name).is_none() && !self.ds().theme.keys_in_namespaces([namespace.as_str()]).is_empty()
    }

    /// The canonical spelling of `class`, `None` when it already is canonical or unknown.
    pub fn canonical(&self, class: &str, rem: f64) -> Option<String> {
        if let Some(canonical) = self.canonical.borrow().get(class) {
            return canonical.clone();
        }
        let options = CanonicalizeOptions { rem: Some(rem), ..CanonicalizeOptions::default() };
        let mut canonicalizer = self.canonicalizer.borrow_mut();
        let canonicalizer = canonicalizer.get_or_insert_with(|| {
            // SAFETY: the compiler is boxed, so the design system stays put until `Project::drop`, which drops this first.
            let ds: &'static DesignSystem<'static> = unsafe { &*std::ptr::from_ref(self.ds()) };
            Canonicalizer::new(ds, self.compiler.alloc())
        });
        let canonical = canonicalizer.canonicalize(&[class], &options).into_iter().next().filter(|c| c != class);
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

fn collect_declarations(nodes: &[Node<'static>], out: &mut Vec<(&'static str, &'static str)>) {
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

/// CSS files under `roots` that import Tailwind, skipping `node_modules` and ignored files.
pub fn discover(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    for root in roots {
        let walker = orisu_ignore::WalkBuilder::new(root).filter_entry(|e| e.file_name() != "node_modules").build();
        for entry in walker.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "css") && std::fs::read_to_string(path).is_ok_and(|css| is_entry(&css)) {
                entries.push(path.to_path_buf());
            }
        }
    }
    entries
}

pub fn is_entry(css: &str) -> bool {
    ["@import \"tailwindcss", "@import 'tailwindcss", "@import url(\"tailwindcss", "@tailwind utilities"]
        .iter()
        .any(|needle| css.contains(needle))
}
