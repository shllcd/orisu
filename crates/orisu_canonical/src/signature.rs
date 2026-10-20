// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use super::fold::{canonicalize_calc_expressions, constant_fold, expand_declaration, locale_compare};
use super::{Canon, Signature, ValueKey, collect_declarations, features, hash_signature};
use orisu_allocator::{AllocExt, Allocator, Vec};
use orisu_ast::{Node, Nodes, clone_in, to_css, walk_mut};
use orisu_candidate::print::print_arbitrary_value;
use orisu_compiler::compile::compile_entries;
use orisu_compiler::compiler::replace_applies;
use orisu_compiler::css_functions::{has_theme_function, substitute_functions_in_value};
use orisu_parser::selector::{self as sp, SNode};
use orisu_parser::value::{self as vp, ValueNode};
use rustc_hash::FxHashSet;
use std::borrow::Cow;

impl<'a> Canon<'a, '_> {
    /// `.x { @apply <params> }` with the `@apply` expanded, built in `alloc`.
    fn apply_ast<'s>(&mut self, params: &str, alloc: &'s Allocator) -> Option<Nodes<'s>>
    where
        'a: 's,
    {
        let mut inner = Vec::new_in(alloc);
        if params.starts_with("--") || params.contains(char::is_whitespace) {
            let ds_alloc = self.ds.alloc;
            let mut apply = Vec::new_in(ds_alloc);
            apply.push(Node::AtRule { name: "@apply", params: ds_alloc.str(params), nodes: Vec::new_in(ds_alloc) });
            let mut ast = Vec::new_in(ds_alloc);
            ast.push(Node::Rule { selector: ".x", nodes: apply });
            replace_applies(&mut ast, &mut self.ds).ok()?;
            return Some(clone_in(&ast, alloc));
        }
        let mut valid = true;
        let entries = compile_entries([params], &mut self.ds, false, |_| valid = false);
        if !valid {
            return None;
        }
        for entry in &entries {
            match entry.node_in(alloc) {
                Node::Rule { nodes, .. } => inner.extend(nodes),
                node => inner.push(node),
            }
        }
        let mut ast = Vec::new_in(alloc);
        ast.push(Node::Rule { selector: ".x", nodes: inner });
        Some(ast)
    }

    /// The signature of `.x { @apply <utility> }` and the properties it sets, `None` when the utility is invalid.
    pub(super) fn compute_utility_signature(&mut self, utility: &str) -> Option<(Signature, std::vec::Vec<String>)> {
        let utility = match self.prefix() {
            Some(prefix) if !utility.starts_with(prefix) => format!("{prefix}:{utility}"),
            _ => utility.to_owned(),
        };
        if !utility.starts_with("--")
            && !utility.contains(char::is_whitespace)
            && let Some(declarations) = self.ds.declarations(&utility)
            && let Some(signature) = self.declarations_signature(&declarations, true)
        {
            return Some(signature);
        }
        self.with_scratch(|cx, alloc| {
            let mut ast = cx.without_inline(|cx| cx.apply_ast(&utility, alloc))?;
            cx.canonicalize_ast(&mut ast, alloc);
            Some((hash_signature(&to_css(&ast)), property_names(&ast)))
        })
    }

    /// The signature of `.x { <declarations> }`, built without parsing or compiling a utility.
    fn declarations_signature(
        &mut self,
        declarations: &[(&'a str, &'a str)],
        properties: bool,
    ) -> Option<(Signature, std::vec::Vec<String>)> {
        self.with_scratch(|cx, alloc| {
            let mut nodes = Vec::new_in(alloc);
            for &(property, value) in declarations {
                let value = if has_theme_function(value) {
                    match substitute_functions_in_value(value, false, &cx.ds) {
                        Ok(value) => value,
                        Err(_) => return None,
                    }
                } else {
                    value
                };
                nodes.push(Node::Decl { property, value: Some(value), important: false });
            }
            let mut ast = Vec::new_in(alloc);
            ast.push(Node::Rule { selector: ".x", nodes });
            cx.canonicalize_ast(&mut ast, alloc);
            let properties = if properties { property_names(&ast) } else { std::vec::Vec::new() };
            Some((hash_signature(&to_css(&ast)), properties))
        })
    }

    /// The signature of `.x { @apply <utility> }`, `None` when the utility is invalid.
    pub(super) fn compute_utility_css(&mut self, utility: &str) -> Option<Signature> {
        let utility = match self.prefix() {
            Some(prefix) if !utility.starts_with(prefix) => format!("{prefix}:{utility}"),
            _ => utility.to_owned(),
        };
        if !utility.starts_with("--")
            && !utility.contains(char::is_whitespace)
            && let Some(declarations) = self.ds.declarations(&utility)
            && let Some((signature, _)) = self.declarations_signature(&declarations, false)
        {
            return Some(signature);
        }
        self.with_scratch(|cx, alloc| {
            let mut ast = cx.without_inline(|cx| cx.apply_ast(&utility, alloc))?;
            cx.canonicalize_ast(&mut ast, alloc);
            Some(hash_signature(&to_css(&ast)))
        })
    }

    /// The properties of `.x { @apply <utility> }`, `None` when the utility is invalid.
    pub(super) fn compute_utility_properties(&mut self, utility: &str) -> Option<std::vec::Vec<String>> {
        let utility = match self.prefix() {
            Some(prefix) if !utility.starts_with(prefix) => format!("{prefix}:{utility}"),
            _ => utility.to_owned(),
        };
        self.with_scratch(|cx, alloc| {
            let mut ast = cx.without_inline(|cx| cx.apply_ast(&utility, alloc))?;
            cx.canonicalize_ast(&mut ast, alloc);
            Some(property_names(&ast))
        })
    }

    /// The signature of `.x { @apply <variant>:flex }` with selectors and at-rule params in minimal form.
    pub(super) fn compute_variant_signature(&mut self, variant: &str) -> Option<Signature> {
        let variant = match self.prefix() {
            Some(prefix) if !variant.starts_with(prefix) => format!("{prefix}:{variant}"),
            _ => variant.to_owned(),
        };
        self.with_scratch(|cx, alloc| {
            let mut ast = cx.apply_ast(&format!("{variant}:flex"), alloc)?;
            walk_mut(&mut ast, &mut |node| {
                match node {
                    Node::AtRule { params, .. } if params.contains(' ') => *params = alloc.str(&params.replace(' ', "")),
                    Node::Rule { selector, .. } => {
                        let mut selector_ast = sp::parse(selector, alloc);
                        let mut changed = false;
                        normalize_selector(&mut selector_ast, &mut changed);
                        if changed {
                            *selector = alloc.str(&sp::to_string(&selector_ast, true));
                        }
                    }
                    _ => {}
                }
                true
            });
            Some(hash_signature(&to_css(&ast)))
        })
    }

    pub(super) fn canonicalize_ast<'s>(&mut self, nodes: &mut Nodes<'s>, alloc: &'s Allocator)
    where
        'a: 's,
    {
        let mut i = 0;
        while i < nodes.len() {
            match &mut nodes[i] {
                Node::Decl { value: None, .. } | Node::Decl { property: "--tw-sort", .. } | Node::Comment(_) => {
                    nodes.remove(i);
                    continue;
                }
                Node::AtRule { name: "@property", .. } => {
                    nodes.remove(i);
                    continue;
                }
                Node::Context { nodes: children, .. } | Node::AtRoot(children) => {
                    let children = std::mem::replace(children, Vec::new_in(alloc));
                    nodes.splice(i..=i, children);
                    continue;
                }
                Node::Decl { property, value: Some(value), important } => {
                    let (property, value, important) = (*property, *value, *important);
                    if property.starts_with("--tw-")
                        && nodes.iter().any(|sibling| {
                            matches!(sibling, Node::Decl { property: p, value: Some(v), important: imp }
                                if *v == value && *imp == important && !p.starts_with("--tw-"))
                        })
                    {
                        nodes.remove(i);
                        continue;
                    }
                    if self.sig.features & features::EXPAND_PROPERTIES != 0
                        && let Some(expanded) = expand_declaration(property, value, self.sig.features & features::LOGICAL_TO_PHYSICAL != 0)
                    {
                        let replacement: std::vec::Vec<Node<'s>> = expanded
                            .into_iter()
                            .map(|(property, value)| Node::Decl { property, value: Some(alloc.str(value)), important })
                            .collect();
                        nodes.splice(i..=i, replacement);
                        continue;
                    }
                    let mut normalized = self.normalize_value(value, alloc);
                    if !property.starts_with("--") {
                        normalized = lowercase_hex(&normalized);
                    }
                    nodes[i] = Node::Decl { property, value: Some(alloc.str(&normalized)), important };
                }
                Node::Rule { nodes: children, .. } | Node::AtRule { nodes: children, .. } => {
                    self.canonicalize_ast(children, alloc);
                    dedupe_and_sort(children);
                }
            }
            i += 1;
        }
    }

    fn normalize_value(&mut self, value: &str, alloc: &Allocator) -> String {
        let sig = self.sig_key();
        let inline = self.ds.options & orisu_theme::opts::IGNORE_INLINE != 0;
        if let Some(normalized) = self.storage.normalized_values.get(&ValueKey { sig, inline, value }) {
            return normalized.clone();
        }
        let normalized = self.compute_normalized_value(value, alloc);
        let value = self.ds.alloc.str(value);
        self.storage.normalized_values.insert(ValueKey { sig, inline, value }, normalized.clone());
        normalized
    }

    fn compute_normalized_value(&self, value: &str, alloc: &Allocator) -> String {
        let mut value = if value.contains("var(") { self.resolve_variables_in_value(value, alloc) } else { value.to_owned() };
        let mut ast = vp::parse(&value, alloc);
        let folded = constant_fold(&mut ast, self.sig.rem, true, alloc);
        let normalized = canonicalize_calc_expressions(&mut ast);
        if folded || normalized {
            value = vp::to_string(&ast);
        }
        print_arbitrary_value(&value, alloc)
    }

    fn resolve_variables_in_value(&self, value: &str, alloc: &Allocator) -> String {
        let mut ast = vp::parse(value, alloc);
        let mut seen = FxHashSet::default();
        let mut changed = false;
        self.resolve_variables(&mut ast, &mut seen, &mut changed, alloc);
        if changed { vp::to_string(&ast) } else { value.to_owned() }
    }

    fn resolve_variables<'s>(
        &self,
        nodes: &mut Vec<'s, ValueNode<'s>>,
        seen: &mut FxHashSet<String>,
        changed: &mut bool,
        alloc: &'s Allocator,
    ) where
        'a: 's,
    {
        let mut i = 0;
        while i < nodes.len() {
            if let ValueNode::Function("var", children) = &mut nodes[i]
                && (children.len() == 1 || children.len() >= 3)
            {
                let raw = children[0].value();
                let variable = match self.prefix().and_then(|prefix| raw.strip_prefix("--")?.strip_prefix(prefix)?.strip_prefix('-')) {
                    Some(unprefixed) => Cow::Owned(format!("--{unprefixed}")),
                    None => Cow::Borrowed(raw),
                };
                let resolved = self.ds.resolve_theme_value(&variable, true);
                if seen.insert(variable.into_owned())
                    && let Some(resolved) = resolved
                {
                    if children.len() == 1 {
                        *changed = true;
                        children.extend(vp::parse(&format!(",{resolved}"), alloc));
                    }
                    if children.len() >= 3 && vp::to_string(children) == format!("{raw},{resolved}") {
                        *changed = true;
                        let replacement = vp::parse(resolved, alloc);
                        nodes.splice(i..=i, replacement);
                        continue;
                    }
                }
            }
            if let ValueNode::Function(_, children) = &mut nodes[i] {
                self.resolve_variables(children, seen, changed, alloc);
            }
            i += 1;
        }
    }
}

fn property_names(nodes: &[Node<'_>]) -> std::vec::Vec<String> {
    let mut declarations = std::vec::Vec::new();
    collect_declarations(nodes, &mut declarations);
    let mut properties: std::vec::Vec<String> = std::vec::Vec::new();
    for (property, _) in declarations {
        if !properties.iter().any(|p| p == property) {
            properties.push(property.to_owned());
        }
    }
    properties
}

fn dedupe_and_sort(nodes: &mut Nodes<'_>) {
    if nodes.len() > 1 {
        let mut seen = FxHashSet::default();
        let mut i = nodes.len();
        while i > 0 {
            i -= 1;
            let Node::Decl { property, value: Some(_), .. } = nodes[i] else {
                continue;
            };
            if !seen.insert(property) {
                nodes.remove(i);
            }
        }
    }
    let slots: std::vec::Vec<usize> = (0..nodes.len()).filter(|&i| matches!(nodes[i], Node::Decl { .. })).collect();
    let mut decls: std::vec::Vec<Node<'_>> = slots.iter().map(|&i| std::mem::replace(&mut nodes[i], Node::Comment(""))).collect();
    decls.sort_by(|a, b| match (a, b) {
        (Node::Decl { property: a, .. }, Node::Decl { property: b, .. }) => locale_compare(a, b),
        _ => std::cmp::Ordering::Equal,
    });
    for (slot, decl) in slots.into_iter().zip(decls) {
        nodes[slot] = decl;
    }
}

fn lowercase_hex(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            let run = bytes[i + 1..].iter().take(8).take_while(|b| b.is_ascii_hexdigit()).count();
            if let Some(len) = [8, 6, 4, 3].into_iter().find(|&n| n <= run) {
                out.push('#');
                out.push_str(&value[i + 1..i + 1 + len].to_ascii_lowercase());
                i += 1 + len;
                continue;
            }
        }
        let ch = value[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8().max(1);
    }
    out
}

fn normalize_selector<'a>(nodes: &mut Vec<'a, SNode<'a>>, changed: &mut bool) {
    let mut i = 0;
    while i < nodes.len() {
        match &mut nodes[i] {
            SNode::List(_) | SNode::Combinator(_) => *changed = true,
            SNode::Function(":is", children) if children.len() == 1 => {
                *changed = true;
                let child = children.pop();
                nodes.splice(i..=i, child);
                continue;
            }
            SNode::Function(":is", children)
                if children.len() == 2 && matches!(children[0], SNode::Selector("*")) && children[1].is_pseudo() =>
            {
                *changed = true;
                let child = children.pop();
                nodes.splice(i..=i, child);
                continue;
            }
            SNode::Function(":is", _) => {}
            SNode::Function(name, children) if name.starts_with(':') && children.first().is_some_and(SNode::is_pseudo) => {
                *changed = true;
                children.insert(0, SNode::Selector("*"));
            }
            _ => {}
        }
        if let Some(children) = nodes[i].children_mut() {
            normalize_selector(children, changed);
        }
        i += 1;
    }
}
