// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use napi::bindgen_prelude::{Either, FnArgs, Function, Null};
use napi::{Error, Result};
use napi_derive::napi;
use orisu_allocator::Allocator;
use orisu_candidate::print::print_candidate;
use orisu_canonical::Canonicalizer;
use orisu_compat_tailwind::Tailwind;
use orisu_compiler::compiler::{self as core_compiler, Options, SourceRoot, Stylesheet};
use orisu_compiler::intellisense;
use orisu_merge::Merger;
use orisu_registry::utilities::UtilityKind;
use std::mem::ManuallyDrop;
use std::path::PathBuf;
use utf16::IndexConverter;

mod utf16;

#[derive(Debug, Clone)]
#[napi(object)]
pub struct ChangedContent {
    pub file: Option<String>,
    pub content: Option<String>,
    pub extension: String,
}

#[derive(Debug, Clone)]
#[napi(object)]
pub struct GlobEntry {
    pub base: String,
    pub pattern: String,
}

#[derive(Debug, Clone)]
#[napi(object)]
pub struct SourceEntry {
    pub base: String,
    pub pattern: String,
    pub negated: bool,
}

impl From<ChangedContent> for orisu_scanner::ChangedContent {
    fn from(changed: ChangedContent) -> Self {
        match (changed.file, changed.content) {
            (Some(file), _) => orisu_scanner::ChangedContent::File(file.into(), changed.extension),
            (None, content) => orisu_scanner::ChangedContent::Content(content.unwrap_or_default(), changed.extension),
        }
    }
}

impl From<orisu_scanner::GlobEntry> for GlobEntry {
    fn from(glob: orisu_scanner::GlobEntry) -> Self {
        Self { base: glob.base, pattern: glob.pattern }
    }
}

impl From<SourceEntry> for orisu_scanner::PublicSourceEntry {
    fn from(source: SourceEntry) -> Self {
        Self { base: source.base, pattern: source.pattern, negated: source.negated }
    }
}

#[derive(Debug, Clone)]
#[napi(object)]
pub struct AttributifyOptions {
    pub prefix: String,
    pub prefixed_only: bool,
}

#[derive(Debug, Clone)]
#[napi(object)]
pub struct ScannerOptions {
    pub sources: Option<Vec<SourceEntry>>,
    pub attributify: Option<Either<AttributifyOptions, Null>>,
    /// Directory for the on-disk candidate cache shared between scanners.
    pub cache: Option<String>,
}

#[derive(Debug, Clone)]
#[napi(object)]
pub struct CandidateWithPosition {
    pub candidate: String,
    pub position: i64,
}

#[napi]
pub struct Scanner {
    scanner: orisu_scanner::Scanner,
}

#[napi]
impl Scanner {
    #[napi(constructor)]
    pub fn new(opts: ScannerOptions) -> Self {
        let sources = opts.sources.unwrap_or_default().into_iter().map(Into::into).collect();
        let mut scanner = orisu_scanner::Scanner::new(sources);
        if let Some(Either::A(AttributifyOptions { prefix, prefixed_only })) = opts.attributify {
            scanner = scanner.with_attributify(orisu_scanner::attributify::Attributify { prefix, prefixed_only });
        }
        if let Some(dir) = opts.cache {
            scanner = scanner.with_cache(dir);
        }
        Self { scanner }
    }

    #[napi]
    pub fn scan(&mut self) -> Vec<String> {
        self.scanner.scan()
    }

    #[napi]
    pub fn scan_files(&mut self, input: Vec<ChangedContent>) -> Vec<String> {
        self.scanner.scan_content(input.into_iter().map(Into::into).collect())
    }

    #[napi]
    pub fn get_candidates_with_positions(&mut self, input: ChangedContent) -> Result<Vec<CandidateWithPosition>> {
        let content = match (input.content, input.file) {
            (Some(content), _) => content,
            (None, Some(file)) => std::fs::read_to_string(file).map_err(|e| Error::from_reason(e.to_string()))?,
            (None, None) => String::new(),
        };
        let mut utf16_idx = IndexConverter::new(&content);
        let candidates =
            self.scanner.get_candidates_with_positions(orisu_scanner::ChangedContent::Content(content.clone(), input.extension));
        Ok(candidates
            .into_iter()
            .map(|(candidate, position)| CandidateWithPosition { candidate, position: utf16_idx.get(position) })
            .collect())
    }

    #[napi(getter)]
    pub fn files(&mut self) -> Vec<String> {
        self.scanner.get_files()
    }

    #[napi(getter)]
    pub fn scanned_files(&self) -> Vec<String> {
        self.scanner.get_scanned_files()
    }

    #[napi(getter)]
    pub fn globs(&mut self) -> Vec<GlobEntry> {
        self.scanner.get_globs().into_iter().map(Into::into).collect()
    }

    #[napi(getter)]
    pub fn normalized_sources(&mut self) -> Vec<GlobEntry> {
        self.scanner.get_normalized_sources().into_iter().map(Into::into).collect()
    }
}

#[napi(object)]
pub struct LoadedStylesheet {
    pub path: String,
    pub base: String,
    pub content: String,
}

#[napi(object)]
pub struct CompileOptions {
    pub base: Option<String>,
    pub polyfills: Option<u32>,
}

#[napi(object)]
pub struct CanonicalizeOptions {
    pub rem: Option<f64>,
    pub collapse: Option<bool>,
    pub logical_to_physical: Option<bool>,
}

#[napi(object)]
pub struct CompilerSource {
    pub base: String,
    pub pattern: String,
    pub negated: bool,
}

#[napi(object)]
pub struct CompilerRoot {
    pub base: String,
    pub pattern: String,
}

#[napi(object)]
pub struct ClassEntry {
    pub name: String,
    pub modifiers: Vec<String>,
}

#[napi(object)]
pub struct VariantEntry {
    pub name: String,
    pub is_arbitrary: bool,
    pub values: Vec<String>,
    pub has_dash: bool,
}

#[napi(object)]
pub struct ThemeEntry {
    pub key: String,
    pub value: String,
    pub options: u32,
}

#[napi(object)]
pub struct MergeTable {
    pub prefix: Option<String>,
    pub keys: Vec<String>,
    pub values: Vec<u32>,
    pub erases: Vec<Vec<u32>>,
}

/// Owns the arena that the inner compiler borrows from.
#[napi]
pub struct Compiler {
    canonicalizer: ManuallyDrop<Option<Canonicalizer<'static, 'static>>>,
    inner: ManuallyDrop<Box<core_compiler::Compiler<'static>>>,
    alloc: *mut Allocator,
}

impl Drop for Compiler {
    fn drop(&mut self) {
        // SAFETY: `canonicalizer` borrows the boxed compiler and `inner` borrows from `alloc`, so they are dropped in that
        // order and never used again; `alloc` came from `Box::into_raw` in `compile` and is freed exactly once here.
        unsafe {
            ManuallyDrop::drop(&mut self.canonicalizer);
            ManuallyDrop::drop(&mut self.inner);
            drop(Box::from_raw(self.alloc));
        }
    }
}

/// Expands `hover:(a b)` variant groups inside the string literals of `code`; `null` when none were found.
#[napi]
pub fn expand_variant_groups(code: String) -> Option<String> {
    orisu_candidate::group::expand_in_strings(&code)
}

type LoadFn<'f> = Function<'f, FnArgs<(String, String)>, LoadedStylesheet>;

#[napi]
pub fn compile(css: String, options: Option<CompileOptions>, load_stylesheet: Option<LoadFn<'_>>) -> Result<Compiler> {
    let alloc = Box::into_raw(Box::new(Allocator::new()));
    // SAFETY: the arena lives until `Compiler::drop`, which drops the borrowing compiler first.
    let arena: &'static Allocator = unsafe { &*alloc };
    let options = options.unwrap_or(CompileOptions { base: None, polyfills: None });
    let base = options.base.unwrap_or_default();
    let mut opts = Options { base: &base, ..Options::default() };
    if let Some(polyfills) = options.polyfills {
        opts.polyfills = polyfills as u8;
    }
    if let Some(load) = load_stylesheet {
        opts.load_stylesheet = Some(Box::new(move |id: &str, base: &str| {
            load.call((id.to_owned(), base.to_owned()).into())
                .map(|s| Stylesheet { path: s.path, base: s.base, content: s.content })
                .map_err(|e| e.reason.clone())
        }));
    }
    opts.extension = Some(&Tailwind);
    match core_compiler::compile(arena.alloc_str(&css), arena, opts) {
        Ok(inner) => Ok(Compiler { canonicalizer: ManuallyDrop::new(None), inner: ManuallyDrop::new(Box::new(inner)), alloc }),
        Err(err) => {
            // SAFETY: nothing borrows the arena once compilation failed.
            unsafe { drop(Box::from_raw(alloc)) };
            Err(Error::from_reason(err))
        }
    }
}

#[napi]
impl Compiler {
    /// The CSS for everything seen so far, `null` when it is unchanged since the previous call.
    #[napi]
    pub fn build_changed(&mut self, candidates: Vec<String>) -> Option<String> {
        self.inner.build_changed(candidates.iter().map(String::as_str)).map(str::to_owned)
    }

    #[napi(getter)]
    pub fn features(&self) -> u32 {
        u32::from(self.inner.features)
    }

    /// The `@attributify` options, `null` without one.
    #[napi(getter)]
    pub fn attributify(&self) -> Option<AttributifyOptions> {
        self.inner.design_system.attributify.map(|a| AttributifyOptions { prefix: a.prefix.to_owned(), prefixed_only: a.prefixed_only })
    }

    #[napi(getter)]
    pub fn sources(&self) -> Vec<CompilerSource> {
        self.inner
            .sources
            .iter()
            .map(|s| CompilerSource { base: s.base.to_owned(), pattern: s.pattern.to_owned(), negated: s.negated })
            .collect()
    }

    /// `null` when no `source(…)` was given, the string `"none"` for `source(none)`.
    #[napi(getter, ts_return_type = "null | 'none' | CompilerRoot")]
    pub fn root(&self) -> napi::Either<Option<&'static str>, CompilerRoot> {
        match self.inner.root {
            None => napi::Either::A(None),
            Some(SourceRoot::Disabled) => napi::Either::A(Some("none")),
            Some(SourceRoot::Path { base, pattern }) => {
                napi::Either::B(CompilerRoot { base: base.to_owned(), pattern: pattern.to_owned() })
            }
        }
    }

    #[napi]
    pub fn candidates_to_css(&mut self, classes: Vec<String>) -> Vec<Option<String>> {
        let classes: Vec<&str> = classes.iter().map(String::as_str).collect();
        self.inner.session().candidates_to_css(&classes)
    }

    #[napi]
    pub fn get_class_order(&mut self, classes: Vec<String>) -> Vec<Option<i64>> {
        let classes: Vec<&str> = classes.iter().map(String::as_str).collect();
        self.inner.session().class_order(&classes).into_iter().map(|(_, o)| o.map(|o| o as i64)).collect()
    }

    #[napi]
    pub fn get_class_list(&self) -> Vec<ClassEntry> {
        intellisense::class_list(&self.inner.design_system)
            .into_iter()
            .map(|c| ClassEntry { name: c.name, modifiers: c.modifiers })
            .collect()
    }

    #[napi]
    pub fn get_variants(&self) -> Vec<VariantEntry> {
        intellisense::variants(&self.inner.design_system)
            .into_iter()
            .map(|v| VariantEntry { name: v.name.to_owned(), is_arbitrary: v.is_arbitrary, values: v.values, has_dash: v.has_dash })
            .collect()
    }

    #[napi]
    pub fn get_variant_selectors(&mut self, root: String, value: Option<String>, modifier: Option<String>) -> Vec<String> {
        intellisense::variant_selectors(&mut self.inner.session(), &root, value.as_deref(), modifier.as_deref())
    }

    #[napi]
    pub fn canonicalize_candidates(&mut self, candidates: Vec<String>, options: Option<CanonicalizeOptions>) -> Vec<String> {
        let candidates: Vec<&str> = candidates.iter().map(String::as_str).collect();
        let options = options.map_or_else(orisu_canonical::CanonicalizeOptions::default, |o| orisu_canonical::CanonicalizeOptions {
            rem: o.rem,
            collapse: o.collapse.unwrap_or(false),
            logical_to_physical: o.logical_to_physical.unwrap_or(false),
        });
        let canonicalizer = self.canonicalizer.get_or_insert_with(|| {
            // SAFETY: the compiler is boxed, so the design system stays put until `Compiler::drop`, which drops this first.
            let ds: &'static orisu_compiler::design_system::DesignSystem<'static> =
                unsafe { &*std::ptr::from_ref(&self.inner.design_system) };
            Canonicalizer::new(ds, self.inner.alloc())
        });
        canonicalizer.canonicalize(&candidates, &options)
    }

    /// Prints every parse of each candidate in canonical form.
    #[napi]
    pub fn print_candidates(&mut self, classes: Vec<String>) -> Vec<Vec<String>> {
        let mut ds = self.inner.session();
        classes.iter().map(|c| ds.parse_candidate(c).1.iter().map(|p| print_candidate(p, ds.theme.prefix())).collect()).collect()
    }

    #[napi(getter)]
    pub fn prefix(&self) -> Option<String> {
        self.inner.design_system.theme.prefix().map(str::to_owned)
    }

    #[napi(getter)]
    pub fn important(&self) -> bool {
        self.inner.design_system.important
    }

    #[napi(getter)]
    pub fn invalid_candidates(&self) -> Vec<String> {
        self.inner.design_system.invalid_candidates.iter().map(|c| (*c).to_owned()).collect()
    }

    #[napi]
    pub fn theme_entries(&mut self) -> Vec<ThemeEntry> {
        let ds = self.inner.session();
        ds.theme
            .entries(ds.alloc)
            .map(|(key, value)| ThemeEntry { key: key.to_owned(), value: value.value.to_owned(), options: u32::from(value.options) })
            .collect()
    }

    #[napi]
    pub fn resolve_theme_value(&mut self, path: String, force_inline: Option<bool>) -> Option<String> {
        self.inner.session().resolve_theme_value(&path, force_inline.unwrap_or(true)).map(str::to_owned)
    }

    #[napi]
    pub fn utility_names(&self) -> Vec<String> {
        let utilities = &self.inner.design_system.utilities;
        let mut names: Vec<String> = utilities.keys(UtilityKind::Static).map(str::to_owned).collect();
        names.extend(utilities.keys(UtilityKind::Functional).map(str::to_owned));
        names
    }

    #[napi]
    pub fn variant_names(&self) -> Vec<String> {
        self.inner.design_system.variants.keys().map(str::to_owned).collect()
    }

    #[napi]
    pub fn merge(&mut self, classes: Vec<String>) -> String {
        Merger::new(self.inner.session()).merge(classes.iter().map(String::as_str))
    }

    #[napi]
    pub fn merge_table(&mut self, classes: Vec<String>) -> MergeTable {
        let table = Merger::new(self.inner.session()).table(classes.iter().map(String::as_str));
        let (keys, values) = table.entries.into_iter().unzip();
        MergeTable { prefix: self.inner.design_system.theme.prefix().map(str::to_owned), keys, values, erases: table.erases }
    }
}

/// Lints `files` against the project of the CSS `entry`; one JSON diagnostic per line. With `fix`, rewrites the files with their fixes.
#[napi]
pub fn check(entry: String, files: Vec<String>, fix: Option<bool>) -> Result<String> {
    let files: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
    let diagnostics = orisu_lsp::check(entry.as_ref(), &files, fix.unwrap_or(false)).map_err(Error::from_reason)?;
    let mut out = String::new();
    for diagnostic in &diagnostics {
        out.push_str(&serde_json::to_string(diagnostic).map_err(|e| Error::from_reason(e.to_string()))?);
        out.push('\n');
    }
    Ok(out)
}

/// Serves the language server over this process's stdio until the client shuts it down.
#[napi]
pub fn lsp() -> Result<()> {
    orisu_lsp::run().map_err(Error::from_reason)
}
