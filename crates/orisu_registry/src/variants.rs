// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::utilities::Cx;
use indexmap::IndexMap;
use orisu_allocator::{AllocExt, Allocator, ArenaString, Vec};
use orisu_ast::bytes::trim;
use orisu_ast::parse_at_rule;
use orisu_ast::{Builder, Node, Nodes, Scope, Walk, clone_in, walk, walk_mut, walk_scoped};
use orisu_candidate::Registry;
use orisu_candidate::{Variant, VariantKind, VariantValue};
use orisu_parser::attribute_selector::is_valid_attribute_selector;
use orisu_parser::compare::{Direction, compare_breakpoints};
use orisu_parser::infer::is_positive_integer;
use orisu_parser::segment::segment;
use orisu_parser::value::{self as value_parser, ValueNode};
use orisu_theme::Theme;
use rustc_hash::{FxBuildHasher, FxHashMap};
use smallvec::SmallVec;
use std::cmp::Ordering;
use std::ops::BitOr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compounds(u8);

impl Compounds {
    pub const NEVER: Self = Self(0);
    pub const AT_RULES: Self = Self(1);
    pub const STYLE_RULES: Self = Self(2);

    #[inline]
    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl BitOr for Compounds {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

pub fn compounds_for_selectors<'s>(selectors: impl IntoIterator<Item = &'s str>) -> Compounds {
    let mut compounds = Compounds::NEVER;
    for sel in selectors {
        if sel.starts_with('@') {
            if !sel.starts_with("@media") && !sel.starts_with("@supports") && !sel.starts_with("@container") {
                return Compounds::NEVER;
            }
            compounds = compounds | Compounds::AT_RULES;
            continue;
        }
        if sel.contains("::") {
            return Compounds::NEVER;
        }
        compounds = compounds | Compounds::STYLE_RULES;
    }
    compounds
}

/// Rewrites the rule node in place; `None` rejects the candidate.
pub type ApplyFn<'a> = Box<dyn for<'t> Fn(&Cx<'a, 't>, &mut Node<'a>, &Variant<'a>) -> Option<()> + Send + Sync + 'a>;
pub type CompareFn<'a> = Box<dyn Fn(&Theme<'a>, &Variant<'a>, &Variant<'a>) -> Ordering + Send + Sync + 'a>;
/// Where a functional variant's suggested values come from.
pub enum VariantValues<'a> {
    List(&'a [&'a str]),
    Namespace(&'a str),
    /// Every variant the named one compounds with.
    Compounds,
}

/// How a variant rewrites the rule it wraps.
pub enum Apply<'a> {
    /// Wraps the body in each selector or at-rule, cloning it for all but the last.
    Wrap(&'a [Wrapper<'a>]),
    /// A `@custom-variant` body with `@slot` standing for the wrapped nodes.
    Body(&'a [Node<'a>]),
    Fn(ApplyFn<'a>),
}

impl<'a> Apply<'a> {
    fn run(&self, cx: &Cx<'a, '_>, node: &mut Node<'a>, variant: &Variant<'a>) -> Option<()> {
        match self {
            Apply::Wrap(wrappers) => replace_body(node, cx.alloc, |nodes| {
                let mut out = Vec::with_capacity_in(wrappers.len(), cx.alloc);
                if let Some((last, rest)) = wrappers.split_last() {
                    out.extend(rest.iter().map(|w| w.wrap(clone_in(&nodes, cx.alloc))));
                    out.push(last.wrap(nodes));
                }
                out
            }),
            Apply::Body(body) => replace_body(node, cx.alloc, |nodes| {
                let mut out = clone_in(body, cx.alloc);
                substitute_at_slot(&mut out, &nodes, cx.alloc);
                out
            }),
            Apply::Fn(f) => f(cx, node, variant),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct VariantKey<'a>(SmallVec<[(u32, &'a Variant<'a>); 2]>);

pub struct VariantEntry<'a> {
    pub kind: VariantKind,
    pub order: u32,
    pub apply: Apply<'a>,
    pub compounds_with: Compounds,
    pub compounds: Compounds,
}

#[derive(Default)]
pub struct Variants<'a> {
    entries: IndexMap<&'a str, VariantEntry<'a>, FxBuildHasher>,
    compare_fns: FxHashMap<u32, CompareFn<'a>>,
    completions: FxHashMap<&'a str, VariantValues<'a>>,
    group_order: Option<u32>,
    last_order: u32,
}

impl<'a> Variants<'a> {
    pub fn static_variant(
        &mut self,
        name: &'a str,
        compounds: Compounds,
        apply: impl for<'t> Fn(&Cx<'a, 't>, &mut Node<'a>, &Variant<'a>) -> Option<()> + Send + Sync + 'a,
    ) {
        self.set(name, VariantKind::Static, Apply::Fn(Box::new(apply)), Compounds::NEVER, compounds);
    }

    pub fn static_data(&mut self, name: &'a str, compounds: Compounds, apply: Apply<'a>) {
        self.set(name, VariantKind::Static, apply, Compounds::NEVER, compounds);
    }

    pub fn functional(
        &mut self,
        name: &'a str,
        compounds: Compounds,
        apply: impl for<'t> Fn(&Cx<'a, 't>, &mut Node<'a>, &Variant<'a>) -> Option<()> + Send + Sync + 'a,
    ) {
        self.set(name, VariantKind::Functional, Apply::Fn(Box::new(apply)), Compounds::NEVER, compounds);
    }

    pub fn compound(
        &mut self,
        name: &'a str,
        compounds_with: Compounds,
        apply: impl for<'t> Fn(&Cx<'a, 't>, &mut Node<'a>, &Variant<'a>) -> Option<()> + Send + Sync + 'a,
    ) {
        self.set(name, VariantKind::Compound, Apply::Fn(Box::new(apply)), compounds_with, Compounds::STYLE_RULES);
    }

    /// Variants registered inside `f` share one order slot, ordered among themselves by `compare`.
    pub fn group(&mut self, compare: Option<CompareFn<'a>>, f: impl FnOnce(&mut Self)) {
        let order = self.next_order();
        self.group_order = Some(order);
        if let Some(compare) = compare {
            self.compare_fns.insert(order, compare);
        }
        f(self);
        self.group_order = None;
    }

    #[inline]
    pub fn has(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    #[inline]
    pub fn get(&self, name: &str) -> Option<&VariantEntry<'a>> {
        self.entries.get(name)
    }

    #[inline]
    pub fn kind(&self, name: &str) -> Option<VariantKind> {
        self.entries.get(name).map(|e| e.kind)
    }

    pub fn keys(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.entries.keys().copied()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&'a str, &VariantEntry<'a>)> + '_ {
        self.entries.iter().map(|(k, v)| (*k, v))
    }

    fn compounds_with_compounds(&self, parent: &str, child: Compounds) -> bool {
        let Some(parent) = self.entries.get(parent) else {
            return false;
        };
        parent.kind == VariantKind::Compound
            && child != Compounds::NEVER
            && parent.compounds_with != Compounds::NEVER
            && parent.compounds_with.intersects(child)
    }

    pub fn compounds_with(&self, parent: &str, child: &Variant<'_>) -> bool {
        let child = match *child {
            Variant::Arbitrary { selector, .. } => compounds_for_selectors([selector]),
            Variant::Static { root } | Variant::Functional { root, .. } | Variant::Compound { root, .. } => match self.entries.get(root) {
                Some(e) => e.compounds,
                None => return false,
            },
        };
        self.compounds_with_compounds(parent, child)
    }

    pub fn compounds_with_name(&self, parent: &str, child: &str) -> bool {
        self.entries.get(child).is_some_and(|c| self.compounds_with_compounds(parent, c.compounds))
    }

    pub fn suggest(&mut self, name: &'a str, values: VariantValues<'a>) {
        self.completions.insert(name, values);
    }

    pub fn completions(&self, name: &str, theme: &Theme<'a>) -> std::vec::Vec<String> {
        match self.completions.get(name) {
            None => std::vec::Vec::new(),
            Some(VariantValues::List(values)) => values.iter().map(|v| (*v).to_owned()).collect(),
            Some(VariantValues::Namespace(namespace)) => {
                theme.namespace(namespace).into_iter().filter_map(|(k, _)| k.map(str::to_owned)).collect()
            }
            Some(VariantValues::Compounds) => self.keys().filter(|k| self.compounds_with_name(name, k)).map(str::to_owned).collect(),
        }
    }

    #[inline]
    pub fn compare(&self, theme: &Theme<'a>, a: Option<&Variant<'a>>, z: Option<&Variant<'a>>) -> Ordering {
        self.compare_exact(theme, a, z).0
    }

    /// Like [`Variants::compare`], plus whether Tailwind's comparator would return non-zero:
    /// ties such as `group-hover/a` vs `group-hover/[a]` sort together yet stay distinct.
    pub fn compare_exact(&self, theme: &Theme<'a>, a: Option<&Variant<'a>>, z: Option<&Variant<'a>>) -> (Ordering, bool) {
        let decided = |o: Ordering| (o, true);
        let (a, z) = match (a, z) {
            _ if a == z => return (Ordering::Equal, false),
            (None, _) => return decided(Ordering::Less),
            (_, None) => return decided(Ordering::Greater),
            (Some(a), Some(z)) => (a, z),
        };
        let (a_root, z_root) = match (a, z) {
            (Variant::Arbitrary { selector: a, .. }, Variant::Arbitrary { selector: z, .. }) => {
                return decided(a.cmp(z));
            }
            (Variant::Arbitrary { .. }, _) => return decided(Ordering::Greater),
            (_, Variant::Arbitrary { .. }) => return decided(Ordering::Less),
            _ => (a.root().unwrap_or(""), z.root().unwrap_or("")),
        };
        let a_order = self.entries.get(a_root).map_or(0, |e| e.order);
        let z_order = self.entries.get(z_root).map_or(0, |e| e.order);
        if a_order != z_order {
            return decided(a_order.cmp(&z_order));
        }
        if let (Variant::Compound { variant: a_inner, modifier: a_mod, .. }, Variant::Compound { variant: z_inner, modifier: z_mod, .. }) =
            (a, z)
        {
            let inner = self.compare_exact(theme, Some(a_inner), Some(z_inner));
            if inner.1 {
                return inner;
            }
            return match (a_mod, z_mod) {
                (Some(a), Some(z)) => decided(a.value().cmp(z.value())),
                (Some(_), None) => decided(Ordering::Greater),
                (None, Some(_)) => decided(Ordering::Less),
                (None, None) => (Ordering::Equal, false),
            };
        }
        if let Some(compare) = self.compare_fns.get(&a_order) {
            let order = compare(theme, a, z);
            return (order, order.is_ne());
        }
        if a_root != z_root {
            return decided(a_root.cmp(z_root));
        }
        let value = |v: &Variant<'a>| match *v {
            Variant::Functional { value, .. } => value,
            _ => None,
        };
        decided(match (value(a), value(z)) {
            (None, None) => Ordering::Equal,
            (None, _) => Ordering::Less,
            (_, None) => Ordering::Greater,
            (Some(VariantValue::Arbitrary(_)), Some(VariantValue::Named(_))) => Ordering::Greater,
            (Some(VariantValue::Named(_)), Some(VariantValue::Arbitrary(_))) => Ordering::Less,
            (Some(VariantValue::Arbitrary(a) | VariantValue::Named(a)), Some(VariantValue::Arbitrary(z) | VariantValue::Named(z))) => {
                a.cmp(z)
            }
        })
    }

    /// The variants of one candidate in descending order, without duplicates, ready for [`Variants::compare_keys`].
    pub fn sort_key(&self, theme: &Theme<'a>, variants: &[&'a Variant<'a>]) -> VariantKey<'a> {
        let mut key: SmallVec<[(u32, &'a Variant<'a>); 2]> = variants.iter().map(|v| (self.rank(v), *v)).collect();
        key.sort_by(|a, z| self.compare_ranked(theme, *z, *a));
        key.dedup_by(|a, z| !self.compare_exact(theme, Some(a.1), Some(z.1)).1);
        VariantKey(key)
    }

    /// Orders candidates by their variants like Tailwind's bitmask of dense variant ranks, without computing the ranks.
    pub fn compare_keys(&self, theme: &Theme<'a>, a: &VariantKey<'a>, z: &VariantKey<'a>) -> Ordering {
        for pair in a.0.iter().zip(&z.0) {
            let order = self.compare_ranked(theme, *pair.0, *pair.1);
            if order.is_ne() {
                return order;
            }
        }
        a.0.len().cmp(&z.0.len())
    }

    #[inline]
    fn rank(&self, variant: &Variant<'a>) -> u32 {
        match variant.root() {
            None => u32::MAX,
            Some(root) => self.entries.get(root).map_or(0, |e| e.order),
        }
    }

    #[inline]
    fn compare_ranked(&self, theme: &Theme<'a>, a: (u32, &'a Variant<'a>), z: (u32, &'a Variant<'a>)) -> Ordering {
        a.0.cmp(&z.0).then_with(|| self.compare(theme, Some(a.1), Some(z.1)))
    }

    fn set(&mut self, name: &'a str, kind: VariantKind, apply: Apply<'a>, compounds_with: Compounds, compounds: Compounds) {
        if let Some(existing) = self.entries.get_mut(name) {
            existing.kind = kind;
            existing.apply = apply;
            existing.compounds = compounds;
            return;
        }
        self.last_order = self.next_order();
        self.entries.insert(name, VariantEntry { kind, order: self.last_order, apply, compounds_with, compounds });
    }

    #[inline]
    fn next_order(&self) -> u32 {
        self.group_order.unwrap_or(self.last_order + 1)
    }
}

#[inline]
fn body<'n, 'a>(node: &'n mut Node<'a>) -> Option<&'n mut Nodes<'a>> {
    match node {
        Node::Rule { nodes, .. } | Node::AtRule { nodes, .. } => Some(nodes),
        _ => None,
    }
}

#[inline]
fn replace_body<'a>(node: &mut Node<'a>, alloc: &'a Allocator, f: impl FnOnce(Nodes<'a>) -> Nodes<'a>) -> Option<()> {
    let nodes = body(node)?;
    let old = std::mem::replace(nodes, Vec::new_in(alloc));
    *nodes = f(old);
    Some(())
}

#[inline]
fn one<'a>(alloc: &'a Allocator, node: Node<'a>) -> Nodes<'a> {
    Builder::new(alloc).vec1(node)
}

pub fn apply_variant<'a>(cx: &Cx<'a, '_>, node: &mut Node<'a>, variant: &Variant<'a>, variants: &Variants<'a>, depth: usize) -> Option<()> {
    match *variant {
        Variant::Arbitrary { selector, relative } => {
            if relative && depth == 0 {
                return None;
            }
            replace_body(node, cx.alloc, |nodes| one(cx.alloc, Builder::new(cx.alloc).rule(selector, nodes)))
        }
        Variant::Compound { root, variant: inner, .. } => {
            let entry = variants.get(root)?;
            let mut isolated = Node::AtRule { name: "@slot", params: "", nodes: Vec::new_in(cx.alloc) };
            apply_variant(cx, &mut isolated, inner, variants, depth + 1)?;
            let mut children = std::mem::replace(body(&mut isolated)?, Vec::new_in(cx.alloc));
            if root == "not" && children.len() > 1 {
                return None;
            }
            for child in children.iter_mut() {
                if !matches!(child, Node::Rule { .. } | Node::AtRule { .. }) {
                    return None;
                }
                entry.apply.run(cx, child, variant)?;
            }
            let target = body(node)?;
            let inner_nodes = std::mem::replace(target, Vec::new_in(cx.alloc));
            walk_mut(&mut children, &mut |child| match child {
                Node::Rule { nodes, .. } | Node::AtRule { nodes, .. } if nodes.is_empty() => {
                    *nodes = clone_in(&inner_nodes, cx.alloc);
                    false
                }
                _ => true,
            });
            *target = children;
            Some(())
        }
        Variant::Static { root } | Variant::Functional { root, .. } => variants.get(root)?.apply.run(cx, node, variant),
    }
}

/// Registers `@custom-variant name (selectors);`.
pub fn register_custom_selectors<'a>(
    v: &mut Variants<'a>,
    alloc: &'a Allocator,
    name: &'a str,
    style_rules: &[&'a str],
    at_rules: &[&'a str],
) {
    let compounds = compounds_for_selectors(style_rules.iter().chain(at_rules).copied());
    let mut selectors: SmallVec<[&'a str; 4]> = SmallVec::new();
    if !style_rules.is_empty() {
        selectors.push(alloc.str(&style_rules.join(", ")));
    }
    selectors.extend_from_slice(at_rules);
    static_variant_with(v, alloc, name, &selectors, compounds);
}

/// Compounds of a `@custom-variant` body and whether it uses `@variant`.
pub fn body_compounds(body: &[Node<'_>]) -> (Compounds, bool) {
    let mut selectors: std::vec::Vec<String> = std::vec::Vec::new();
    let mut uses_at_variant = false;
    walk(body, &mut |node| {
        match node {
            Node::Rule { selector, .. } => selectors.push((*selector).to_owned()),
            Node::AtRule { name: "@variant", .. } => uses_at_variant = true,
            Node::AtRule { name, params, .. } if *name != "@slot" => selectors.push(format!("{name} {params}")),
            _ => {}
        }
        true
    });
    (compounds_for_selectors(selectors.iter().map(String::as_str)), uses_at_variant)
}

/// Registers a `@custom-variant` with a body whose `@variant` rules were already substituted.
pub fn register_custom_body<'a>(v: &mut Variants<'a>, name: &'a str, body: Nodes<'a>, compounds: Compounds) {
    v.static_data(name, compounds, Apply::Body(body.into_bump_slice()));
}

pub fn substitute_at_slot<'a>(ast: &mut Nodes<'a>, nodes: &Nodes<'a>, alloc: &'a Allocator) {
    let _ = walk_scoped(ast, Scope::default(), &mut |node, _| {
        Ok::<_, ()>(match node {
            Node::AtRule { name: "@slot", .. } => Walk::ReplaceSkip(clone_in(nodes, alloc)),
            Node::AtRule { name: name @ ("@keyframes" | "@property"), params, nodes: children } => {
                let inner = Node::AtRule { name, params, nodes: std::mem::replace(children, Vec::new_in(alloc)) };
                *node = Node::AtRoot(one(alloc, inner));
                Walk::Skip
            }
            _ => Walk::Continue,
        })
    });
}

/// Replaces `@variant a:b, c { … }` rules; returns whether any were present.
pub fn substitute_at_variant<'a>(
    ast: &mut Nodes<'a>,
    cx: &Cx<'a, '_>,
    registry: &mut impl Registry<'a>,
    variants: &Variants<'a>,
) -> Result<bool, String> {
    let alloc = cx.alloc;
    let mut found = false;
    walk_scoped(ast, Scope::default(), &mut |node, _| {
        let Node::AtRule { name: "@variant", params, nodes: body } = node else {
            return Ok(Walk::Continue);
        };
        let compound_variants = segment(params, b',');
        let mut out = Vec::new_in(alloc);
        for (idx, compound) in compound_variants.iter().enumerate() {
            let nodes =
                if idx == compound_variants.len() - 1 { std::mem::replace(body, Vec::new_in(alloc)) } else { clone_in(body, alloc) };
            let mut rule = Node::Rule { selector: "&", nodes };
            for variant in segment(compound, b':').iter().rev() {
                let variant = trim(variant);
                if variant.is_empty() {
                    return Err("Cannot use `@variant` with empty variant".to_owned());
                }
                let Some(parsed) = registry.parse_variant(alloc.str(variant)) else {
                    return Err(format!("Cannot use `@variant` with unknown variant: {variant}"));
                };
                if apply_variant(cx, &mut rule, parsed, variants, 0).is_none() {
                    return Err(format!("Cannot use `@variant` with variant: {variant}"));
                }
            }
            match rule {
                Node::Rule { selector: "&", nodes } => out.extend(nodes),
                rule => out.push(rule),
            }
        }
        found = true;
        Ok(Walk::Replace(out))
    })?;
    Ok(found)
}

#[derive(Clone, Copy)]
pub enum Wrapper<'a> {
    Rule(&'a str),
    AtRule(&'a str, &'a str),
}

impl<'a> Wrapper<'a> {
    fn parse(selector: &'a str, alloc: &'a Allocator) -> Self {
        if selector.starts_with('@')
            && let Node::AtRule { name, params, .. } = parse_at_rule(selector, Vec::new_in(alloc))
        {
            return Wrapper::AtRule(name, params);
        }
        Wrapper::Rule(selector)
    }

    #[inline]
    fn wrap(self, nodes: Nodes<'a>) -> Node<'a> {
        match self {
            Wrapper::Rule(selector) => Node::Rule { selector, nodes },
            Wrapper::AtRule(name, params) => Node::AtRule { name, params, nodes },
        }
    }
}

fn static_variant<'a>(v: &mut Variants<'a>, alloc: &'a Allocator, name: &'a str, selectors: &[&'a str]) {
    static_variant_with(v, alloc, name, selectors, compounds_for_selectors(selectors.iter().copied()));
}

pub fn static_variant_with<'a>(v: &mut Variants<'a>, alloc: &'a Allocator, name: &'a str, selectors: &[&'a str], compounds: Compounds) {
    let wrappers = alloc.alloc_slice_fill_iter(selectors.iter().map(|s| Wrapper::parse(s, alloc)));
    v.static_data(name, compounds, Apply::Wrap(wrappers));
}

fn negate_condition<'a>(alloc: &'a Allocator, rule_name: &str, condition: &'a str) -> &'a str {
    if rule_name != "@container" {
        let condition = trim(condition);
        if segment(condition, b' ')[0] == "not" {
            return condition.get(4..).unwrap_or("");
        }
        return alloc.concat(&["not ", condition]);
    }
    let ast = value_parser::parse(trim(condition), alloc);
    let is_fn = |i: usize| matches!(ast.get(i), Some(ValueNode::Function(..)));
    let word = |i: usize| match ast.get(i) {
        Some(ValueNode::Word(w)) => Some(*w),
        _ => None,
    };
    if is_fn(0) {
        return alloc.concat(&["not ", condition]);
    }
    if word(0) == Some("not") && is_fn(2) {
        return value_parser::to_css(&ast[2..], alloc);
    }
    if word(0).is_some() && word(2) == Some("not") && is_fn(4) {
        let mut out = ArenaString::new_in(alloc);
        value_parser::write_css(&ast[..2], &mut out);
        value_parser::write_css(&ast[4..], &mut out);
        return out.into_bump_str();
    }
    if word(0).is_some_and(|w| w != "not") && is_fn(2) {
        let mut out = ArenaString::new_in(alloc);
        value_parser::write_css(&ast[..1], &mut out);
        out.push_str(" not");
        value_parser::write_css(&ast[1..], &mut out);
        return out.into_bump_str();
    }
    alloc.concat(&["not ", condition])
}

fn negate_at_rule<'a>(alloc: &'a Allocator, name: &'a str, params: &'a str) -> Option<Node<'a>> {
    if !matches!(name, "@media" | "@supports" | "@container") {
        return None;
    }
    let conditions = segment(params, b',');
    if conditions.len() > 1 {
        return None;
    }
    Some(Node::AtRule { name, params: negate_condition(alloc, name, conditions[0]), nodes: Vec::new_in(alloc) })
}

fn negate_selector<'a>(alloc: &'a Allocator, selector: &str) -> Option<&'a str> {
    if selector.contains("::") {
        return None;
    }
    let mut out = ArenaString::with_capacity_in(selector.len() + 8, alloc);
    out.push_str("&:not(");
    for (i, sel) in segment(selector, b',').into_iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&sel.replace('&', "*"));
    }
    out.push(')');
    Some(out.into_bump_str())
}

struct Negate<'a, 'o> {
    alloc: &'a Allocator,
    root: &'o Node<'a>,
    replacement: Option<Node<'a>>,
    did_apply: bool,
    stopped: bool,
}

impl<'a, 'o> Negate<'a, 'o> {
    fn visit(&mut self, node: &'o Node<'a>, ancestors: &mut SmallVec<[&'o Node<'a>; 8]>) {
        if self.stopped {
            return;
        }
        let leaf = match node {
            Node::Rule { nodes, .. } | Node::AtRule { nodes, .. } => nodes.is_empty(),
            _ => false,
        };
        if !leaf {
            if let Some(children) = node.children() {
                ancestors.push(node);
                for child in children {
                    self.visit(child, ancestors);
                }
                ancestors.pop();
            }
            return;
        }
        let mut at_rules: SmallVec<[(&'a str, &'a str); 2]> = SmallVec::new();
        let mut selectors: SmallVec<[&'a str; 2]> = SmallVec::new();
        let current_root = self.replacement.as_ref().unwrap_or(self.root);
        let path = ancestors.iter().enumerate().map(|(i, n)| if i == 0 { current_root } else { *n });
        for n in path.chain(std::iter::once(node)) {
            match *n {
                Node::AtRule { name, params, .. } => at_rules.push((name, params)),
                Node::Rule { selector, .. } => selectors.push(selector),
                _ => {}
            }
        }
        if at_rules.len() > 1 || selectors.len() > 1 {
            self.stopped = true;
            return;
        }
        let mut rules = Vec::with_capacity_in(2, self.alloc);
        for selector in selectors {
            let Some(selector) = negate_selector(self.alloc, selector) else {
                self.did_apply = false;
                self.stopped = true;
                return;
            };
            rules.push(Node::Rule { selector, nodes: Vec::new_in(self.alloc) });
        }
        for (name, params) in at_rules {
            let Some(rule) = negate_at_rule(self.alloc, name, params) else {
                self.did_apply = false;
                self.stopped = true;
                return;
            };
            rules.push(rule);
        }
        self.replacement = Some(Node::Rule { selector: "&", nodes: rules });
        self.did_apply = true;
    }
}

fn rewrite_rules<'a>(node: &mut Node<'a>, inside_rule: bool, f: &mut impl FnMut(&'a str) -> &'a str) -> Option<bool> {
    let mut applied = false;
    if let Node::Rule { selector, .. } = node {
        if inside_rule {
            return None;
        }
        *selector = f(selector);
        applied = true;
    }
    let inside = inside_rule || applied;
    if let Some(children) = node.children_mut() {
        for child in children.iter_mut() {
            applied |= rewrite_rules(child, inside, f)?;
        }
    }
    Some(applied)
}

fn rewrite_variant<'a>(node: &mut Node<'a>, mut f: impl FnMut(&'a str) -> &'a str) -> Option<()> {
    rewrite_rules(node, false, &mut f)?.then_some(())
}

fn is_relative(variant: &Variant<'_>) -> bool {
    matches!(variant, Variant::Compound { variant: Variant::Arbitrary { relative: true, .. }, .. })
}

fn compound_modifier<'a>(variant: &Variant<'a>) -> Option<&'a str> {
    match *variant {
        Variant::Compound { modifier, .. } | Variant::Functional { modifier, .. } => modifier.map(|m| m.value()),
        _ => None,
    }
}

fn quote_attribute_value<'a>(alloc: &'a Allocator, input: &'a str) -> &'a str {
    if !input.contains('=') {
        return input;
    }
    let parts = segment(input, b'=');
    let attribute = parts[0];
    let value = if parts.len() > 1 { trim(&input[attribute.len() + 1..]) } else { "" };
    if value.starts_with(['\'', '"']) {
        return input;
    }
    let b = value.as_bytes();
    if b.len() > 1 && b[b.len() - 2] == b' ' && matches!(b[b.len() - 1], b'i' | b'I' | b's' | b'S') {
        let flag = &value[value.len() - 1..];
        return alloc.concat(&[attribute, "=\"", &value[..value.len() - 2], "\" ", flag]);
    }
    alloc.concat(&[attribute, "=\"", value, "\""])
}

fn functional_value<'a>(variant: &Variant<'a>) -> Option<(VariantValue<'a>, bool)> {
    match *variant {
        Variant::Functional { value: Some(value), modifier, .. } => Some((value, modifier.is_some())),
        _ => None,
    }
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_supports_function(value: &str) -> bool {
    let b = value.as_bytes();
    let mut i = b.iter().take_while(|&&c| is_word_byte(c) || c == b'-').count();
    while b.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    b.get(i) == Some(&b'(')
}

fn space_logical_operators<'a>(alloc: &'a Allocator, value: &'a str) -> &'a str {
    let b = value.as_bytes();
    let mut out = ArenaString::with_capacity_in(value.len() + 8, alloc);
    let mut last = 0;
    let mut i = 0;
    while i < b.len() {
        if i > 0 && is_word_byte(b[i - 1]) || !is_word_byte(b[i]) {
            i += 1;
            continue;
        }
        let word = ["and", "or", "not"]
            .into_iter()
            .find(|w| b[i..].starts_with(w.as_bytes()) && !b.get(i + w.len()).copied().is_some_and(is_word_byte));
        match word {
            Some(w) => {
                out.push_str(&value[last..i]);
                out.push(' ');
                out.push_str(w);
                out.push(' ');
                i += w.len();
                last = i;
            }
            None => i += 1,
        }
    }
    if last == 0 {
        return value;
    }
    out.push_str(&value[last..]);
    out.into_bump_str()
}

fn breakpoint<'a>(theme: &Theme<'a>, variant: &Variant<'a>) -> Option<&'a str> {
    let value = match *variant {
        Variant::Static { root } => return theme.resolve_value(Some(root), &["--breakpoint"]),
        Variant::Functional { value: Some(_), modifier: Some(_), .. } => return None,
        Variant::Functional { value: Some(VariantValue::Arbitrary(v)), .. } => Some(v),
        Variant::Functional { value: Some(VariantValue::Named(v)), .. } => theme.resolve_value(Some(v), &["--breakpoint"]),
        _ => None,
    };
    value.filter(|v| !v.is_empty() && !v.contains("var("))
}

fn container_width<'a>(theme: &Theme<'a>, variant: &Variant<'a>) -> Option<&'a str> {
    let value = match *variant {
        Variant::Functional { value: Some(VariantValue::Arbitrary(v)), .. } => Some(v),
        Variant::Functional { value: Some(VariantValue::Named(v)), .. } => theme.resolve_value(Some(v), &["--container"]),
        _ => None,
    };
    value.filter(|v| !v.is_empty() && !v.contains("var("))
}

type Lookup = for<'a> fn(&Theme<'a>, &Variant<'a>) -> Option<&'a str>;

fn compare_by<'a>(direction: Direction, lookup: Lookup) -> Option<CompareFn<'a>> {
    Some(Box::new(move |theme, a, z| {
        if a == z {
            return Ordering::Equal;
        }
        let asc = direction == Direction::Asc;
        let Some(a) = lookup(theme, a) else {
            return if asc { Ordering::Less } else { Ordering::Greater };
        };
        let Some(z) = lookup(theme, z) else {
            return if asc { Ordering::Greater } else { Ordering::Less };
        };
        compare_breakpoints(a, z, direction)
    }))
}

fn content_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    Node::AtRoot(cx.nodes([Node::AtRule {
        name: "@property",
        params: "--tw-content",
        nodes: cx.nodes([cx.decl("syntax", "\"*\""), cx.decl("initial-value", "\"\""), cx.decl("inherits", "false")]),
    }]))
}

fn pseudo_content<'a>(cx: &Cx<'a, '_>, node: &mut Node<'a>, selector: &'a str) -> Option<()> {
    replace_body(node, cx.alloc, |nodes| {
        let mut inner = Vec::with_capacity_in(nodes.len() + 2, cx.alloc);
        inner.push(content_properties(cx));
        inner.push(cx.decl("content", "var(--tw-content)"));
        inner.extend(nodes);
        one(cx.alloc, Node::Rule { selector, nodes: inner })
    })
}

fn wrap_rule<'a>(cx: &Cx<'a, '_>, node: &mut Node<'a>, selector: &'a str) -> Option<()> {
    replace_body(node, cx.alloc, |nodes| one(cx.alloc, Node::Rule { selector, nodes }))
}

fn wrap_at_rule<'a>(cx: &Cx<'a, '_>, node: &mut Node<'a>, name: &'a str, params: &'a str) -> Option<()> {
    replace_body(node, cx.alloc, |nodes| one(cx.alloc, Node::AtRule { name, params, nodes }))
}

fn nth<'a>(v: &mut Variants<'a>, name: &'a str, pseudo: &'static str) {
    v.functional(name, Compounds::STYLE_RULES, move |cx, node, variant| {
        let (value, has_modifier) = functional_value(variant)?;
        if has_modifier {
            return None;
        }
        let value = match value {
            VariantValue::Named(v) if !is_positive_integer(v) => return None,
            VariantValue::Arbitrary(v) | VariantValue::Named(v) => v,
        };
        wrap_rule(cx, node, cx.cat(&["&:", pseudo, "(", value, ")"]))
    });
}

fn container_query<'a>(cx: &Cx<'a, '_>, node: &mut Node<'a>, variant: &Variant<'a>, op: &str) -> Option<()> {
    let value = container_width(cx.theme, variant)?;
    let params = match compound_modifier(variant) {
        Some(name) => cx.cat(&[name, " (width ", op, " ", value, ")"]),
        None => cx.cat(&["(width ", op, " ", value, ")"]),
    };
    wrap_at_rule(cx, node, "@container", params)
}

fn group_like<'a>(v: &mut Variants<'a>, name: &'a str, class: &'a str, combinator: &'static str) {
    v.compound(name, Compounds::STYLE_RULES, move |cx, node, variant| {
        if is_relative(variant) {
            return None;
        }
        let prefix = cx.theme.prefix().map_or(String::new(), |p| format!("{p}\\:"));
        let variant_selector = match compound_modifier(variant) {
            Some(modifier) => cx.cat(&[":where(.", &prefix, class, "\\/", modifier, ")"]),
            None => cx.cat(&[":where(.", &prefix, class, ")"]),
        };
        rewrite_variant(node, |selector| {
            let selector = selector.replace('&', variant_selector);
            if segment(&selector, b',').len() > 1 {
                return cx.cat(&["&:is(:is(", &selector, ")", combinator, ")"]);
            }
            cx.cat(&["&:is(", &selector, combinator, ")"])
        })
    });
    v.suggest(name, VariantValues::Compounds);
}

pub fn register_builtin<'a>(v: &mut Variants<'a>, theme: &Theme<'a>, alloc: &'a Allocator) {
    static_variant_with(v, alloc, "*", &[":is(& > *)"], Compounds::NEVER);
    static_variant_with(v, alloc, "**", &[":is(& *)"], Compounds::NEVER);

    v.compound("not", Compounds::STYLE_RULES | Compounds::AT_RULES, |cx, node, variant| {
        if is_relative(variant) || compound_modifier(variant).is_some() {
            return None;
        }
        let mut negate = Negate { alloc: cx.alloc, root: &*node, replacement: None, did_apply: false, stopped: false };
        negate.visit(negate.root, &mut SmallVec::new());
        let did_apply = negate.did_apply;
        if let Some(replacement) = negate.replacement {
            *node = replacement;
        }
        if let Node::Rule { selector: "&", nodes } = node
            && nodes.len() == 1
            && let Some(child) = nodes.pop()
        {
            *node = child;
        }
        did_apply.then_some(())
    });
    v.suggest("not", VariantValues::Compounds);

    group_like(v, "group", "group", " *");
    group_like(v, "peer", "peer", " ~ *");

    static_variant(v, alloc, "first-letter", &["&::first-letter"]);
    static_variant(v, alloc, "first-line", &["&::first-line"]);
    static_variant(v, alloc, "marker", &["& *::marker", "&::marker", "& *::-webkit-details-marker", "&::-webkit-details-marker"]);
    static_variant(v, alloc, "selection", &["& *::selection", "&::selection"]);
    static_variant(v, alloc, "file", &["&::file-selector-button"]);
    static_variant(v, alloc, "placeholder", &["&::placeholder"]);
    static_variant(v, alloc, "backdrop", &["&::backdrop"]);
    static_variant(v, alloc, "details-content", &["&::details-content"]);

    v.static_variant("before", Compounds::NEVER, |cx, node, _| pseudo_content(cx, node, "&::before"));
    v.static_variant("after", Compounds::NEVER, |cx, node, _| pseudo_content(cx, node, "&::after"));

    for (name, selector) in [
        ("first", "&:first-child"),
        ("last", "&:last-child"),
        ("only", "&:only-child"),
        ("odd", "&:nth-child(odd)"),
        ("even", "&:nth-child(even)"),
        ("first-of-type", "&:first-of-type"),
        ("last-of-type", "&:last-of-type"),
        ("only-of-type", "&:only-of-type"),
        ("visited", "&:visited"),
        ("target", "&:target"),
        ("open", "&:is([open], :popover-open, :open)"),
        ("default", "&:default"),
        ("checked", "&:checked"),
        ("indeterminate", "&:indeterminate"),
        ("placeholder-shown", "&:placeholder-shown"),
        ("autofill", "&:autofill"),
        ("optional", "&:optional"),
        ("required", "&:required"),
        ("valid", "&:valid"),
        ("invalid", "&:invalid"),
        ("user-valid", "&:user-valid"),
        ("user-invalid", "&:user-invalid"),
        ("in-range", "&:in-range"),
        ("out-of-range", "&:out-of-range"),
        ("read-only", "&:read-only"),
        ("empty", "&:empty"),
        ("focus-within", "&:focus-within"),
    ] {
        static_variant(v, alloc, name, &[selector]);
    }
    v.static_variant("hover", Compounds::STYLE_RULES, |cx, node, _| {
        replace_body(node, cx.alloc, |nodes| {
            let media = Node::AtRule { name: "@media", params: "(hover: hover)", nodes };
            one(cx.alloc, Node::Rule { selector: "&:hover", nodes: one(cx.alloc, media) })
        })
    });
    for (name, selector) in [
        ("focus", "&:focus"),
        ("focus-visible", "&:focus-visible"),
        ("active", "&:active"),
        ("enabled", "&:enabled"),
        ("disabled", "&:disabled"),
        ("inert", "&:is([inert], [inert] *)"),
    ] {
        static_variant(v, alloc, name, &[selector]);
    }

    v.compound("in", Compounds::STYLE_RULES, |cx, node, variant| {
        if compound_modifier(variant).is_some() {
            return None;
        }
        rewrite_variant(node, |selector| cx.cat(&[":where(", &selector.replace('&', "*"), ") &"]))
    });
    v.suggest("in", VariantValues::Compounds);
    v.compound("has", Compounds::STYLE_RULES, |cx, node, variant| {
        if compound_modifier(variant).is_some() {
            return None;
        }
        rewrite_variant(node, |selector| cx.cat(&["&:has(", &selector.replace('&', "*"), ")"]))
    });
    v.suggest("has", VariantValues::Compounds);

    v.functional("aria", Compounds::STYLE_RULES, |cx, node, variant| {
        let (value, has_modifier) = functional_value(variant)?;
        if has_modifier {
            return None;
        }
        let selector = match value {
            VariantValue::Arbitrary(v) => cx.cat(&["[aria-", quote_attribute_value(cx.alloc, v), "]"]),
            VariantValue::Named(v) => cx.cat(&["[aria-", v, "=\"true\"]"]),
        };
        if !is_valid_attribute_selector(selector) {
            return None;
        }
        wrap_rule(cx, node, cx.cat(&["&", selector]))
    });
    v.suggest(
        "aria",
        VariantValues::List(&["busy", "checked", "disabled", "expanded", "hidden", "pressed", "readonly", "required", "selected"]),
    );
    v.functional("data", Compounds::STYLE_RULES, |cx, node, variant| {
        let (value, has_modifier) = functional_value(variant)?;
        if has_modifier {
            return None;
        }
        let (VariantValue::Arbitrary(value) | VariantValue::Named(value)) = value;
        let selector = cx.cat(&["[data-", quote_attribute_value(cx.alloc, value), "]"]);
        if !is_valid_attribute_selector(selector) {
            return None;
        }
        wrap_rule(cx, node, cx.cat(&["&", selector]))
    });
    nth(v, "nth", "nth-child");
    nth(v, "nth-last", "nth-last-child");
    nth(v, "nth-of-type", "nth-of-type");
    nth(v, "nth-last-of-type", "nth-last-of-type");

    v.functional("supports", Compounds::AT_RULES, |cx, node, variant| {
        let (value, has_modifier) = functional_value(variant)?;
        if has_modifier {
            return None;
        }
        let (VariantValue::Arbitrary(value) | VariantValue::Named(value)) = value;
        if is_supports_function(value) {
            return wrap_at_rule(cx, node, "@supports", space_logical_operators(cx.alloc, value));
        }
        let value = if value.contains(':') { value } else { cx.cat(&[value, ": var(--tw)"]) };
        let value = if value.starts_with('(') && value.ends_with(')') { value } else { cx.cat(&["(", value, ")"]) };
        wrap_at_rule(cx, node, "@supports", value)
    });

    static_variant(v, alloc, "motion-safe", &["@media (prefers-reduced-motion: no-preference)"]);
    static_variant(v, alloc, "motion-reduce", &["@media (prefers-reduced-motion: reduce)"]);
    static_variant(v, alloc, "contrast-more", &["@media (prefers-contrast: more)"]);
    static_variant(v, alloc, "contrast-less", &["@media (prefers-contrast: less)"]);

    v.group(compare_by(Direction::Desc, breakpoint), |v| {
        v.functional("max", Compounds::AT_RULES, |cx, node, variant| {
            if compound_modifier(variant).is_some() {
                return None;
            }
            let value = breakpoint(cx.theme, variant)?;
            wrap_at_rule(cx, node, "@media", cx.cat(&["(width < ", value, ")"]))
        });
    });
    v.suggest("max", VariantValues::Namespace("--breakpoint"));
    v.group(compare_by(Direction::Asc, breakpoint), |v| {
        for (key, value) in theme.namespace("--breakpoint") {
            let Some(key) = key else {
                continue;
            };
            let params = alloc.concat(&["(width >= ", value, ")"]);
            v.static_data(key, Compounds::AT_RULES, Apply::Wrap(alloc.alloc_slice_copy(&[Wrapper::AtRule("@media", params)])));
        }
        v.functional("min", Compounds::AT_RULES, |cx, node, variant| {
            if compound_modifier(variant).is_some() {
                return None;
            }
            let value = breakpoint(cx.theme, variant)?;
            wrap_at_rule(cx, node, "@media", cx.cat(&["(width >= ", value, ")"]))
        });
    });
    v.suggest("min", VariantValues::Namespace("--breakpoint"));

    v.group(compare_by(Direction::Desc, container_width), |v| {
        v.functional("@max", Compounds::AT_RULES, |cx, node, variant| container_query(cx, node, variant, "<"));
    });
    v.suggest("@max", VariantValues::Namespace("--container"));
    v.group(compare_by(Direction::Asc, container_width), |v| {
        v.functional("@", Compounds::AT_RULES, |cx, node, variant| container_query(cx, node, variant, ">="));
        v.functional("@min", Compounds::AT_RULES, |cx, node, variant| container_query(cx, node, variant, ">="));
    });
    v.suggest("@min", VariantValues::Namespace("--container"));
    v.suggest("@", VariantValues::Namespace("--container"));

    for (name, selector) in [
        ("portrait", "@media (orientation: portrait)"),
        ("landscape", "@media (orientation: landscape)"),
        ("ltr", "&:where(:dir(ltr), [dir=\"ltr\"], [dir=\"ltr\"] *)"),
        ("rtl", "&:where(:dir(rtl), [dir=\"rtl\"], [dir=\"rtl\"] *)"),
        ("dark", "@media (prefers-color-scheme: dark)"),
        ("starting", "@starting-style"),
        ("print", "@media print"),
        ("forced-colors", "@media (forced-colors: active)"),
        ("inverted-colors", "@media (inverted-colors: inverted)"),
        ("pointer-none", "@media (pointer: none)"),
        ("pointer-coarse", "@media (pointer: coarse)"),
        ("pointer-fine", "@media (pointer: fine)"),
        ("any-pointer-none", "@media (any-pointer: none)"),
        ("any-pointer-coarse", "@media (any-pointer: coarse)"),
        ("any-pointer-fine", "@media (any-pointer: fine)"),
        ("noscript", "@media (scripting: none)"),
    ] {
        static_variant(v, alloc, name, &[selector]);
    }
}
