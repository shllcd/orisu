// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::compile::{self, CompiledRule, compile_candidates};
use crate::css_functions::substitute_functions;
use crate::extension::Extension;
use indexmap::IndexMap;
use orisu_allocator::{AllocExt, Allocator};
use orisu_ast::{Nodes, to_css};
use orisu_candidate::{self, Candidate, CandidateKind, Registry, Variant, VariantKind};
use orisu_optimizer::{optimize_ast, polyfills};
use orisu_registry::utilities::{self, Cx, Declared, Utilities, UtilityKind};
use orisu_registry::variants::{self, Variants};
use orisu_theme::{Theme, UsedVars, opts};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::ops::Deref;

const IGNORE_INLINE_KEY: u8 = 1 << 7;

pub type ExtensionRef<'a> = &'a (dyn Extension<'a> + Sync + 'a);

pub struct DesignSystem<'a> {
    pub theme: Theme<'a>,
    pub utilities: Utilities<'a>,
    pub variants: Variants<'a>,
    pub invalid_candidates: FxHashSet<&'a str>,
    pub important: bool,
    pub attributify: Option<crate::attributify::Attributify<'a>>,
    extension: Option<ExtensionRef<'a>>,
}

const _: () = {
    const fn assert_sync<T: Sync>() {}
    assert_sync::<DesignSystem<'static>>();
};

impl<'a> DesignSystem<'a> {
    pub fn new(alloc: &'a Allocator, theme: Theme<'a>, extension: Option<ExtensionRef<'a>>) -> Self {
        let mut utilities = Utilities::default();
        utilities::register_builtin(&mut utilities, &theme, alloc);
        if let Some(extension) = extension {
            extension.register_utilities(&mut utilities, &theme, alloc);
        }
        let mut variants = Variants::default();
        variants::register_builtin(&mut variants, &theme, alloc);
        Self { theme, utilities, variants, invalid_candidates: FxHashSet::default(), important: false, attributify: None, extension }
    }

    pub fn extension(&self) -> Option<ExtensionRef<'a>> {
        self.extension
    }
}

/// Parse and compile results memoized for one [`DesignSystem`]; every entry lives in the session's arena.
#[derive(Default)]
pub struct Caches<'a> {
    parsed_variants: IndexMap<&'a str, Option<&'a Variant<'a>>, FxBuildHasher>,
    parsed_candidates: FxHashMap<&'a str, &'a [Candidate<'a>]>,
    compiled: FxHashMap<(Candidate<'a>, u8), &'a [CompiledRule<'a>]>,
}

/// A design system with the arena and caches of one compiler, project or canonicalizer.
pub struct Session<'s, 'a> {
    pub ds: &'s DesignSystem<'a>,
    pub alloc: &'a Allocator,
    /// [`opts`] flags applied to every theme lookup, such as [`opts::IGNORE_INLINE`] while canonicalizing.
    pub options: u8,
    caches: &'s mut Caches<'a>,
}

impl<'a> Deref for Session<'_, 'a> {
    type Target = DesignSystem<'a>;

    fn deref(&self) -> &DesignSystem<'a> {
        self.ds
    }
}

impl<'s, 'a> Session<'s, 'a> {
    pub fn new(ds: &'s DesignSystem<'a>, alloc: &'a Allocator, caches: &'s mut Caches<'a>) -> Self {
        Self { ds, alloc, options: opts::NONE, caches }
    }

    #[inline]
    pub fn cx(&self) -> Cx<'a, 's> {
        Cx { theme: &self.ds.theme, alloc: self.alloc, options: self.options }
    }

    /// Returns the arena-interned raw string together with its parsed candidates.
    pub fn parse_candidate(&mut self, raw: &str) -> (&'a str, &'a [Candidate<'a>]) {
        if let Some((&raw, &candidates)) = self.caches.parsed_candidates.get_key_value(raw) {
            return (raw, candidates);
        }
        let raw = self.alloc.str(raw);
        let candidates: &'a [Candidate<'a>] = self.alloc.alloc_slice_copy(&orisu_candidate::parse_candidate(raw, self));
        self.caches.parsed_candidates.insert(raw, candidates);
        (raw, candidates)
    }

    pub fn compile_ast_nodes(&mut self, candidate: &Candidate<'a>, flags: u8) -> &'a [CompiledRule<'a>] {
        let key = (*candidate, if self.options & opts::IGNORE_INLINE != 0 { flags | IGNORE_INLINE_KEY } else { flags });
        if let Some(&rules) = self.caches.compiled.get(&key) {
            return rules;
        }
        let mut rules = compile::compile_ast_nodes(candidate, self, flags);
        for rule in &mut rules {
            if substitute_functions(std::slice::from_mut(&mut rule.node), self).is_err() {
                rules.clear();
                break;
            }
        }
        let rules: &'a [CompiledRule<'a>] = self.alloc.alloc_slice_fill_iter(rules);
        self.caches.compiled.insert(key, rules);
        rules
    }

    /// The declarations a class sets, computed from the utility data without compiling it; `None` when it needs a handler.
    pub fn declarations(&mut self, class: &str) -> Option<std::vec::Vec<(&'a str, &'a str)>> {
        let (_, candidates) = self.parse_candidate(class);
        let mut out = std::vec::Vec::new();
        let mut found_any = false;
        for candidate in candidates {
            if !candidate.variants.is_empty() || candidate.modifier().is_some() {
                return None;
            }
            let kind = match candidate.kind {
                CandidateKind::Arbitrary { .. } => return None,
                CandidateKind::Static { .. } => UtilityKind::Static,
                CandidateKind::Functional { .. } => UtilityKind::Functional,
            };
            let root = match candidate.kind {
                CandidateKind::Static { root } | CandidateKind::Functional { root, .. } => root,
                CandidateKind::Arbitrary { .. } => unreachable!(),
            };
            let mut found = false;
            for fallback in [false, true] {
                for utility in self.utilities.get(root) {
                    if utility.kind != kind || utility.is_fallback() != fallback {
                        continue;
                    }
                    match utility.declarations(&self.cx(), candidate) {
                        Declared::Yes(declarations) => {
                            out.extend(declarations);
                            found = true;
                        }
                        Declared::No => {}
                        Declared::Unknown => return None,
                    }
                }
                if found {
                    break;
                }
            }
            found_any |= found;
        }
        found_any.then_some(out)
    }

    pub fn resolve_theme_value(&self, path: &str, force_inline: bool) -> Option<&'a str> {
        if !path.starts_with("--")
            && let Some(extension) = self.extension
        {
            return extension.resolve_theme(self, path);
        }
        let (path, modifier) = match path.rfind('/') {
            Some(i) => (path[..i].trim(), Some(path[i + 1..].trim())),
            None => (path, None),
        };
        let value = self.theme.resolve(None, &[path], if force_inline { opts::INLINE } else { opts::NONE })?;
        match modifier {
            Some(modifier) if !modifier.is_empty() => Some(self.cx().with_alpha(value, self.alloc.str(modifier))),
            _ => Some(value),
        }
    }

    /// Position of each class in Tailwind's output order; `None` for non-Tailwind classes.
    pub fn class_order<'c>(&mut self, classes: &[&'c str]) -> std::vec::Vec<(&'c str, Option<u64>)> {
        let entries = compile::compile_entries(classes.iter().copied(), self, true, |_| {});
        let mut sorted: FxHashMap<&str, u64> = FxHashMap::default();
        let mut idx = 0;
        for entry in &entries {
            sorted.entry(entry.sorting.candidate).or_insert_with(|| {
                idx += 1;
                idx - 1
            });
        }
        classes.iter().map(|class| (*class, sorted.get(class).copied())).collect()
    }

    pub fn candidates_to_ast(&mut self, classes: &[&str]) -> std::vec::Vec<Nodes<'a>> {
        classes
            .iter()
            .map(|class| {
                let mut valid = true;
                let compiled = compile_candidates([*class], self, true, |_| valid = false);
                let nodes = optimize_ast(&compiled.ast_nodes, &self.ds.theme, &mut UsedVars::default(), polyfills::NONE, self.alloc);
                if valid { nodes } else { orisu_allocator::Vec::new_in(self.alloc) }
            })
            .collect()
    }

    pub fn candidates_to_css(&mut self, classes: &[&str]) -> std::vec::Vec<Option<String>> {
        self.candidates_to_ast(classes).into_iter().map(|nodes| (!nodes.is_empty()).then(|| to_css(&nodes))).collect()
    }
}

impl<'a> Registry<'a> for Session<'_, 'a> {
    #[inline]
    fn alloc(&self) -> &'a Allocator {
        self.alloc
    }

    #[inline]
    fn prefix(&self) -> Option<&str> {
        self.ds.theme.prefix()
    }

    #[inline]
    fn has_static_utility(&self, root: &str) -> bool {
        self.utilities.has(root, UtilityKind::Static)
    }

    #[inline]
    fn has_functional_utility(&self, root: &str) -> bool {
        self.utilities.has(root, UtilityKind::Functional)
    }

    #[inline]
    fn variant_kind(&self, root: &str) -> Option<VariantKind> {
        self.variants.kind(root)
    }

    #[inline]
    fn compounds_with(&self, root: &str, variant: &Variant<'a>) -> bool {
        self.variants.compounds_with(root, variant)
    }

    fn parse_variant(&mut self, raw: &'a str) -> Option<&'a Variant<'a>> {
        if let Some(&parsed) = self.caches.parsed_variants.get(raw) {
            return parsed;
        }
        let parsed = orisu_candidate::parse_variant(raw, self).map(|v| &*self.alloc.alloc(v));
        self.caches.parsed_variants.insert(raw, parsed);
        parsed
    }
}
