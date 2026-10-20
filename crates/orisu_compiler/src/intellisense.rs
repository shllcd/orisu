// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::design_system::{DesignSystem, Session};
use indexmap::{IndexMap, IndexSet};
use orisu_allocator::{AllocExt, Vec};
use orisu_ast::Node;
use orisu_candidate::{Registry, VariantKind};
use orisu_parser::compare::compare;
use orisu_registry::utilities::UtilityKind;
use orisu_registry::variants::apply_variant;
use rustc_hash::FxBuildHasher;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassEntry {
    pub name: String,
    /// The functional or static utility the class was completed from.
    pub utility: String,
    pub modifiers: std::vec::Vec<String>,
}

struct ClassItem {
    utility: String,
    fraction: bool,
    modifiers: IndexSet<String, FxBuildHasher>,
}

fn is_fraction(value: &str) -> bool {
    value.split_once('/').is_some_and(|(a, b)| {
        !a.is_empty() && !b.is_empty() && a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit())
    })
}

pub fn class_list(ds: &DesignSystem<'_>) -> std::vec::Vec<ClassEntry> {
    class_list_keeping(ds, |_| true)
}

/// [`class_list`] with only the modifiers `keep` accepts.
pub fn class_list_keeping(ds: &DesignSystem<'_>, keep: impl Fn(&str) -> bool) -> std::vec::Vec<ClassEntry> {
    let mut list = class_items(ds, keep);
    list.sort_by(|(a, _), (b, _)| compare(a, b));
    sort_fractions_last(list)
}

/// [`class_list_keeping`] in completion order, without the sort only collapsing needs.
pub fn class_list_unsorted(ds: &DesignSystem<'_>, keep: impl Fn(&str) -> bool) -> std::vec::Vec<ClassEntry> {
    class_items(ds, keep)
        .into_iter()
        .map(|(name, item)| ClassEntry { name, utility: item.utility, modifiers: item.modifiers.into_iter().collect() })
        .collect()
}

fn class_items(ds: &DesignSystem<'_>, keep: impl Fn(&str) -> bool) -> std::vec::Vec<(String, ClassItem)> {
    let mut items: IndexMap<String, ClassItem, FxBuildHasher> = IndexMap::default();
    fn item(items: &mut IndexMap<String, ClassItem, FxBuildHasher>, name: String) -> &mut ClassItem {
        items.entry(name.clone()).or_insert_with(|| ClassItem { utility: name, fraction: false, modifiers: IndexSet::default() })
    }
    for utility in ds.utilities.keys(UtilityKind::Static) {
        if ds.utilities.completions(utility, &ds.theme).is_empty() {
            continue;
        }
        let entry = item(&mut items, utility.to_owned());
        entry.fraction = false;
        entry.modifiers.clear();
    }
    for utility in ds.utilities.keys(UtilityKind::Functional) {
        for group in ds.utilities.completions_filtered(utility, &ds.theme, &keep) {
            let modifiers: std::vec::Vec<&String> = group.modifiers.iter().filter(|m| keep(m)).collect();
            for value in &group.values {
                let fraction = value.as_deref().is_some_and(is_fraction);
                let name = match value {
                    Some(value) => format!("{utility}-{value}"),
                    None => utility.to_owned(),
                };
                let negative = group.supports_negative.then(|| format!("-{name}"));
                let entry = item(&mut items, name);
                entry.utility = utility.to_owned();
                entry.fraction |= fraction;
                entry.modifiers.extend(modifiers.iter().map(|m| (*m).clone()));
                if let Some(negative) = negative {
                    let entry = item(&mut items, negative);
                    entry.utility = format!("-{utility}");
                    entry.fraction |= fraction;
                    entry.modifiers.extend(modifiers.iter().map(|m| (*m).clone()));
                }
            }
        }
    }

    items.into_iter().collect()
}

fn sort_fractions_last(list: std::vec::Vec<(String, ClassItem)>) -> std::vec::Vec<ClassEntry> {
    let mut buckets: std::vec::Vec<(String, std::vec::Vec<ClassEntry>)> = std::vec::Vec::new();
    let mut last_bucket: IndexMap<String, usize, FxBuildHasher> = IndexMap::default();
    let mut fractions: IndexMap<String, std::vec::Vec<ClassEntry>, FxBuildHasher> = IndexMap::default();
    for (name, item) in list {
        if buckets.last().is_none_or(|(utility, _)| *utility != item.utility) {
            buckets.push((item.utility.clone(), std::vec::Vec::new()));
            last_bucket.insert(item.utility.clone(), buckets.len() - 1);
        }
        let entry = ClassEntry { name, utility: item.utility.clone(), modifiers: item.modifiers.into_iter().collect() };
        if item.fraction {
            fractions.entry(item.utility).or_default().push(entry);
        } else if let Some((_, items)) = buckets.last_mut() {
            items.push(entry);
        }
    }
    for (utility, items) in fractions {
        if let Some(&idx) = last_bucket.get(&utility) {
            buckets[idx].1.extend(items);
        }
    }
    buckets.into_iter().flat_map(|(_, items)| items).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantInfo<'a> {
    pub name: &'a str,
    pub is_arbitrary: bool,
    pub values: std::vec::Vec<String>,
    pub has_dash: bool,
}

pub fn variants<'a>(ds: &DesignSystem<'a>) -> std::vec::Vec<VariantInfo<'a>> {
    ds.variants
        .entries()
        .map(|(root, entry)| VariantInfo {
            name: root,
            is_arbitrary: entry.kind != VariantKind::Static,
            values: ds.variants.completions(root, &ds.theme),
            has_dash: root != "@",
        })
        .collect()
}

/// Selectors produced by `root[-value][/modifier]`, as v3-style nested strings.
pub fn variant_selectors(ds: &mut Session<'_, '_>, root: &str, value: Option<&str>, modifier: Option<&str>) -> std::vec::Vec<String> {
    let mut name = root.to_owned();
    if let Some(value) = value.filter(|v| !v.is_empty()) {
        if root != "@" {
            name.push('-');
        }
        name.push_str(value);
    }
    if let Some(modifier) = modifier.filter(|m| !m.is_empty()) {
        name.push('/');
        name.push_str(modifier);
    }
    let Some(variant) = ds.parse_variant(ds.alloc.str(&name)) else {
        return std::vec::Vec::new();
    };
    let mut node = Node::Rule { selector: ".__placeholder__", nodes: Vec::new_in(ds.alloc) };
    if apply_variant(&ds.cx(), &mut node, variant, &ds.variants, 0).is_none() {
        return std::vec::Vec::new();
    }
    let mut selectors = std::vec::Vec::new();
    if let Some(children) = node.children() {
        collect_selectors(children, &mut std::vec::Vec::new(), &mut selectors);
    }
    selectors
}

fn collect_selectors<'n>(nodes: &'n [Node<'_>], path: &mut std::vec::Vec<&'n Node<'n>>, out: &mut std::vec::Vec<String>) {
    for node in nodes {
        let children = node.children().map_or(&[][..], |c| c.as_slice());
        if !children.is_empty() {
            path.push(node);
            collect_selectors(children, path, out);
            path.pop();
            continue;
        }
        if !matches!(node, Node::Rule { .. } | Node::AtRule { .. }) {
            continue;
        }
        let mut full: std::vec::Vec<&Node<'_>> = path.clone();
        full.push(node);
        full.sort_by_key(|n| !matches!(n, Node::AtRule { .. }));
        let group: std::vec::Vec<String> = full
            .iter()
            .filter_map(|n| match n {
                Node::Rule { selector, .. } if *selector != "&" => Some((*selector).to_owned()),
                Node::AtRule { name, params, .. } => Some(format!("{name} {params}")),
                _ => None,
            })
            .collect();
        let mut selector = String::new();
        for part in group.iter().rev() {
            selector = if selector.is_empty() { part.clone() } else { format!("{part} {{ {selector} }}") };
        }
        out.push(selector);
    }
}
