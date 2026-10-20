// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

mod builtin;
mod css_utility;

pub use builtin::register_builtin;
pub use css_utility::{CssTemplate, CssUtility, is_valid_functional_utility_name, is_valid_static_utility_name};

use indexmap::IndexMap;
use orisu_allocator::{AllocExt, Allocator, Vec};
use orisu_ast::{Node, Nodes, clone_in};
use orisu_candidate::{Candidate, CandidateKind, Modifier, UtilityValue};
use orisu_parser::infer::{format_js_number, is_multiple_of_quarter, is_positive_integer, js_number};
use orisu_parser::math::add_whitespace_around_math_operators;
use orisu_parser::segment::segment;
use orisu_theme::Theme;
use rustc_hash::FxBuildHasher;
use smallvec::SmallVec;
use std::sync::Arc;

type SpacingHandle<'a> = dyn for<'t> Fn(&Cx<'a, 't>, &'a str) -> Option<Nodes<'a>> + Send + Sync + 'a;

/// The declarations a utility sets, computed without building nodes.
pub type DeclFn<'a> = dyn for<'t> Fn(&Cx<'a, 't>, &'a str, Option<&'a str>) -> Option<Vec<'a, (&'a str, &'a str)>> + Send + Sync + 'a;

/// A candidate-based declaration path for utilities that resolve their own values.
pub type UtilityDeclFn<'a> = dyn for<'t> Fn(&Cx<'a, 't>, &Candidate<'a>) -> Option<Vec<'a, (&'a str, &'a str)>> + Send + Sync + 'a;

fn declaration_fn<'a>(
    f: impl for<'t> Fn(&Cx<'a, 't>, &'a str, Option<&'a str>) -> Option<Vec<'a, (&'a str, &'a str)>> + Send + Sync + 'a,
) -> Arc<DeclFn<'a>> {
    Arc::new(f)
}

fn one_declaration<'a>(property: &'a str) -> Arc<DeclFn<'a>> {
    declaration_fn(move |cx, value, _| Some(Vec::from_iter_in([(property, value)], cx.alloc)))
}

/// Compile-time context handed to every utility.
#[derive(Clone, Copy)]
pub struct Cx<'a, 't> {
    pub theme: &'t Theme<'a>,
    pub alloc: &'a Allocator,
    /// [`orisu_theme::opts`] flags applied to every theme lookup.
    pub options: u8,
}

impl<'a> Cx<'a, '_> {
    #[inline]
    pub fn decl(&self, property: &'a str, value: &'a str) -> Node<'a> {
        Node::Decl { property, value: Some(value), important: false }
    }

    #[inline]
    pub fn decl_opt(&self, property: &'a str, value: Option<&'a str>) -> Node<'a> {
        Node::Decl { property, value, important: false }
    }

    #[inline]
    pub fn nodes<const N: usize>(&self, nodes: [Node<'a>; N]) -> Nodes<'a> {
        Vec::from_iter_in(nodes, self.alloc)
    }

    #[inline]
    pub fn one(&self, property: &'a str, value: &'a str) -> Option<Nodes<'a>> {
        Some(self.nodes([self.decl(property, value)]))
    }

    #[inline]
    pub fn cat(&self, parts: &[&str]) -> &'a str {
        self.alloc.concat(parts)
    }

    #[inline]
    pub fn str(&self, s: &str) -> &'a str {
        self.alloc.str(s)
    }

    #[inline]
    pub fn resolve(&self, value: Option<&str>, keys: &[&str]) -> Option<&'a str> {
        self.theme.resolve(value, keys, self.options)
    }

    #[inline]
    pub fn at_root<const N: usize>(&self, nodes: [Node<'a>; N]) -> Node<'a> {
        Node::AtRoot(self.nodes(nodes))
    }

    pub fn property(&self, ident: &'a str, initial: Option<&'a str>, syntax: Option<&'a str>) -> Node<'a> {
        let mut nodes = Vec::with_capacity_in(3, self.alloc);
        nodes.push(self.decl("syntax", syntax.map_or("\"*\"", |s| self.cat(&["\"", s, "\""]))));
        nodes.push(self.decl("inherits", "false"));
        if let Some(initial) = initial.filter(|v| !v.is_empty()) {
            nodes.push(self.decl("initial-value", initial));
        }
        Node::AtRule { name: "@property", params: ident, nodes }
    }

    #[inline]
    pub fn negate(&self, value: &'a str) -> &'a str {
        self.cat(&["calc(", value, " * -1)"])
    }

    pub fn with_alpha(&self, value: &'a str, alpha: &'a str) -> &'a str {
        let alpha = js_percent(self, alpha);
        if alpha == "100%" {
            return value;
        }
        self.cat(&["color-mix(in oklab, ", value, " ", alpha, ", transparent)"])
    }

    pub fn replace_alpha(&self, value: &'a str, alpha: &'a str) -> &'a str {
        let alpha = js_percent(self, alpha);
        self.cat(&["oklab(from ", value, " l a b / ", alpha, ")"])
    }

    pub fn as_color(&self, value: &'a str, modifier: Option<Modifier<'a>>) -> Option<&'a str> {
        match modifier {
            None => Some(value),
            Some(Modifier::Arbitrary(alpha)) => Some(self.with_alpha(value, alpha)),
            Some(Modifier::Named(name)) => {
                if let Some(alpha) = self.resolve(Some(name), &["--opacity"]).filter(|a| !a.is_empty()) {
                    return Some(self.with_alpha(value, alpha));
                }
                if !is_multiple_of_quarter(name) {
                    return None;
                }
                Some(self.with_alpha(value, self.cat(&[name, "%"])))
            }
        }
    }

    pub fn resolve_theme_color(&self, value: &'a str, modifier: Option<Modifier<'a>>, keys: &[&str]) -> Option<&'a str> {
        let resolved = match value {
            "inherit" => Some("inherit"),
            "transparent" => Some("transparent"),
            "current" => Some("currentcolor"),
            _ => self.resolve(Some(value), keys),
        };
        resolved.filter(|v| !v.is_empty()).and_then(|v| self.as_color(v, modifier))
    }
}

fn js_percent<'a>(cx: &Cx<'a, '_>, alpha: &'a str) -> &'a str {
    let n = js_number(alpha);
    if n.is_nan() { alpha } else { cx.cat(&[&format_js_number(n * 100.0), "%"]) }
}

pub type CompileFn<'a> = Box<dyn for<'t> Fn(&Cx<'a, 't>, &Candidate<'a>) -> Option<Nodes<'a>> + Send + Sync + 'a>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtilityKind {
    Static,
    Functional,
}

pub enum Compile<'a> {
    Decls(&'a [(&'a str, &'a str)]),
    Fn(CompileFn<'a>),
    /// A static `@utility` body.
    Css(Nodes<'a>),
    /// A functional `@utility` body with `--value()` and `--modifier()` normalized.
    CssFunctional(CssTemplate<'a>),
}

pub struct Utility<'a> {
    pub kind: UtilityKind,
    pub compile: Compile<'a>,
    pub types: Option<&'a [&'a str]>,
    /// Set for utilities registered through [`Functional`], so the canonical index can skip compilation.
    pub functional: Option<Arc<Functional<'a>>>,
    pub negative: bool,
    /// Declarations of a utility registered with its own handler, for the canonical index.
    pub declarations: Option<Arc<UtilityDeclFn<'a>>>,
}

/// The outcome of the declaration-only path of a utility.
pub enum Declared<'a> {
    /// The declarations the utility sets for this candidate.
    Yes(Vec<'a, (&'a str, &'a str)>),
    /// The utility does not apply to this candidate.
    No,
    /// The utility cannot be described without its handler.
    Unknown,
}

impl<'a> Utility<'a> {
    /// The declarations this utility sets, without compiling it.
    pub fn declarations(&self, cx: &Cx<'a, '_>, candidate: &Candidate<'a>) -> Declared<'a> {
        match &self.compile {
            Compile::Decls(declarations) => Declared::Yes(Vec::from_iter_in(declarations.iter().copied(), cx.alloc)),
            Compile::Css(body) => match flat_declarations(cx.alloc, body) {
                Some(declarations) => Declared::Yes(declarations),
                None => Declared::Unknown,
            },
            Compile::CssFunctional(template) => match css_utility::compile(cx, candidate, template) {
                Some(nodes) => match flat_declarations(cx.alloc, &nodes) {
                    Some(declarations) => Declared::Yes(declarations),
                    None => Declared::Unknown,
                },
                None => Declared::No,
            },
            Compile::Fn(_) => match (&self.functional, &self.declarations) {
                (Some(functional), _) if functional.has_declarations() => match functional.declarations(cx, candidate, self.negative) {
                    Some(declarations) => Declared::Yes(declarations),
                    None => Declared::No,
                },
                (None, Some(declarations)) => match declarations(cx, candidate) {
                    Some(declarations) => Declared::Yes(declarations),
                    None => Declared::No,
                },
                _ => Declared::Unknown,
            },
        }
    }

    #[inline]
    pub fn is_fallback(&self) -> bool {
        self.types.is_some_and(|t| t.len() > 1 && t.contains(&"any"))
    }

    #[inline]
    pub fn run(&self, cx: &Cx<'a, '_>, candidate: &Candidate<'a>) -> Option<Nodes<'a>> {
        match &self.compile {
            Compile::Decls(decls) => Some(Vec::from_iter_in(decls.iter().map(|&(p, v)| cx.decl(p, v)), cx.alloc)),
            Compile::Fn(f) => f(cx, candidate),
            Compile::Css(body) => Some(clone_in(body, cx.alloc)),
            Compile::CssFunctional(template) => css_utility::compile(cx, candidate, template),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SuggestionGroup {
    pub supports_negative: bool,
    pub values: std::vec::Vec<Option<String>>,
    pub modifiers: std::vec::Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Suggest<'a> {
    pub supports_negative: bool,
    pub supports_fractions: bool,
    pub values: &'a [&'a str],
    pub modifiers: &'a [&'a str],
    pub value_theme_keys: &'a [&'a str],
    pub modifier_theme_keys: &'a [&'a str],
    pub has_default_value: bool,
    /// Adds the default spacing scale to `values` when the theme defines `--spacing`.
    pub spacing: bool,
    /// Sets `has_default_value` when the theme defines this key.
    pub default_key: Option<&'a str>,
    pub literal: Option<String>,
    pub group: Option<SuggestionGroup>,
}

impl<'a> Suggest<'a> {
    pub fn values(values: &'a [&'a str]) -> Self {
        Self { values, ..Self::default() }
    }

    pub fn color(keys: &'a [&'a str]) -> Self {
        Self::values(&["current", "inherit", "transparent"]).keys(keys).modifier_keys(&["--opacity"]).modifiers(&STEPS_OF_5)
    }

    pub fn literal(value: impl Into<String>) -> Self {
        Self { literal: Some(value.into()), ..Self::default() }
    }

    pub fn keys(mut self, keys: &'a [&'a str]) -> Self {
        self.value_theme_keys = keys;
        self
    }

    pub fn modifier_keys(mut self, keys: &'a [&'a str]) -> Self {
        self.modifier_theme_keys = keys;
        self
    }

    pub fn modifiers(mut self, modifiers: &'a [&'a str]) -> Self {
        self.modifiers = modifiers;
        self
    }

    pub fn negative(mut self, yes: bool) -> Self {
        self.supports_negative = yes;
        self
    }

    pub fn fractions(mut self, yes: bool) -> Self {
        self.supports_fractions = yes;
        self
    }

    pub fn default_value(mut self, yes: bool) -> Self {
        self.has_default_value = yes;
        self
    }

    pub fn default_key(mut self, key: &'a str) -> Self {
        self.default_key = Some(key);
        self
    }
}

pub const ONE_TO_12: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"];
pub const ONE_TO_13: [&str; 13] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13"];
pub const STEPS_OF_5: [&str; 21] =
    ["0", "5", "10", "15", "20", "25", "30", "35", "40", "45", "50", "55", "60", "65", "70", "75", "80", "85", "90", "95", "100"];
pub const PERCENT_STEPS_OF_5: [&str; 21] = [
    "0%", "5%", "10%", "15%", "20%", "25%", "30%", "35%", "40%", "45%", "50%", "55%", "60%", "65%", "70%", "75%", "80%", "85%", "90%",
    "95%", "100%",
];

const SUGGESTED_FRACTIONS: [&str; 26] = [
    "1/2", "1/3", "2/3", "1/4", "2/4", "3/4", "1/5", "2/5", "3/5", "4/5", "1/6", "2/6", "3/6", "4/6", "5/6", "1/12", "2/12", "3/12",
    "4/12", "5/12", "6/12", "7/12", "8/12", "9/12", "10/12", "11/12",
];

#[derive(Default)]
pub struct Utilities<'a> {
    utilities: IndexMap<&'a str, SmallVec<[Utility<'a>; 1]>, FxBuildHasher>,
    completions: IndexMap<&'a str, std::vec::Vec<Suggest<'a>>, FxBuildHasher>,
}

impl<'a> Utilities<'a> {
    /// Returns the utility's index among those sharing `name`.
    pub fn add(&mut self, name: &'a str, utility: Utility<'a>) -> usize {
        let entry = self.utilities.entry(name).or_default();
        entry.push(utility);
        entry.len() - 1
    }

    pub fn body(&self, name: &str, index: usize) -> Option<&Nodes<'a>> {
        match &self.utilities.get(name)?.get(index)?.compile {
            Compile::Css(body) | Compile::CssFunctional(CssTemplate { body, .. }) => Some(body),
            _ => None,
        }
    }

    /// Swaps in a new body for a CSS utility, returning the old one.
    pub fn replace_body(&mut self, name: &str, index: usize, body: Nodes<'a>, alloc: &'a Allocator) -> Option<Nodes<'a>> {
        let compile = &mut self.utilities.get_mut(name)?.get_mut(index)?.compile;
        match compile {
            Compile::Css(old) => Some(std::mem::replace(old, body)),
            Compile::CssFunctional(old) => Some(std::mem::replace(old, CssTemplate::new(body, alloc)).body),
            _ => None,
        }
    }

    pub fn has(&self, name: &str, kind: UtilityKind) -> bool {
        self.utilities.get(name).is_some_and(|u| u.iter().any(|u| u.kind == kind))
    }

    pub fn get(&self, name: &str) -> &[Utility<'a>] {
        self.utilities.get(name).map_or(&[], |u| u.as_slice())
    }

    pub fn keys(&self, kind: UtilityKind) -> impl Iterator<Item = &'a str> + '_ {
        self.utilities.iter().filter(move |(_, u)| u.iter().any(|u| u.kind == kind)).map(|(k, _)| *k)
    }

    pub fn suggest(&mut self, name: &'a str, suggestions: impl IntoIterator<Item = Suggest<'a>>) {
        self.completions.entry(name).or_default().extend(suggestions);
    }

    pub fn completions(&self, name: &str, theme: &Theme<'a>) -> std::vec::Vec<SuggestionGroup> {
        self.completions_filtered(name, theme, |_| true)
    }

    /// [`Utilities::completions`] with only the modifiers `keep` accepts.
    pub fn completions_filtered(&self, name: &str, theme: &Theme<'a>, keep: impl Fn(&str) -> bool) -> std::vec::Vec<SuggestionGroup> {
        let Some(suggestions) = self.completions.get(name) else {
            if self.has(name, UtilityKind::Static) {
                return vec![SuggestionGroup::default()];
            }
            return vec![];
        };
        let mut groups = vec![];
        let has_spacing = theme.get(&["--spacing"]).is_some_and(|v| !v.is_empty());
        for defn in suggestions {
            if let Some(group) = &defn.group {
                let mut group = group.clone();
                group.modifiers.retain(|modifier| keep(modifier));
                groups.push(group);
                continue;
            }
            if let Some(literal) = &defn.literal {
                groups.push(SuggestionGroup { values: vec![Some(literal.clone())], ..Default::default() });
                continue;
            }
            let resolve = |keys: &[&str]| -> std::vec::Vec<String> {
                theme.keys_in_namespaces(keys.iter().copied()).into_iter().map(legacy_numeric_key).collect()
            };
            let mut values: std::vec::Vec<Option<String>> = defn.values.iter().map(|v| Some((*v).to_owned())).collect();
            if defn.spacing && has_spacing {
                values.extend(DEFAULT_SPACING_SUGGESTIONS.iter().map(|v| Some((*v).to_owned())));
            }
            values.extend(resolve(defn.value_theme_keys).into_iter().map(Some));
            let mut modifiers: std::vec::Vec<String> =
                defn.modifiers.iter().filter(|modifier| keep(modifier)).map(|v| (*v).to_owned()).collect();
            modifiers.extend(
                theme
                    .keys_in_namespaces(defn.modifier_theme_keys.iter().copied())
                    .into_iter()
                    .filter(|modifier| keep(modifier))
                    .map(legacy_numeric_key),
            );
            if defn.supports_fractions {
                values.extend(SUGGESTED_FRACTIONS.iter().map(|f| Some((*f).to_owned())));
            }
            if defn.has_default_value || defn.default_key.is_some_and(|key| theme.get(&[key]).is_some()) {
                values.insert(0, None);
            }
            groups.push(SuggestionGroup { supports_negative: defn.supports_negative, values, modifiers });
        }
        groups
    }
}

fn legacy_numeric_key(key: &str) -> String {
    let b = key.as_bytes();
    let mut out = String::with_capacity(key.len());
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i + 1 < b.len() && b[i] == b'_' && b[i + 1].is_ascii_digit() {
                let mid = i + 1;
                let mut end = mid;
                while end < b.len() && b[end].is_ascii_digit() {
                    end += 1;
                }
                out.push_str(&key[start..i]);
                out.push('.');
                out.push_str(&key[mid..end]);
                i = end;
                continue;
            }
            out.push_str(&key[start..i]);
            continue;
        }
        let ch = key[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct Named<'a> {
    pub value: &'a str,
    pub fraction: Option<&'a str>,
}

pub type BareFn<'a> = Box<dyn for<'t> Fn(&Cx<'a, 't>, Named<'a>) -> Option<&'a str> + Send + Sync + 'a>;
pub type StaticFn<'a> = Box<dyn for<'t> Fn(&Cx<'a, 't>, &'a str) -> Option<Nodes<'a>> + Send + Sync + 'a>;
pub type HandleFn<'a> = Box<dyn for<'t> Fn(&Cx<'a, 't>, &'a str, Option<&'a str>) -> Option<Nodes<'a>> + Send + Sync + 'a>;

pub enum DefaultValue<'a> {
    Undefined,
    Null,
    Value(&'a str),
}

pub struct Functional<'a> {
    pub supports_negative: bool,
    pub supports_fractions: bool,
    pub theme_keys: &'a [&'a str],
    pub default_value: DefaultValue<'a>,
    pub static_values: &'a [(&'a str, &'a [(&'a str, &'a str)])],
    pub static_nodes: Option<(&'a [&'a str], StaticFn<'a>)>,
    pub handle_bare_value: Option<BareFn<'a>>,
    pub handle_negative_bare_value: Option<BareFn<'a>>,
    pub handle: HandleFn<'a>,
    pub decl: Option<Arc<DeclFn<'a>>>,
}

impl<'a> Functional<'a> {
    pub fn new(handle: impl for<'t> Fn(&Cx<'a, 't>, &'a str, Option<&'a str>) -> Option<Nodes<'a>> + Send + Sync + 'a) -> Self {
        Self {
            supports_negative: false,
            supports_fractions: false,
            theme_keys: &[],
            default_value: DefaultValue::Undefined,
            static_values: &[],
            static_nodes: None,
            handle_bare_value: None,
            handle_negative_bare_value: None,
            handle: Box::new(handle),
            decl: None,
        }
    }

    pub fn decl(property: &'a str) -> Self {
        Self::new(move |cx, value, _| cx.one(property, value)).declaration(one_declaration(property))
    }

    /// Sets the declaration-only path the canonical index uses; compilation emits the same pairs.
    pub fn declaration(mut self, declaration: Arc<DeclFn<'a>>) -> Self {
        self.decl = Some(declaration);
        self
    }

    pub fn has_declarations(&self) -> bool {
        self.decl.is_some()
    }

    pub fn negative(mut self) -> Self {
        self.supports_negative = true;
        self
    }

    pub fn fractions(mut self) -> Self {
        self.supports_fractions = true;
        self
    }

    pub fn keys(mut self, keys: &'a [&'a str]) -> Self {
        self.theme_keys = keys;
        self
    }

    pub fn default_value(mut self, value: DefaultValue<'a>) -> Self {
        self.default_value = value;
        self
    }

    pub fn statics(mut self, values: &'a [(&'a str, &'a [(&'a str, &'a str)])]) -> Self {
        self.static_values = values;
        self
    }

    pub fn static_nodes(
        mut self,
        keys: &'a [&'a str],
        f: impl for<'t> Fn(&Cx<'a, 't>, &'a str) -> Option<Nodes<'a>> + Send + Sync + 'a,
    ) -> Self {
        self.static_nodes = Some((keys, Box::new(f)));
        self
    }

    fn static_keys(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.static_values.iter().map(|(k, _)| *k).chain(self.static_nodes.iter().flat_map(|(keys, _)| keys.iter().copied()))
    }

    pub fn bare(mut self, f: impl for<'t> Fn(&Cx<'a, 't>, Named<'a>) -> Option<&'a str> + Send + Sync + 'a) -> Self {
        self.handle_bare_value = Some(Box::new(f));
        self
    }

    pub fn negative_bare(mut self, f: impl for<'t> Fn(&Cx<'a, 't>, Named<'a>) -> Option<&'a str> + Send + Sync + 'a) -> Self {
        self.handle_negative_bare_value = Some(Box::new(f));
        self
    }

    fn resolve(&self, cx: &Cx<'a, '_>, candidate: &Candidate<'a>, negative: bool) -> Option<Resolved<'a>> {
        let CandidateKind::Functional { value, modifier, .. } = candidate.kind else {
            return None;
        };
        let mut data_type = None;
        let resolved = match value {
            None => {
                if modifier.is_some() {
                    return None;
                }
                match self.default_value {
                    DefaultValue::Value(v) => Some(v),
                    DefaultValue::Null => None,
                    DefaultValue::Undefined => cx.resolve(None, self.theme_keys),
                }
            }
            Some(UtilityValue::Arbitrary { data_type: dt, value }) => {
                if modifier.is_some() {
                    return None;
                }
                data_type = dt;
                Some(value)
            }
            Some(UtilityValue::Named { value, fraction }) => {
                let named = Named { value, fraction };
                let mut resolved = cx.resolve(Some(fraction.unwrap_or(value)), self.theme_keys);
                if resolved.is_none()
                    && self.supports_fractions
                    && let Some(fraction) = fraction
                {
                    let parts = segment(fraction, b'/');
                    let (lhs, rhs) = (parts[0], parts.get(1).copied().unwrap_or(""));
                    if !is_positive_integer(lhs) || !is_positive_integer(rhs) {
                        return None;
                    }
                    resolved = Some(cx.cat(&["calc(", lhs, " / ", rhs, " * 100%)"]));
                }
                if resolved.is_none()
                    && negative
                    && let Some(f) = &self.handle_negative_bare_value
                {
                    resolved = f(cx, named);
                    if !resolved.is_some_and(|v| v.contains('/')) && modifier.is_some() {
                        return None;
                    }
                    if let Some(v) = resolved {
                        return Some(Resolved::Value(v, None));
                    }
                }
                if resolved.is_none()
                    && let Some(f) = &self.handle_bare_value
                {
                    resolved = f(cx, named);
                    if !resolved.is_some_and(|v| v.contains('/')) && modifier.is_some() {
                        return None;
                    }
                }
                if resolved.is_none() && !negative && modifier.is_none() {
                    if let Some((_, declarations)) = self.static_values.iter().find(|(k, _)| *k == value) {
                        return Some(Resolved::Declarations(declarations));
                    }
                    if let Some((keys, f)) = &self.static_nodes
                        && keys.contains(&value)
                    {
                        return f(cx, value).map(Resolved::Nodes);
                    }
                }
                resolved
            }
        };
        let value = resolved?;
        let value = if negative { add_whitespace_around_math_operators(cx.negate(value), cx.alloc) } else { value };
        Some(Resolved::Value(value, data_type))
    }

    fn compile(&self, cx: &Cx<'a, '_>, candidate: &Candidate<'a>, negative: bool) -> Option<Nodes<'a>> {
        match self.resolve(cx, candidate, negative)? {
            Resolved::Declarations(declarations) => Some(declaration_nodes(cx, declarations.iter().copied())),
            Resolved::Nodes(nodes) => Some(nodes),
            Resolved::Value(value, data_type) => match &self.decl {
                Some(decl) => Some(declaration_nodes(cx, decl(cx, value, data_type)?.iter().copied())),
                None => (self.handle)(cx, value, data_type),
            },
        }
    }

    /// The declarations this utility sets, without building nodes; `None` when it needs a handler.
    pub fn declarations(&self, cx: &Cx<'a, '_>, candidate: &Candidate<'a>, negative: bool) -> Option<Vec<'a, (&'a str, &'a str)>> {
        match self.resolve(cx, candidate, negative)? {
            Resolved::Declarations(declarations) => Some(Vec::from_iter_in(declarations.iter().copied(), cx.alloc)),
            Resolved::Nodes(_) => None,
            Resolved::Value(value, data_type) => (self.decl.as_ref()?)(cx, value, data_type),
        }
    }
}

enum Resolved<'a> {
    Value(&'a str, Option<&'a str>),
    Declarations(&'a [(&'a str, &'a str)]),
    Nodes(Nodes<'a>),
}

fn declaration_nodes<'a>(cx: &Cx<'a, '_>, declarations: impl Iterator<Item = (&'a str, &'a str)>) -> Nodes<'a> {
    Vec::from_iter_in(declarations.map(|(property, value)| cx.decl(property, value)), cx.alloc)
}

fn flat_declarations<'a>(alloc: &'a Allocator, nodes: &[Node<'a>]) -> Option<Vec<'a, (&'a str, &'a str)>> {
    let mut out = Vec::new_in(alloc);
    for node in nodes {
        match node {
            Node::Decl { property, value: Some(value), .. } => out.push((*property, *value)),
            Node::AtRule { name: "@property", .. } => {}
            Node::AtRoot(nodes) | Node::Context { nodes, .. } => out.extend(flat_declarations(alloc, nodes)?),
            _ => return None,
        }
    }
    Some(out)
}

pub struct Spacing {
    pub supports_negative: bool,
    pub supports_fractions: bool,
}

pub const DEFAULT_SPACING_SUGGESTIONS: [&str; 34] = [
    "0", "0.5", "1", "1.5", "2", "2.5", "3", "3.5", "4", "5", "6", "7", "8", "9", "10", "11", "12", "14", "16", "20", "24", "28", "32",
    "36", "40", "44", "48", "52", "56", "60", "64", "72", "80", "96",
];

/// Registration-time helper mirroring the closure helpers inside Tailwind's `createUtilities`.
pub struct Registrar<'a, 'r> {
    pub utilities: &'r mut Utilities<'a>,
    pub theme: &'r Theme<'a>,
    pub alloc: &'a Allocator,
}

impl<'a> Registrar<'a, '_> {
    #[inline]
    pub fn cat(&self, parts: &[&str]) -> &'a str {
        self.alloc.concat(parts)
    }

    pub fn decls(&self, decls: &[(&str, &str)]) -> &'a [(&'a str, &'a str)] {
        let mut out = Vec::with_capacity_in(decls.len(), self.alloc);
        out.extend(decls.iter().map(|&(p, v)| (self.alloc.str(p), self.alloc.str(v))));
        out.into_bump_slice()
    }

    pub fn statics(&self, entries: &[(&str, &[(&str, &str)])]) -> &'a [(&'a str, &'a [(&'a str, &'a str)])] {
        let mut out = Vec::with_capacity_in(entries.len(), self.alloc);
        out.extend(entries.iter().map(|&(k, decls)| (self.alloc.str(k), self.decls(decls))));
        out.into_bump_slice()
    }

    pub fn keys(&self, keys: &[&str]) -> &'a [&'a str] {
        let mut out = Vec::with_capacity_in(keys.len(), self.alloc);
        out.extend(keys.iter().map(|k| self.alloc.str(k)));
        out.into_bump_slice()
    }

    pub fn static_decls(&mut self, name: &str, decls: &[(&str, &str)]) {
        let name = self.alloc.str(name);
        let decls = self.decls(decls);
        self.utilities.add(
            name,
            Utility {
                kind: UtilityKind::Static,
                compile: Compile::Decls(decls),
                types: None,
                functional: None,
                negative: false,
                declarations: None,
            },
        );
    }

    pub fn static_fn(&mut self, name: &str, f: impl for<'t> Fn(&Cx<'a, 't>, &Candidate<'a>) -> Option<Nodes<'a>> + Send + Sync + 'a) {
        let name = self.alloc.str(name);
        self.utilities.add(
            name,
            Utility {
                kind: UtilityKind::Static,
                compile: Compile::Fn(Box::new(f)),
                types: None,
                functional: None,
                negative: false,
                declarations: None,
            },
        );
    }

    pub fn functional_fn(&mut self, name: &str, f: impl for<'t> Fn(&Cx<'a, 't>, &Candidate<'a>) -> Option<Nodes<'a>> + Send + Sync + 'a) {
        let name = self.alloc.str(name);
        self.utilities.add(
            name,
            Utility {
                kind: UtilityKind::Functional,
                compile: Compile::Fn(Box::new(f)),
                types: None,
                functional: None,
                negative: false,
                declarations: None,
            },
        );
    }

    /// [`Registrar::functional_fn`] with a declaration-only twin used by the canonical index.
    pub fn functional_decl(
        &mut self,
        name: &str,
        f: impl for<'t> Fn(&Cx<'a, 't>, &Candidate<'a>) -> Option<Nodes<'a>> + Send + Sync + 'a,
        declarations: impl for<'t> Fn(&Cx<'a, 't>, &Candidate<'a>) -> Option<Vec<'a, (&'a str, &'a str)>> + Send + Sync + 'a,
    ) {
        let name = self.alloc.str(name);
        self.utilities.add(
            name,
            Utility {
                kind: UtilityKind::Functional,
                compile: Compile::Fn(Box::new(f)),
                types: None,
                functional: None,
                negative: false,
                declarations: Some(Arc::new(declarations)),
            },
        );
    }

    pub fn suggest(&mut self, name: &str, suggestions: impl IntoIterator<Item = Suggest<'a>>) {
        let name = self.alloc.str(name);
        self.utilities.suggest(name, suggestions);
    }

    fn add_functional(&mut self, name: &'a str, desc: Arc<Functional<'a>>, negative: bool) {
        let compile: CompileFn<'a> = {
            let desc = Arc::clone(&desc);
            Box::new(move |cx, candidate| desc.compile(cx, candidate, negative))
        };
        self.utilities.add(
            name,
            Utility {
                kind: UtilityKind::Functional,
                compile: Compile::Fn(compile),
                types: None,
                functional: Some(desc),
                negative,
                declarations: None,
            },
        );
    }

    pub fn functional(&mut self, name: &str, desc: Functional<'a>) {
        let desc = Arc::new(desc);
        let name = self.alloc.str(name);
        if desc.supports_negative {
            self.add_functional(self.cat(&["-", name]), Arc::clone(&desc), true);
        }
        let negative = desc.supports_negative;
        let fractions = desc.supports_fractions;
        let has_default = matches!(desc.default_value, DefaultValue::Value(_));
        let keys = desc.theme_keys;
        self.add_functional(name, Arc::clone(&desc), false);

        self.suggest(
            name,
            vec![Suggest {
                supports_negative: negative,
                supports_fractions: fractions,
                value_theme_keys: keys,
                has_default_value: has_default,
                ..Suggest::default()
            }],
        );
        let mut values = Vec::new_in(self.alloc);
        values.extend(desc.static_keys());
        if !values.is_empty() {
            self.suggest(name, vec![Suggest::values(values.into_bump_slice())]);
        }
    }

    pub fn color(
        &mut self,
        name: &str,
        keys: &'a [&'a str],
        handle: impl for<'t> Fn(&Cx<'a, 't>, &'a str) -> Option<Nodes<'a>> + Send + Sync + 'a,
    ) {
        self.functional_fn(name, move |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            let value = match value? {
                UtilityValue::Arbitrary { value, .. } => cx.as_color(value, modifier),
                UtilityValue::Named { value, .. } => cx.resolve_theme_color(value, modifier, keys),
            }?;
            handle(cx, value)
        });
        self.suggest(name, vec![Suggest::color(keys)]);
    }

    /// [`Registrar::color`] with a declaration-only twin used by the canonical index.
    pub fn color_decl(
        &mut self,
        name: &str,
        keys: &'a [&'a str],
        handle: impl for<'t> Fn(&Cx<'a, 't>, &'a str) -> Option<Nodes<'a>> + Send + Sync + 'a,
        declarations: impl for<'t> Fn(&Cx<'a, 't>, &'a str) -> Vec<'a, (&'a str, &'a str)> + Send + Sync + 'a,
    ) {
        self.functional_decl(
            name,
            move |cx, c| {
                let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                    return None;
                };
                let value = match value? {
                    UtilityValue::Arbitrary { value, .. } => cx.as_color(value, modifier),
                    UtilityValue::Named { value, .. } => cx.resolve_theme_color(value, modifier, keys),
                }?;
                handle(cx, value)
            },
            move |cx, c| {
                let CandidateKind::Functional { value, .. } = c.kind else {
                    return None;
                };
                let value = match value? {
                    UtilityValue::Arbitrary { value, .. } => cx.as_color(value, None),
                    UtilityValue::Named { value, .. } => cx.resolve_theme_color(value, None, keys),
                }?;
                Some(declarations(cx, value))
            },
        );
        self.suggest(name, vec![Suggest::color(keys)]);
    }

    pub fn spacing(
        &mut self,
        name: &str,
        keys: &'a [&'a str],
        opts: Spacing,
        handle: impl for<'t> Fn(&Cx<'a, 't>, &'a str) -> Option<Nodes<'a>> + Send + Sync + 'a,
    ) {
        self.spacing_with(name, keys, opts, handle, |desc| desc);
    }

    /// [`Registrar::spacing`] for utilities whose declarations are a single property.
    pub fn spacing_decl(&mut self, name: &str, keys: &'a [&'a str], opts: Spacing, property: &'a str) {
        self.spacing_with(
            name,
            keys,
            opts,
            move |cx, value| cx.one(property, value),
            move |desc| desc.declaration(one_declaration(property)),
        );
    }

    /// [`Registrar::spacing`] with a declaration-only twin used by the canonical index.
    pub fn spacing_declarations(
        &mut self,
        name: &str,
        keys: &'a [&'a str],
        opts: Spacing,
        handle: impl for<'t> Fn(&Cx<'a, 't>, &'a str) -> Option<Nodes<'a>> + Send + Sync + 'a,
        declarations: impl for<'t> Fn(&Cx<'a, 't>, &'a str) -> Vec<'a, (&'a str, &'a str)> + Send + Sync + 'a,
    ) {
        self.spacing_with(name, keys, opts, handle, move |desc| {
            desc.declaration(declaration_fn(move |cx, value, _| Some(declarations(cx, value))))
        });
    }

    pub fn spacing_with(
        &mut self,
        name: &str,
        keys: &'a [&'a str],
        opts: Spacing,
        handle: impl for<'t> Fn(&Cx<'a, 't>, &'a str) -> Option<Nodes<'a>> + Send + Sync + 'a,
        configure: impl FnOnce(Functional<'a>) -> Functional<'a>,
    ) {
        let handle: Arc<SpacingHandle<'a>> = Arc::new(handle);
        if opts.supports_negative {
            let h = Arc::clone(&handle);
            self.static_fn(self.cat(&["-", name, "-px"]), move |cx, _| h(cx, "-1px"));
        }
        let h = Arc::clone(&handle);
        self.static_fn(self.cat(&[name, "-px"]), move |cx, _| h(cx, "1px"));
        let mut desc = Functional::new(move |cx, value, _| handle(cx, value))
            .keys(keys)
            .default_value(DefaultValue::Null)
            .bare(|cx, named| {
                cx.resolve(None, &["--spacing"]).filter(|v| !v.is_empty())?;
                is_multiple_of_quarter(named.value).then(|| cx.cat(&["--spacing(", named.value, ")"]))
            })
            .negative_bare(|cx, named| {
                cx.resolve(None, &["--spacing"]).filter(|v| !v.is_empty())?;
                is_multiple_of_quarter(named.value).then(|| cx.cat(&["--spacing(-", named.value, ")"]))
            });
        desc.supports_negative = opts.supports_negative;
        desc.supports_fractions = opts.supports_fractions;
        self.functional(name, configure(desc));
        let (negative, fractions) = (opts.supports_negative, opts.supports_fractions);
        self.suggest(name, [Suggest { spacing: true, ..Suggest::default() }.negative(negative).fractions(fractions).keys(keys)]);
    }
}
