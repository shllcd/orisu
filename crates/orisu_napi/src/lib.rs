// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use napi::bindgen_prelude::{Either, FnArgs, Function, Null, Unknown};
use napi::{Env, Error, Result};
use napi_derive::napi;
use orisu_allocator::Allocator;
use orisu_candidate::print::print_candidate;
use orisu_compat_tailwind::{OwnedCompiler, resolve};
use orisu_compiler::compiler::{LoadStylesheet, SourceRoot, Stylesheet};
use orisu_compiler::intellisense;
use orisu_merge::Merger;
use orisu_registry::utilities::UtilityKind;
use orisu_theme::Inline;
use std::path::PathBuf;
use utf16::IndexConverter;

pub mod mangle;
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

impl From<AttributifyOptions> for orisu_candidate::attributify::Attributify {
    fn from(options: AttributifyOptions) -> Self {
        Self { prefix: options.prefix, prefixed_only: options.prefixed_only }
    }
}

impl From<&orisu_candidate::attributify::Attributify> for AttributifyOptions {
    fn from(attributify: &orisu_candidate::attributify::Attributify) -> Self {
        Self { prefix: attributify.prefix.clone(), prefixed_only: attributify.prefixed_only }
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
        if let Some(Either::A(attributify)) = opts.attributify {
            scanner = scanner.with_attributify(attributify.into());
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

    /// Scans like `scan`, returning only the candidates no earlier scan of this scanner returned.
    #[napi]
    pub fn scan_new(&mut self) -> Vec<String> {
        self.scanner.scan_new()
    }

    #[napi]
    pub fn scan_files(&mut self, input: Vec<ChangedContent>) -> Vec<String> {
        self.scanner.scan_content(input.into_iter().map(Into::into).collect())
    }

    #[napi]
    pub fn get_candidates_with_positions(&mut self, input: ChangedContent) -> Result<Vec<CandidateWithPosition>> {
        let (content, extension) = match input.into() {
            orisu_scanner::ChangedContent::File(file, extension) => {
                (std::fs::read_to_string(file).map_err(|e| Error::from_reason(e.to_string()))?, extension)
            }
            orisu_scanner::ChangedContent::Content(content, extension) => (content, extension),
        };
        let mut utf16_idx = IndexConverter::new(&content);
        let candidates = self.scanner.get_candidates_with_positions(orisu_scanner::ChangedContent::Content(content.clone(), extension));
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
    pub names: Option<mangle::NameTable>,
}

#[napi]
pub struct Compiler(OwnedCompiler);

#[napi]
pub fn expand_variant_groups(code: String) -> Option<String> {
    orisu_candidate::group::expand_in_strings(&code)
}

type LoadFn<'f> = Function<'f, FnArgs<(String, String)>, LoadedStylesheet>;

/// Throws errors whose `code` is a diagnostic code such as `INVALID_STYLESHEET`, or `InvalidArg` for `polyfills` beyond 8 bits.
#[napi]
pub fn compile(css: String, options: Option<CompileOptions>, load_stylesheet: Option<LoadFn<'_>>) -> Result<Compiler, &'static str> {
    let options = options.unwrap_or(CompileOptions { base: None, polyfills: None });
    let load = load_stylesheet.map(|load| -> Box<LoadStylesheet<'_>> {
        Box::new(move |id: &str, base: &str| {
            load.call((id.to_owned(), base.to_owned()).into())
                .map(|s| Stylesheet { path: s.path, base: s.base, content: s.content })
                .map_err(|e| e.reason.clone())
        })
    });
    let polyfills =
        options.polyfills.map(u8::try_from).transpose().map_err(|_| Error::new("InvalidArg", "`polyfills` must fit in 8 bits"))?;
    OwnedCompiler::compile(&css, &options.base.unwrap_or_default(), polyfills, load)
        .map(Compiler)
        .map_err(|error| Error::new(error.code.name(), error.message))
}

/// The file an `@import` of `id` from the directory `base` loads, `null` when none.
#[napi]
pub fn resolve_css(id: String, base: String) -> Option<String> {
    resolve::resolve_css(&id, base.as_ref()).map(|path| path.to_string_lossy().into_owned())
}

#[napi]
impl Compiler {
    /// The CSS for everything seen so far, `null` when it is unchanged since the previous call.
    #[napi]
    pub fn build_changed(&mut self, candidates: Vec<String>) -> Option<String> {
        self.0.with_mut(|c| c.build_changed(candidates.iter().map(String::as_str)).map(str::to_owned))
    }

    #[napi(getter)]
    pub fn features(&self) -> u32 {
        self.0.with(|c| u32::from(c.features))
    }

    /// The `@attributify` options, `null` without one.
    #[napi(getter)]
    pub fn attributify(&self) -> Option<AttributifyOptions> {
        self.0.with(|c| c.design_system.attributify.as_ref().map(Into::into))
    }

    #[napi(getter)]
    pub fn sources(&self) -> Vec<SourceEntry> {
        self.0.with(|c| {
            c.sources.iter().map(|s| SourceEntry { base: s.base.to_owned(), pattern: s.pattern.to_owned(), negated: s.negated }).collect()
        })
    }

    /// `null` when no `source(…)` was given, the string `"none"` for `source(none)`.
    #[napi(getter, ts_return_type = "null | 'none' | GlobEntry")]
    pub fn root(&self) -> napi::Either<Option<&'static str>, GlobEntry> {
        self.0.with(|c| match c.root {
            None => napi::Either::A(None),
            Some(SourceRoot::Disabled) => napi::Either::A(Some("none")),
            Some(SourceRoot::Path { base, pattern }) => napi::Either::B(GlobEntry { base: base.to_owned(), pattern: pattern.to_owned() }),
        })
    }

    #[napi]
    pub fn candidates_to_css(&mut self, classes: Vec<String>) -> Vec<Option<String>> {
        let classes: Vec<&str> = classes.iter().map(String::as_str).collect();
        self.0.with_mut(|c| c.session().candidates_to_css(&classes))
    }

    #[napi]
    pub fn get_class_order(&mut self, classes: Vec<String>) -> Vec<Option<i64>> {
        let classes: Vec<&str> = classes.iter().map(String::as_str).collect();
        self.0.with_mut(|c| c.session().class_order(&classes).into_iter().map(|(_, o)| o.map(|o| o as i64)).collect())
    }

    #[napi]
    pub fn get_class_list(&self) -> Vec<ClassEntry> {
        self.0.with(|c| {
            intellisense::class_list(&c.design_system).into_iter().map(|c| ClassEntry { name: c.name, modifiers: c.modifiers }).collect()
        })
    }

    #[napi]
    pub fn get_variants(&self) -> Vec<VariantEntry> {
        self.0.with(|c| {
            intellisense::variants(&c.design_system)
                .into_iter()
                .map(|v| VariantEntry { name: v.name.to_owned(), is_arbitrary: v.is_arbitrary, values: v.values, has_dash: v.has_dash })
                .collect()
        })
    }

    #[napi]
    pub fn get_variant_selectors(&mut self, root: String, value: Option<String>, modifier: Option<String>) -> Vec<String> {
        self.0.with_mut(|c| intellisense::variant_selectors(&mut c.session(), &root, value.as_deref(), modifier.as_deref()))
    }

    #[napi]
    pub fn canonicalize_candidates(&mut self, candidates: Vec<String>, options: Option<CanonicalizeOptions>) -> Vec<String> {
        let candidates: Vec<&str> = candidates.iter().map(String::as_str).collect();
        let options = options.map_or_else(orisu_canonical::CanonicalizeOptions::default, |o| orisu_canonical::CanonicalizeOptions {
            rem: o.rem,
            collapse: o.collapse.unwrap_or(false),
            logical_to_physical: o.logical_to_physical.unwrap_or(false),
        });
        self.0.canonicalize(&candidates, &options)
    }

    /// Prints every parse of each candidate in canonical form.
    #[napi]
    pub fn print_candidates(&mut self, classes: Vec<String>) -> Vec<Vec<String>> {
        self.0.with_mut(|c| {
            let mut ds = c.session();
            let scratch = Allocator::new();
            classes
                .iter()
                .map(|c| ds.parse_candidate(c).1.iter().map(|p| print_candidate(p, ds.theme.prefix(), &scratch)).collect())
                .collect()
        })
    }

    #[napi(getter)]
    pub fn prefix(&self) -> Option<String> {
        self.0.with(|c| c.design_system.theme.prefix().map(str::to_owned))
    }

    #[napi(getter)]
    pub fn important(&self) -> bool {
        self.0.with(|c| c.design_system.important)
    }

    #[napi(getter)]
    pub fn invalid_candidates(&self) -> Vec<String> {
        self.0.with(|c| c.design_system.invalid_candidates.iter().map(|c| (*c).to_owned()).collect())
    }

    #[napi]
    pub fn theme_entries(&mut self) -> Vec<ThemeEntry> {
        self.0.with_mut(|c| {
            let ds = c.session();
            ds.theme
                .entries(ds.alloc)
                .map(|(key, value)| ThemeEntry { key: key.to_owned(), value: value.value.to_owned(), options: u32::from(value.options) })
                .collect()
        })
    }

    #[napi]
    pub fn resolve_theme_value(&mut self, path: String, force_inline: Option<bool>) -> Option<String> {
        self.0.with_mut(|c| {
            c.session()
                .resolve_theme_value(&path, if force_inline.unwrap_or(true) { Inline::Always } else { Inline::AsDeclared })
                .map(str::to_owned)
        })
    }

    #[napi]
    pub fn utility_names(&self) -> Vec<String> {
        self.0.with(|c| {
            let utilities = &c.design_system.utilities;
            let mut names: Vec<String> = utilities.keys(UtilityKind::Static).map(str::to_owned).collect();
            names.extend(utilities.keys(UtilityKind::Functional).map(str::to_owned));
            names
        })
    }

    #[napi]
    pub fn variant_names(&self) -> Vec<String> {
        self.0.with(|c| c.design_system.variants.keys().map(str::to_owned).collect())
    }

    #[napi]
    pub fn merge(&mut self, classes: Vec<String>) -> String {
        self.0.with_mut(|c| Merger::new(c.session()).merge(classes.iter().map(String::as_str)))
    }

    #[napi]
    pub fn merge_table(&mut self, classes: Vec<String>) -> MergeTable {
        self.0.with_mut(|c| {
            let orisu_merge::MergeTable { keys, values, erases } = Merger::new(c.session()).table(classes.iter().map(String::as_str)).0;
            MergeTable { prefix: c.design_system.theme.prefix().map(str::to_owned), keys, values, erases, names: None }
        })
    }
}

#[napi(ts_return_type = "Diagnostic[]")]
pub fn check(env: &Env, entry: String, files: Vec<String>, fix: Option<bool>) -> Result<Unknown<'_>> {
    let files: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
    let diagnostics = orisu_lsp::check(entry.as_ref(), &files, fix.unwrap_or(false)).map_err(Error::from_reason)?;
    env.to_js_value(&diagnostics)
}

#[napi]
pub fn lsp() -> Result<()> {
    orisu_lsp::run().map_err(Error::from_reason)
}
