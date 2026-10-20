// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

mod convert;
mod fold;
mod signature;
mod utility;
mod variant;

use fold::{constant_fold_declaration, locale_compare};
use indexmap::{IndexMap, IndexSet};
use orisu_allocator::{AllocExt, Allocator, Vec};
use orisu_ast::{Node, Nodes};
use orisu_candidate::print::{print_candidate_with, print_variant};
use orisu_candidate::{Candidate, CandidateKind, Variant, VariantKind};
use orisu_compiler::compile::flags;
use orisu_compiler::design_system::{Caches, DesignSystem, Session};
use orisu_compiler::intellisense::{class_list_keeping, class_list_unsorted};
use orisu_parser::compare::dimension;
use orisu_parser::infer::{format_js_number, is_multiple_of_quarter};
use orisu_parser::segment::segment;
use orisu_registry::utilities::UtilityKind;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet, FxHasher};
use std::borrow::Cow;
use std::sync::Arc;
use utility::{UTILITY_PASSES, is_valid_spacing_number};
use variant::VARIANT_PASSES;

#[derive(Debug, Clone, Copy, Default)]
pub struct CanonicalizeOptions {
    /// Root font size in pixels; `rem` values are compared as `px` when set.
    pub rem: Option<f64>,
    pub collapse: bool,
    pub logical_to_physical: bool,
}

pub(crate) mod features {
    pub const EXPAND_PROPERTIES: u8 = 1 << 0;
    pub const LOGICAL_TO_PHYSICAL: u8 = 1 << 1;
}

#[derive(Clone, Copy)]
pub(crate) struct SigOpts {
    pub rem: Option<f64>,
    pub features: u8,
}

type SigKey = (Option<u64>, u8);
type CanonKey = (SigKey, bool);
type Set<T> = IndexSet<T, FxBuildHasher>;
type Map<K, V> = IndexMap<K, V, FxBuildHasher>;
type PropertyValues = Map<String, Set<String>>;
type Lookup<'a> = FxHashMap<Signature, std::vec::Vec<&'a str>>;
type StaticUtilities = Map<String, Map<String, Set<String>>>;
type SpacingKey = (Option<u64>, String);

/// Identity of a canonical signature: a 128-bit hash of its canonical CSS.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Signature(u128);

impl Signature {
    fn is_valid(self) -> bool {
        self.0 >> 127 == 0
    }
}

fn hash_signature(css: &str) -> Signature {
    use std::hash::{Hash, Hasher};
    let mut first = FxHasher::default();
    0u8.hash(&mut first);
    css.hash(&mut first);
    let mut second = FxHasher::default();
    1u8.hash(&mut second);
    css.as_bytes().hash(&mut second);
    let hash = (((first.finish() as u128) << 64) | second.finish() as u128) & ((1u128 << 127) - 1);
    Signature(if hash == 0 { 1 } else { hash })
}

#[derive(PartialEq, Eq, Hash)]
struct UtilityKey<'a> {
    sig: SigKey,
    utility: &'a str,
}

#[derive(PartialEq, Eq, Hash)]
struct ValueKey<'a> {
    sig: SigKey,
    inline: bool,
    value: &'a str,
}

#[derive(Default)]
struct Storage<'a> {
    sentinels: usize,
    utility_signatures: FxHashMap<UtilityKey<'a>, Signature>,
    variant_signatures: FxHashMap<String, Signature>,
    precomputed_utilities: FxHashMap<SigKey, Lookup<'a>>,
    precomputed_variants: Option<Lookup<'a>>,
    static_utilities: FxHashMap<SigKey, StaticUtilities>,
    recorded_statics: FxHashSet<SigKey>,
    utility_properties: FxHashMap<(SigKey, String), Arc<PropertyValues>>,
    signature_properties: FxHashMap<Signature, Arc<[String]>>,
    property_roots: FxHashMap<SigKey, FxHashMap<String, std::vec::Vec<&'a str>>>,
    property_seen: FxHashMap<SigKey, FxHashSet<String>>,
    spacing: FxHashMap<SpacingKey, Option<f64>>,
    normalized_values: FxHashMap<ValueKey<'a>, String>,
    candidates: FxHashMap<(CanonKey, &'a str), &'a str>,
    variants: FxHashMap<Variant<'a>, &'a [&'a Variant<'a>]>,
    scratch: Allocator,
    print_scratch: Allocator,
}

pub(crate) struct Canon<'a, 'd> {
    pub ds: Session<'d, 'a>,
    storage: &'d mut Storage<'a>,
    pub sig: SigOpts,
    collapse: bool,
}

/// Canonicalizes candidates against one design system, caching per option set so results never depend on call order.
pub struct Canonicalizer<'a, 'd> {
    ds: &'d DesignSystem<'a>,
    alloc: &'a Allocator,
    caches: Caches<'a>,
    storage: Storage<'a>,
}

impl<'a, 'd> Canonicalizer<'a, 'd> {
    pub fn new(ds: &'d DesignSystem<'a>, alloc: &'a Allocator) -> Self {
        Self { ds, alloc, caches: Caches::default(), storage: Storage::default() }
    }

    pub fn canonicalize(&mut self, candidates: &[&str], options: &CanonicalizeOptions) -> std::vec::Vec<String> {
        let mut features = 0;
        if options.collapse {
            features |= features::EXPAND_PROPERTIES;
        }
        if options.logical_to_physical {
            features |= features::LOGICAL_TO_PHYSICAL;
        }
        let sig = SigOpts { rem: options.rem, features };
        let mut cx =
            Canon { ds: Session::new(self.ds, self.alloc, &mut self.caches), storage: &mut self.storage, sig, collapse: options.collapse };
        let result: Set<&str> = candidates.iter().map(|c| cx.canonicalize_candidate(c)).collect();
        if result.len() <= 1 || !options.collapse {
            return result.into_iter().map(str::to_owned).collect();
        }
        cx.collapse_candidates(result.into_iter().map(str::to_owned).collect())
    }
}

const MAX_BARE_VALUE_IN_PX: f64 = 1536.0;
const MAX_BARE_VALUE_IN_REM: f64 = MAX_BARE_VALUE_IN_PX / 16.0;

/// Arbitrary values whose data types cover the property branches a functional root can take.
const ROOT_VALUE_PROBES: [&str; 16] = [
    "4px",
    "50%",
    "red",
    "#0f0",
    "0.5",
    "4",
    "1/2",
    "url(x)",
    "0deg",
    "solid",
    "center",
    "cover",
    "linear-gradient(red,blue)",
    "var(--x)",
    "calc(1px+1px)",
    "auto",
];

#[derive(PartialEq)]
enum LineHeight<'s> {
    Bare(f64),
    Raw(&'s str),
}

impl<'a, 'd> Canon<'a, 'd> {
    fn sig_key(&self) -> SigKey {
        (self.sig.rem.map(f64::to_bits), self.sig.features)
    }

    fn canon_key(&self) -> CanonKey {
        (self.sig_key(), self.collapse)
    }

    pub(crate) fn prefix(&self) -> Option<&'a str> {
        self.ds.theme.prefix().filter(|p| !p.is_empty())
    }

    pub(crate) fn without_inline<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let previous = self.ds.options;
        self.ds.options |= orisu_theme::opts::IGNORE_INLINE;
        let result = f(self);
        self.ds.options = previous;
        result
    }

    pub(crate) fn print(&mut self, candidate: &Candidate<'_>) -> String {
        let mut scratch = std::mem::take(&mut self.storage.print_scratch);
        let printed = print_candidate_with(candidate, self.ds.theme.prefix(), &scratch);
        scratch.reset();
        self.storage.print_scratch = scratch;
        printed
    }

    pub(crate) fn print_unprefixed(&mut self, candidate: &Candidate<'_>) -> String {
        let printed = self.print(candidate);
        match self.prefix() {
            Some(prefix) => match printed.strip_prefix(prefix).and_then(|r| r.strip_prefix(':')) {
                Some(rest) => rest.to_owned(),
                None => printed,
            },
            None => printed,
        }
    }

    /// Parses `input`, adding the theme prefix when it is missing.
    pub(crate) fn parse(&mut self, input: &str) -> &'a [Candidate<'a>] {
        match self.prefix() {
            Some(prefix) if !input.strip_prefix(prefix).is_some_and(|r| r.starts_with(':')) => {
                self.ds.parse_candidate(&format!("{prefix}:{input}")).1
            }
            _ => self.ds.parse_candidate(input).1,
        }
    }

    /// Runs `f` with an empty arena for data that does not outlive it.
    pub(crate) fn with_scratch<T>(&mut self, f: impl FnOnce(&mut Self, &Allocator) -> T) -> T {
        let mut scratch = std::mem::take(&mut self.storage.scratch);
        let result = f(self, &scratch);
        scratch.reset();
        self.storage.scratch = scratch;
        result
    }

    /// A signature equal only to itself, standing in for an invalid utility.
    fn sentinel(&mut self) -> Signature {
        let n = self.storage.sentinels;
        self.storage.sentinels += 1;
        Signature((1u128 << 127) | n as u128)
    }

    pub(crate) fn utility_signature(&mut self, utility: &str) -> Signature {
        let sig = self.sig_key();
        if let Some(&signature) = self.storage.utility_signatures.get(&UtilityKey { sig, utility }) {
            return signature;
        }
        let signature = match self.compute_utility_signature(utility) {
            Some((signature, properties)) => {
                self.storage.signature_properties.insert(signature, Arc::from(properties));
                signature
            }
            None => self.sentinel(),
        };
        let utility = self.ds.alloc.str(utility);
        self.storage.utility_signatures.insert(UtilityKey { sig, utility }, signature);
        signature
    }

    /// Records every property a static class sets, so that plain properties only statics produce are not searched.
    fn prefill_static_properties(&mut self, key: SigKey) {
        let mut properties = std::vec::Vec::new();
        for root in self.ds.ds.utilities.keys(UtilityKind::Static) {
            if let Some(found) = self.compute_utility_properties(root) {
                properties.extend(found);
            }
        }
        self.storage.property_seen.entry(key).or_default().extend(properties);
    }

    /// Records the properties of every functional root reachable only through an arbitrary value of each data type.
    fn prefill_root_properties(&mut self, key: SigKey) {
        let roots: std::vec::Vec<&'a str> = self.ds.ds.utilities.keys(UtilityKind::Functional).collect();
        let (ds, options, sig, collapse) = (self.ds.ds, self.ds.options, self.sig, self.collapse);
        // SAFETY: `DesignSystem` has no interior mutability and is only read through `&`, so nothing allocated in the
        // shorter-lived worker arena can be stored into it; this only narrows the lifetime as covariance would.
        let ds = unsafe { std::mem::transmute::<&DesignSystem<'a>, &DesignSystem<'_>>(ds) };
        let results: std::vec::Vec<std::vec::Vec<Option<std::vec::Vec<String>>>> = roots
            .par_chunks(32)
            .map(|chunk| {
                let alloc = Allocator::default();
                let mut caches = Caches::default();
                let mut storage = Storage::default();
                // SAFETY: `DesignSystem` has no interior mutability and is only read through `&`, so nothing allocated in the
                // shorter-lived worker arena can be stored into it; this only narrows the lifetime as covariance would.
                let ds = unsafe { std::mem::transmute::<&DesignSystem<'a>, &DesignSystem<'_>>(ds) };
                let mut session = Session::new(ds, &alloc, &mut caches);
                session.options = options;
                let mut cx = Canon { ds: session, storage: &mut storage, sig, collapse };
                let mut properties = std::vec::Vec::new();
                for root in chunk {
                    for value in ROOT_VALUE_PROBES {
                        properties.push(cx.compute_utility_properties(&format!("{root}-[{value}]")));
                    }
                }
                properties
            })
            .collect();
        let index = self.storage.property_roots.entry(key).or_default();
        let seen = self.storage.property_seen.entry(key).or_default();
        for (chunk, properties) in roots.chunks(32).zip(results) {
            for (i, properties) in properties.into_iter().enumerate() {
                let Some(properties) = properties else {
                    continue;
                };
                let root = chunk[i / ROOT_VALUE_PROBES.len()];
                for property in properties {
                    seen.insert(property.clone());
                    let list = index.entry(property).or_default();
                    if !list.contains(&root) {
                        list.push(root);
                    }
                }
            }
        }
    }

    pub(crate) fn variant_signature(&mut self, variant: &str) -> Signature {
        if let Some(&signature) = self.storage.variant_signatures.get(variant) {
            return signature;
        }
        let signature = match self.compute_variant_signature(variant) {
            Some(signature) => signature,
            None => self.sentinel(),
        };
        self.storage.variant_signatures.insert(variant.to_owned(), signature);
        signature
    }

    pub(crate) fn same_signature(&mut self, a: &str, b: &str) -> bool {
        let a = self.utility_signature(a);
        a.is_valid() && {
            let b = self.utility_signature(b);
            b.is_valid() && a == b
        }
    }

    pub(crate) fn same_candidate_signature(&mut self, a: &Candidate<'_>, b: &Candidate<'_>) -> bool {
        let (a, b) = (self.print(a), self.print(b));
        self.same_signature(&a, &b)
    }

    fn spacing_multiplier(&self) -> Option<(f64, Option<String>)> {
        let multiplier = self.ds.resolve_theme_value("--spacing", true)?;
        let folded = constant_fold_declaration(multiplier, self.sig.rem, self.ds.alloc);
        dimension(&folded).map(|(value, unit)| (value, unit.map(str::to_owned)))
    }

    /// `input` as a multiple of `--spacing`, when both share a unit.
    pub(crate) fn spacing(&mut self, input: &str) -> Option<f64> {
        let key = (self.sig.rem.map(f64::to_bits), input.to_owned());
        if let Some(&cached) = self.storage.spacing.get(&key) {
            return cached;
        }
        let result = self.spacing_multiplier().and_then(|(value, unit)| {
            if value == 0.0 {
                return None;
            }
            let folded = constant_fold_declaration(input, self.sig.rem, self.ds.alloc);
            let (mine, my_unit) = dimension(&folded)?;
            (my_unit == unit.as_deref()).then(|| mine / value)
        });
        self.storage.spacing.insert(key, result);
        result
    }

    pub(crate) fn is_reasonable_bare_value(&self, value: f64) -> bool {
        let Some((spacing, unit)) = self.spacing_multiplier() else {
            return false;
        };
        let pixels = value * spacing;
        match unit.as_deref() {
            Some("px") => pixels <= MAX_BARE_VALUE_IN_PX,
            Some("rem") => pixels <= MAX_BARE_VALUE_IN_REM,
            _ => false,
        }
    }

    pub(crate) fn precomputed_utilities(&mut self) -> &Lookup<'a> {
        let key = self.sig_key();
        if !self.storage.precomputed_utilities.contains_key(&key) {
            let lookup = self.compute_precomputed_utilities(key);
            self.storage.precomputed_utilities.insert(key, lookup);
        }
        self.storage.precomputed_utilities.entry(key).or_default()
    }

    fn compute_precomputed_utilities(&mut self, key: SigKey) -> Lookup<'a> {
        let alloc = self.ds.alloc;
        let mut lookup = Lookup::default();
        let entries = class_list_unsorted(self.ds.ds, |modifier| !is_multiple_of_quarter(modifier));
        self.prefill_utility_signatures(
            entries
                .iter()
                .flat_map(|entry| {
                    let positive = entry.name.strip_prefix('-').filter(|_| entry.name.ends_with("-0"));
                    [Some(entry.name.as_str()), positive].into_iter().flatten()
                })
                .collect(),
        );
        let with_modifiers =
            entries.iter().flat_map(|entry| entry.modifiers.iter().map(|modifier| format!("{}/{modifier}", entry.name))).collect();
        self.prefill_utility_signatures(with_modifiers);
        for entry in entries {
            let name = entry.name.as_str();
            let signature = self.utility_signature(name);
            if !signature.is_valid() {
                continue;
            }
            if let Some(positive) = name.strip_prefix('-')
                && name.ends_with("-0")
                && self.utility_signature(positive) == signature
            {
                continue;
            }
            let name = alloc.str(name);
            lookup.entry(signature).or_default().push(name);
            for modifier in &entry.modifiers {
                let with_modifier = format!("{name}/{modifier}");
                let signature = self.utility_signature(&with_modifier);
                if !signature.is_valid() {
                    continue;
                }
                let with_modifier = alloc.str(&with_modifier);
                lookup.entry(signature).or_default().push(with_modifier);
            }
        }
        self.prefill_root_properties(key);
        self.prefill_static_properties(key);
        lookup
    }

    /// Computes the missing signatures of `utilities` in parallel, each worker in its own arena.
    fn prefill_utility_signatures<S: AsRef<str> + Sync>(&mut self, utilities: std::vec::Vec<S>) {
        let key = self.sig_key();
        let missing: std::vec::Vec<&str> = utilities.iter().map(AsRef::as_ref).collect();
        let (ds, options, sig, collapse) = (self.ds.ds, self.ds.options, self.sig, self.collapse);
        // SAFETY: `DesignSystem` has no interior mutability and is only read through `&`, so nothing allocated in the
        // shorter-lived worker arena can be stored into it; this only narrows the lifetime as covariance would.
        let ds = unsafe { std::mem::transmute::<&DesignSystem<'a>, &DesignSystem<'_>>(ds) };
        let signatures: std::vec::Vec<Option<Signature>> = missing
            .par_chunks(1024)
            .flat_map_iter(|chunk| {
                let alloc = Allocator::default();
                let mut caches = Caches::default();
                let mut storage = Storage::default();
                // SAFETY: `DesignSystem` has no interior mutability and is only read through `&`, so nothing allocated in the
                // shorter-lived worker arena can be stored into it; this only narrows the lifetime as covariance would.
                let ds = unsafe { std::mem::transmute::<&DesignSystem<'a>, &DesignSystem<'_>>(ds) };
                let mut session = Session::new(ds, &alloc, &mut caches);
                session.options = options;
                let mut cx = Canon { ds: session, storage: &mut storage, sig, collapse };
                chunk.iter().map(|utility| cx.compute_utility_css(utility)).collect::<std::vec::Vec<_>>()
            })
            .collect();
        for (utility, signature) in missing.into_iter().zip(signatures) {
            let signature = signature.unwrap_or_else(|| self.sentinel());
            let utility = self.ds.alloc.str(utility);
            self.storage.utility_signatures.insert(UtilityKey { sig: key, utility }, signature);
        }
    }

    pub(crate) fn precomputed_variants(&mut self) -> &Lookup<'a> {
        if self.storage.precomputed_variants.is_none() {
            let mut lookup = Lookup::default();
            for (root, entry) in self.ds.ds.variants.entries() {
                if entry.kind != VariantKind::Static {
                    continue;
                }
                let signature = self.variant_signature(root);
                if signature.is_valid() {
                    lookup.entry(signature).or_default().push(root);
                }
            }
            self.storage.precomputed_variants = Some(lookup);
        }
        self.storage.precomputed_variants.get_or_insert_default()
    }

    /// Records the precomputed utilities in the static utility lookup, which only collapsing reads.
    fn record_statics(&mut self) {
        let key = self.sig_key();
        if !self.storage.recorded_statics.insert(key) {
            return;
        }
        for class in self.collect_unrecorded() {
            self.utility_properties(class);
        }
    }

    /// The classes collapsing reads, in class-list order; built only when collapsing.
    fn collect_unrecorded(&mut self) -> std::vec::Vec<&'a str> {
        let alloc = self.ds.alloc;
        let mut unrecorded = std::vec::Vec::new();
        for entry in class_list_keeping(self.ds.ds, |modifier| !is_multiple_of_quarter(modifier)) {
            let name = entry.name.as_str();
            let signature = self.utility_signature(name);
            if !signature.is_valid() {
                continue;
            }
            if let Some(positive) = name.strip_prefix('-')
                && name.ends_with("-0")
                && self.utility_signature(positive) == signature
            {
                continue;
            }
            let name = alloc.str(name);
            unrecorded.push(name);
            for modifier in &entry.modifiers {
                let with_modifier = format!("{name}/{modifier}");
                let signature = self.utility_signature(&with_modifier);
                if !signature.is_valid() {
                    continue;
                }
                unrecorded.push(alloc.str(&with_modifier));
            }
        }
        unrecorded
    }

    /// The declarations `class` produces, also recorded in the static utility lookup.
    fn utility_properties(&mut self, class: &str) -> Arc<PropertyValues> {
        let key = (self.sig_key(), class.to_owned());
        if let Some(properties) = self.storage.utility_properties.get(&key) {
            return Arc::clone(properties);
        }
        let mut properties = PropertyValues::default();
        let class = match self.prefix() {
            Some(prefix) if !class.starts_with(prefix) => Cow::Owned(format!("{prefix}:{class}")),
            _ => Cow::Borrowed(class),
        };
        if let Some(candidate) = self.ds.parse_candidate(&class).1.first() {
            let declarations = self.with_scratch(|cx, alloc| {
                let mut ast: Nodes<'_> = Vec::new_in(alloc);
                cx.without_inline(|cx| {
                    ast.extend(cx.ds.compile_ast_nodes(candidate, flags::RESPECT_IMPORTANT).iter().map(|rule| rule.node.clone_in(alloc)));
                    cx.canonicalize_ast(&mut ast, alloc);
                });
                let mut declarations = std::vec::Vec::new();
                collect_declarations(&ast, &mut declarations);
                declarations.into_iter().map(|(property, value)| (property.to_owned(), value.to_owned())).collect::<std::vec::Vec<_>>()
            });
            let statics = self.storage.static_utilities.entry(key.0).or_default();
            for (property, value) in declarations {
                statics.entry(property.clone()).or_default().entry(value.clone()).or_default().insert(class.to_string());
                properties.entry(property).or_default().insert(value);
            }
        }
        let properties = Arc::new(properties);
        self.storage.utility_properties.insert(key, Arc::clone(&properties));
        properties
    }

    fn canonicalize_candidate(&mut self, raw: &str) -> &'a str {
        let key = (self.canon_key(), self.ds.parse_candidate(raw).0);
        if let Some(&result) = self.storage.candidates.get(&key) {
            return result;
        }
        let result = self.compute_candidate(raw);
        let result = self.ds.alloc.str(&result);
        self.storage.candidates.insert(key, result);
        result
    }

    fn compute_candidate(&mut self, raw: &str) -> String {
        for candidate in self.ds.parse_candidate(raw).1 {
            let variants: std::vec::Vec<&'a Variant<'a>> =
                candidate.variants.iter().rev().flat_map(|v| self.canonicalize_variant(v).iter().copied()).collect();
            if candidate.important || !variants.is_empty() {
                let base = Candidate { variants: &[], important: false, ..*candidate };
                let printed = self.print(&base);
                let mut result = self.canonicalize_candidate(&printed).to_owned();
                let prefix = self.ds.theme.prefix().filter(|_| !variants.is_empty());
                if let Some(prefix) = prefix {
                    result = result.get(prefix.len() + 1..).unwrap_or("").to_owned();
                }
                if !variants.is_empty() {
                    let printed: std::vec::Vec<String> = variants.iter().map(|v| print_variant(v)).collect();
                    result = format!("{}:{result}", printed.join(":"));
                }
                if candidate.important {
                    result.push('!');
                }
                if let Some(prefix) = prefix {
                    result = format!("{prefix}:{result}");
                }
                return result;
            }
            let result = self.compute_utility(raw);
            if result != raw {
                return result;
            }
        }
        raw.to_owned()
    }

    fn compute_utility(&mut self, raw: &str) -> String {
        for candidate in self.ds.parse_candidate(raw).1 {
            let replacement = UTILITY_PASSES.iter().fold(*candidate, |candidate, pass| pass(self, candidate));
            let printed = self.print(&replacement);
            if printed != raw {
                return printed;
            }
        }
        raw.to_owned()
    }

    fn canonicalize_variant(&mut self, variant: &'a Variant<'a>) -> &'a [&'a Variant<'a>] {
        if let Some(&result) = self.storage.variants.get(variant) {
            return result;
        }
        let mut replacement = vec![variant];
        for pass in VARIANT_PASSES {
            replacement = replacement.into_iter().flat_map(|v| pass(self, v)).collect();
        }
        let result: &'a [&'a Variant<'a>] = self.ds.alloc.alloc_slice_copy(&replacement);
        self.storage.variants.insert(*variant, result);
        result
    }

    fn collapse_candidates(&mut self, candidates: std::vec::Vec<String>) -> std::vec::Vec<String> {
        self.record_statics();
        let prefix = self.prefix().map(|p| format!("{p}:")).unwrap_or_default();
        let mut groups: Map<String, Map<&str, Set<String>>> = Map::default();
        for candidate in &candidates {
            let mut variants: std::vec::Vec<&str> = segment(candidate, b':').into_iter().collect();
            let mut utility = variants.pop().unwrap_or("");
            let important = utility.ends_with('!');
            if important {
                utility = &utility[..utility.len() - 1];
            }
            let before = if variants.is_empty() { String::new() } else { format!("{}:", variants.join(":")) };
            let after = if important { "!" } else { "" };
            groups.entry(before).or_default().entry(after).or_default().insert(format!("{prefix}{utility}"));
        }
        let mut result: Set<String> = Set::default();
        for (before, group) in groups {
            for (after, candidates) in group {
                for candidate in self.collapse_group(candidates.into_iter().collect()) {
                    let candidate = match candidate.strip_prefix(prefix.as_str()) {
                        Some(rest) if !prefix.is_empty() => rest,
                        _ => &candidate,
                    };
                    result.insert(format!("{before}{candidate}{after}"));
                }
            }
        }
        result.into_iter().collect()
    }

    fn precompute_line_heights(&mut self, properties: &[Arc<PropertyValues>]) {
        let font_sizes = self.ds.ds.theme.keys_in_namespaces(["--text"]);
        if font_sizes.is_empty() {
            return;
        }
        let mut interesting: std::vec::Vec<LineHeight<'_>> = std::vec::Vec::new();
        let mut seen = FxHashSet::default();
        for line_height in properties.iter().filter_map(|p| p.get("line-height")).flatten() {
            if !seen.insert(line_height.as_str()) {
                continue;
            }
            let Some(bare) = self.spacing(line_height) else {
                continue;
            };
            let (value, modifier) = if is_valid_spacing_number(bare) {
                (LineHeight::Bare(bare), format_js_number(bare))
            } else {
                (LineHeight::Raw(line_height), format!("[{line_height}]"))
            };
            if !interesting.contains(&value) {
                interesting.push(value);
            }
            for name in &font_sizes {
                self.utility_properties(&format!("text-{name}/{modifier}"));
            }
        }
        let mut seen = FxHashSet::default();
        for font_size in properties.iter().filter_map(|p| p.get("font-size")).flatten() {
            if !seen.insert(font_size.as_str()) {
                continue;
            }
            for line_height in &interesting {
                match line_height {
                    LineHeight::Bare(value) => self.utility_properties(&format!("text-[{font_size}]/{}", format_js_number(*value))),
                    LineHeight::Raw(value) => self.utility_properties(&format!("text-[{font_size}]/[{value}]")),
                };
            }
        }
    }

    /// Other functional utilities with the same value as `candidate`, by the declarations they share with it.
    fn dynamic_utilities(&mut self, candidate: &str) -> Map<String, Map<String, Set<String>>> {
        let mut result: Map<String, Map<String, Set<String>>> = Map::default();
        let relevant = self.utility_properties(candidate);
        if relevant.is_empty() {
            return result;
        }
        for parsed in self.parse(candidate) {
            let CandidateKind::Functional { root: own, value: Some(value), modifier } = parsed.kind else {
                continue;
            };
            for root in self.ds.ds.utilities.keys(UtilityKind::Functional) {
                if root == own {
                    continue;
                }
                let replacement =
                    self.print_unprefixed(&Candidate { kind: CandidateKind::Functional { root, value: Some(value), modifier }, ..*parsed });
                for (property, values) in self.utility_properties(&replacement).iter() {
                    if !relevant.contains_key(property) {
                        continue;
                    }
                    let entry = result.entry(property.clone()).or_default();
                    for value in values {
                        entry.entry(value.clone()).or_default().insert(replacement.clone());
                    }
                }
            }
            return result;
        }
        result
    }

    fn collapse_group(&mut self, candidates: std::vec::Vec<String>) -> std::vec::Vec<String> {
        let properties: std::vec::Vec<Arc<PropertyValues>> = candidates.iter().map(|c| self.utility_properties(c)).collect();
        if properties.iter().any(|p| p.contains_key("line-height")) {
            self.precompute_line_heights(&properties);
        }

        let sig_key = self.sig_key();
        let mut dynamic: FxHashMap<usize, Map<String, Map<String, Set<String>>>> = FxHashMap::default();
        let other_utilities: std::vec::Vec<Set<String>> = properties
            .iter()
            .enumerate()
            .map(|(idx, property_values)| {
                let mut result: Option<Set<String>> = None;
                for (property, values) in property_values.iter() {
                    let mut others: Set<String> = Set::default();
                    if let Some(group) = self.storage.static_utilities.get(&sig_key).and_then(|s| s.get(property)) {
                        others.extend(group.values().flatten().cloned());
                    }
                    for value in values {
                        let dynamic = dynamic.entry(idx).or_insert_with(|| self.dynamic_utilities(&candidates[idx]));
                        if let Some(found) = dynamic.get(property).and_then(|v| v.get(value)) {
                            others.extend(found.iter().cloned());
                        }
                    }
                    let current = match result {
                        None => others,
                        Some(previous) => intersection(&previous, &others),
                    };
                    let empty = current.is_empty();
                    result = Some(current);
                    if empty {
                        break;
                    }
                }
                result.unwrap_or_default()
            })
            .collect();

        let mut linked: Map<usize, Set<usize>> = Map::default();
        for i in 0..other_utilities.len() {
            for j in i + 1..other_utilities.len() {
                if other_utilities[i].iter().any(|u| other_utilities[j].contains(u)) {
                    linked.entry(i).or_insert_with(|| Set::from_iter([i])).insert(j);
                    linked.entry(j).or_insert_with(|| Set::from_iter([j])).insert(i);
                }
            }
        }
        if linked.is_empty() {
            return candidates;
        }
        let mut unique: Set<std::vec::Vec<usize>> = Set::default();
        for group in linked.values() {
            let mut sorted: std::vec::Vec<usize> = group.iter().copied().collect();
            sorted.sort_unstable();
            unique.insert(sorted);
        }

        let mut result: Set<String> = candidates.iter().cloned().collect();
        let mut drop: Set<&str> = Set::default();
        for idxs in &unique {
            for combo in combinations(idxs) {
                if combo.iter().any(|&idx| drop.contains(candidates[idx].as_str())) {
                    continue;
                }
                let Some(potential) = combo.iter().map(|&idx| other_utilities[idx].clone()).reduce(|a, b| intersection(&a, &b)) else {
                    continue;
                };
                if potential.is_empty() {
                    continue;
                }
                let mut names: std::vec::Vec<&str> = combo.iter().map(|&idx| candidates[idx].as_str()).collect();
                names.sort_by(|a, b| locale_compare(a, b));
                let collapsed = self.utility_signature(&names.join(" "));
                for replacement in &potential {
                    if self.utility_signature(replacement) != collapsed {
                        continue;
                    }
                    result.insert(replacement.clone());
                    for &item in &combo {
                        if candidates[item] != *replacement {
                            drop.insert(&candidates[item]);
                        }
                    }
                    break;
                }
            }
        }
        for item in drop {
            result.shift_remove(item);
        }
        result.into_iter().collect()
    }
}

fn collect_declarations<'a>(nodes: &[Node<'a>], out: &mut std::vec::Vec<(&'a str, &'a str)>) {
    for node in nodes {
        match node {
            Node::Decl { property, value: Some(value), .. } => out.push((property, value)),
            Node::Rule { nodes, .. } | Node::AtRule { nodes, .. } | Node::Context { nodes, .. } | Node::AtRoot(nodes) => {
                collect_declarations(nodes, out);
            }
            _ => {}
        }
    }
}

/// `Set.prototype.intersection`: iterates the smaller set, keeping its order.
fn intersection(a: &Set<String>, b: &Set<String>) -> Set<String> {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    small.iter().filter(|item| large.contains(*item)).cloned().collect()
}

/// Subsets of at least two items, largest first, each size in Gosper order.
fn combinations(items: &[usize]) -> impl Iterator<Item = std::vec::Vec<usize>> + '_ {
    let n = if items.len() < 128 { items.len() as u32 } else { 0 };
    (2..=n).rev().flat_map(move |k| {
        let limit = 1u128 << n;
        let mut mask = (1u128 << k) - 1;
        std::iter::from_fn(move || {
            if mask >= limit {
                return None;
            }
            let out = (0..n).filter(|i| mask >> i & 1 == 1).map(|i| items[i as usize]).collect();
            let carry = mask.isolate_lowest_one();
            let ripple = mask + carry;
            mask = (((ripple ^ mask) >> 2) / carry) | ripple;
            Some(out)
        })
    })
}
