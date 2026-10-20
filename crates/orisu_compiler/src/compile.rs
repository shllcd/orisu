// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::attributify::retarget;
use crate::design_system::{DesignSystem, Session};
use orisu_allocator::{AllocExt, Allocator, ArenaString, Vec};
use orisu_ast::{Node, Nodes};
use orisu_candidate::{Candidate, CandidateKind};
use orisu_optimizer::Fragment;
use orisu_parser::compare::compare;
use orisu_parser::escape::{escape, escape_into};
use orisu_registry::property_order::property_order;
use orisu_registry::utilities::UtilityKind;
use orisu_registry::variants::{VariantKey, apply_variant};
use smallvec::SmallVec;
use std::cmp::Ordering;
use std::collections::VecDeque;

pub mod flags {
    pub const NONE: u8 = 0;
    pub const RESPECT_IMPORTANT: u8 = 1 << 0;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PropertySort<'a> {
    pub order: &'a [u16],
    pub count: u32,
}

#[derive(Debug)]
pub struct CompiledRule<'a> {
    pub node: Node<'a>,
    pub property_sort: PropertySort<'a>,
}

#[derive(Debug, Clone)]
pub struct NodeSorting<'a> {
    pub properties: PropertySort<'a>,
    pub variants: VariantKey<'a>,
    pub candidate: &'a str,
}

pub struct Compiled<'a> {
    pub ast_nodes: Nodes<'a>,
    pub sorting: std::vec::Vec<NodeSorting<'a>>,
}

/// The class selector of an attributified utility and the attribute selector replacing it.
type Retarget<'a> = &'a (&'a str, &'a str);

/// One rule of a compiled candidate, still shared with the session's cache.
pub struct Entry<'a> {
    rules: &'a [CompiledRule<'a>],
    index: usize,
    target: Option<Retarget<'a>>,
    pub sorting: NodeSorting<'a>,
    pub fragment: Option<Fragment<'a>>,
}

impl<'a> Entry<'a> {
    pub fn node_in<'s>(&self, alloc: &'s Allocator) -> Node<'s>
    where
        'a: 's,
    {
        let mut node = self.rules[self.index].node.clone_in(alloc);
        if let Some(target) = self.target {
            retarget(&mut node, target.0, target.1, alloc);
        }
        node
    }

    /// The cached rule itself, or a copy in `alloc` when it needs retargeting.
    pub fn node_ref<'r, 's: 'r>(&'r self, alloc: &'s Allocator) -> &'r Node<'s>
    where
        'a: 's,
    {
        match self.target {
            None => &self.rules[self.index].node,
            Some(_) => alloc.alloc(self.node_in(alloc)),
        }
    }
}

pub fn compile_candidates<'a, 's>(
    raw_candidates: impl IntoIterator<Item = &'s str>,
    ds: &mut Session<'_, 'a>,
    respect_important: bool,
    on_invalid: impl FnMut(&str),
) -> Compiled<'a> {
    let entries = compile_entries(raw_candidates, ds, respect_important, on_invalid);
    let mut ast_nodes = Vec::with_capacity_in(entries.len(), ds.alloc);
    let mut sorting = std::vec::Vec::with_capacity(entries.len());
    for entry in entries {
        ast_nodes.push(entry.node_in(ds.alloc));
        sorting.push(entry.sorting);
    }
    Compiled { ast_nodes, sorting }
}

/// The rules of every valid candidate in output order.
pub fn compile_entries<'a, 's>(
    raw_candidates: impl IntoIterator<Item = &'s str>,
    ds: &mut Session<'_, 'a>,
    respect_important: bool,
    mut on_invalid: impl FnMut(&str),
) -> std::vec::Vec<Entry<'a>> {
    let mut matches: std::vec::Vec<(&'a str, &'a [Candidate<'a>], Option<Retarget<'a>>)> = std::vec::Vec::new();
    for raw in raw_candidates {
        if ds.invalid_candidates.contains(raw) {
            on_invalid(raw);
            continue;
        }
        let attribute = ds.attributify.and_then(|attributify| attributify.resolve(raw));
        let (raw, candidates, target) = match attribute {
            Some((utility, selector)) => {
                let candidates = ds.parse_candidate(&utility).1;
                let class = ds.alloc.concat(&[".", &escape(&utility)]);
                let target: Retarget<'a> = ds.alloc.alloc((class, ds.alloc.str(&selector)));
                (ds.alloc.str(raw), candidates, Some(target))
            }
            None => {
                let (raw, candidates) = ds.parse_candidate(raw);
                (raw, candidates, None)
            }
        };
        if candidates.is_empty() {
            on_invalid(raw);
            continue;
        }
        matches.push((raw, candidates, target));
    }

    let flags = if respect_important { flags::RESPECT_IMPORTANT } else { flags::NONE };

    let mut entries = std::vec::Vec::new();
    for (raw, candidates, target) in matches {
        let mut found = false;
        for candidate in candidates {
            let rules = ds.compile_ast_nodes(candidate, flags);
            if rules.is_empty() {
                continue;
            }
            found = true;
            let variants = ds.variants.sort_key(&ds.theme, candidate.variants);
            for (index, rule) in rules.iter().enumerate() {
                let sorting = NodeSorting { properties: rule.property_sort, variants: variants.clone(), candidate: raw };
                entries.push(Entry { rules, index, target, sorting, fragment: None });
            }
        }
        if !found {
            on_invalid(raw);
        }
    }

    let ds = ds.ds;
    entries.sort_by(|a, z| compare_sorting(ds, &a.sorting, &z.sorting));
    entries
}

/// Merges `fresh`, already in output order, into `entries` as if both had been sorted together with `fresh` last.
pub fn insert_sorted<'a>(entries: &mut std::vec::Vec<Entry<'a>>, fresh: std::vec::Vec<Entry<'a>>, ds: &DesignSystem<'a>) {
    if entries.is_empty() {
        *entries = fresh;
        return;
    }
    let mut from = 0;
    for entry in fresh {
        let at = from + entries[from..].partition_point(|e| compare_sorting(ds, &e.sorting, &entry.sorting).is_le());
        entries.insert(at, entry);
        from = at + 1;
    }
}

fn compare_sorting<'a>(ds: &DesignSystem<'a>, a: &NodeSorting<'a>, z: &NodeSorting<'a>) -> Ordering {
    ds.variants
        .compare_keys(&ds.theme, &a.variants, &z.variants)
        .then_with(|| {
            let (ao, zo) = (a.properties.order, z.properties.order);
            let offset = ao.iter().zip(zo.iter()).take_while(|(x, y)| x == y).count();
            match (ao.get(offset), zo.get(offset)) {
                (Some(x), Some(y)) => x.cmp(y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        })
        .then_with(|| z.properties.count.cmp(&a.properties.count))
        .then_with(|| compare(a.candidate, z.candidate))
}

pub fn compile_ast_nodes<'a>(candidate: &Candidate<'a>, ds: &Session<'_, 'a>, flags: u8) -> std::vec::Vec<CompiledRule<'a>> {
    let asts = compile_base_utility(candidate, ds);
    if asts.is_empty() {
        return std::vec::Vec::new();
    }
    let respect_important = ds.important && flags & flags::RESPECT_IMPORTANT != 0;
    let mut selector = ArenaString::with_capacity_in(candidate.raw.len() + 1, ds.alloc);
    selector.push('.');
    escape_into(candidate.raw, &mut selector);
    let selector = selector.into_bump_str();

    let cx = ds.cx();
    let mut rules = std::vec::Vec::with_capacity(asts.len());
    for mut nodes in asts {
        let property_sort = property_sort(&nodes, ds.alloc);
        if candidate.important || respect_important {
            apply_important(&mut nodes);
        }
        let mut node = Node::Rule { selector, nodes };
        for variant in candidate.variants {
            if apply_variant(&cx, &mut node, variant, &ds.variants, 0).is_none() {
                return std::vec::Vec::new();
            }
        }
        rules.push(CompiledRule { node, property_sort });
    }
    rules
}

fn compile_base_utility<'a>(candidate: &Candidate<'a>, ds: &Session<'_, 'a>) -> SmallVec<[Nodes<'a>; 1]> {
    let mut asts = SmallVec::new();
    let cx = ds.cx();
    let (root, kind) = match candidate.kind {
        CandidateKind::Arbitrary { property, value, modifier } => {
            let Some(value) = cx.as_color(value, modifier) else {
                return asts;
            };
            asts.push(cx.nodes([cx.decl(property, value)]));
            return asts;
        }
        CandidateKind::Static { root } => (root, UtilityKind::Static),
        CandidateKind::Functional { root, .. } => (root, UtilityKind::Functional),
    };
    let utilities = ds.utilities.get(root);
    for fallback in [false, true] {
        for utility in utilities {
            if utility.kind != kind || utility.is_fallback() != fallback {
                continue;
            }
            match utility.run(&ds.cx(), candidate) {
                Some(nodes) => asts.push(nodes),
                None if utility.types.is_some_and(|t| !t.is_empty()) => return asts,
                None => {}
            }
        }
        if !asts.is_empty() {
            return asts;
        }
    }
    asts
}

fn apply_important(ast: &mut [Node<'_>]) {
    for node in ast {
        match node {
            Node::Decl { important, .. } => *important = true,
            Node::Rule { nodes, .. } | Node::AtRule { nodes, .. } => apply_important(nodes),
            _ => {}
        }
    }
}

pub fn property_sort<'a>(nodes: &[Node<'_>], alloc: &'a Allocator) -> PropertySort<'a> {
    let mut order: SmallVec<[u16; 8]> = SmallVec::new();
    let mut count = 0;
    let mut queue: VecDeque<&Node<'_>> = nodes.iter().collect();
    let mut seen_tw_sort = false;
    while let Some(node) = queue.pop_front() {
        match node {
            Node::Decl { property, value: Some(value), .. } => {
                count += 1;
                if seen_tw_sort {
                    continue;
                }
                if *property == "--tw-sort"
                    && let Some(idx) = property_order(value)
                {
                    order.push(idx);
                    seen_tw_sort = true;
                    continue;
                }
                if let Some(idx) = property_order(property) {
                    order.push(idx);
                }
            }
            Node::Rule { nodes, .. } | Node::AtRule { nodes, .. } => queue.extend(nodes.iter()),
            _ => {}
        }
    }
    order.sort_unstable();
    order.dedup();
    PropertySort { order: alloc.alloc_slice_copy(&order), count }
}
