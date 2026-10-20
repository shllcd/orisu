// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use super::Canon;
use super::convert::Convert;
use orisu_allocator::{AllocExt, Allocator, Vec};
use orisu_candidate::print::print_variant;
use orisu_candidate::{Registry, Variant, VariantValue};
use orisu_parser::attribute_selector;
use orisu_parser::infer::is_positive_integer;
use orisu_parser::segment::segment;
use orisu_parser::selector::{self as sp, SNode};
use orisu_parser::value::{self as vp, ValueNode};

pub(super) type Variants<'a> = std::vec::Vec<&'a Variant<'a>>;

type Pass = for<'a, 'd> fn(&mut Canon<'a, 'd>, &'a Variant<'a>) -> Variants<'a>;

pub(super) const VARIANT_PASSES: &[Pass] = &[theme_to_var, arbitrary_value_to_bare_value, modernize_arbitrary_values, arbitrary_variants];

type Step<'a> = (Option<&'a Variant<'a>>, &'a Variant<'a>);

/// Applies `f` to `variant`, then to the variant nested in its result. `f` sees `true` for the outermost
/// variant; a prefix variant returned by `f` ends the walk.
fn walk<'a>(alloc: &'a Allocator, variant: &'a Variant<'a>, top: bool, f: &mut dyn FnMut(&'a Variant<'a>, bool) -> Step<'a>) -> Step<'a> {
    let (prefix, variant) = f(variant, top);
    if prefix.is_some() {
        return (prefix, variant);
    }
    let Variant::Compound { root, modifier, variant: inner } = *variant else {
        return (None, variant);
    };
    let (_, next) = walk(alloc, inner, false, f);
    if std::ptr::eq(next, inner) { (None, variant) } else { (None, alloc.alloc(Variant::Compound { root, modifier, variant: next })) }
}

fn map<'a>(alloc: &'a Allocator, variant: &'a Variant<'a>, f: &mut dyn FnMut(&'a Variant<'a>) -> &'a Variant<'a>) -> Variants<'a> {
    vec![walk(alloc, variant, true, &mut |v, _| (None, f(v))).1]
}

fn theme_to_var<'a>(cx: &mut Canon<'a, '_>, variant: &'a Variant<'a>) -> Variants<'a> {
    let alloc = cx.ds.alloc;
    map(alloc, variant, &mut |v| match *v {
        Variant::Arbitrary { selector, relative } => {
            let (converted, _) = cx.convert(selector, Convert::ThemeOnly);
            if converted == selector { v } else { alloc.alloc(Variant::Arbitrary { selector: alloc.str(&converted), relative }) }
        }
        Variant::Functional { root, value: Some(VariantValue::Arbitrary(value)), modifier } => {
            let (converted, _) = cx.convert(value, Convert::ThemeOnly);
            if converted == value {
                v
            } else {
                alloc.alloc(Variant::Functional { root, value: Some(VariantValue::Arbitrary(alloc.str(&converted))), modifier })
            }
        }
        _ => v,
    })
}

fn is_supports_ident(value: &str) -> bool {
    let b = value.as_bytes();
    b.first().is_some_and(|c| c.is_ascii_alphabetic() || *c == b'-') && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
}

fn arbitrary_value_to_bare_value<'a>(cx: &mut Canon<'a, '_>, variant: &'a Variant<'a>) -> Variants<'a> {
    let alloc = cx.ds.alloc;
    map(alloc, variant, &mut |v| {
        let Variant::Functional { root, value: Some(VariantValue::Arbitrary(value)), modifier } = *v else {
            return v;
        };
        let named = match root {
            "data" if !value.contains('=') => value,
            "aria" if value.ends_with("=true") || value.ends_with("=\"true\"") || value.ends_with("='true'") => {
                let key = segment(value, b'=').first().copied().unwrap_or("");
                if key.ends_with(['~', '|', '^', '$', '*']) {
                    return v;
                }
                &value[..value.find('=').unwrap_or(value.len())]
            }
            "supports" if is_supports_ident(value) => value,
            _ => return v,
        };
        alloc.alloc(Variant::Functional { root, value: Some(VariantValue::Named(named)), modifier })
    })
}

fn arbitrary_variants<'a>(cx: &mut Canon<'a, '_>, variant: &'a Variant<'a>) -> Variants<'a> {
    map(cx.ds.alloc, variant, &mut |v| {
        if matches!(v, Variant::Compound { .. }) {
            return v;
        }
        let signature = cx.variant_signature(&print_variant(v));
        if !signature.is_valid() {
            return v;
        }
        match *cx.precomputed_variants().get(&signature).map_or(&[][..], std::vec::Vec::as_slice) {
            [found] => cx.ds.parse_variant(found).unwrap_or(v),
            _ => v,
        }
    })
}

fn modernize_arbitrary_values<'a>(cx: &mut Canon<'a, '_>, variant: &'a Variant<'a>) -> Variants<'a> {
    let alloc = cx.ds.alloc;
    match walk(alloc, variant, true, &mut |v, top| modernize(cx, v, top)) {
        (Some(prefix), variant) => vec![prefix, variant],
        (None, variant) => vec![variant],
    }
}

fn modernize<'a>(cx: &mut Canon<'a, '_>, variant: &'a Variant<'a>, top: bool) -> Step<'a> {
    let alloc = cx.ds.alloc;
    match *variant {
        Variant::Compound { root: root @ ("has" | "not" | "in"), modifier: Some(modifier), variant: inner } => {
            let forwarded = match *inner {
                Variant::Functional { root, value, .. } => Variant::Functional { root, value, modifier: Some(modifier) },
                Variant::Compound { root, variant, .. } => Variant::Compound { root, modifier: Some(modifier), variant },
                _ => return (None, variant),
            };
            (None, alloc.alloc(Variant::Compound { root, modifier: None, variant: alloc.alloc(forwarded) }))
        }
        Variant::Arbitrary { selector, relative: false } => modernize_selector(cx, variant, selector, top).unwrap_or((None, variant)),
        _ => (None, variant),
    }
}

fn is_attribute(value: &str) -> bool {
    value.starts_with('[') && value.ends_with(']')
}

fn is_selector(node: &SNode<'_>, value: &str) -> bool {
    matches!(node, SNode::Selector(v) if *v == value)
}

fn is_combinator(node: &SNode<'_>, value: &str) -> bool {
    matches!(node, SNode::Combinator(v) if *v == value)
}

fn nested(ast: &[SNode<'_>], top: bool, combinator: &str, target: &dyn Fn(&SNode<'_>) -> bool) -> bool {
    top && ast.len() == 3 && is_selector(&ast[0], "&") && is_combinator(&ast[1], combinator) && target(&ast[2])
}

fn modernize_selector<'a>(cx: &mut Canon<'a, '_>, variant: &'a Variant<'a>, selector: &str, top: bool) -> Option<Step<'a>> {
    let alloc = cx.ds.alloc;
    let mut ast = sp::parse(selector, alloc);
    if matches!(ast.as_slice(), [SNode::List(_)]) {
        return None;
    }
    if let [SNode::Complex(nodes)] = ast.as_mut_slice() {
        let nodes = std::mem::replace(nodes, Vec::new_in(alloc));
        ast = nodes;
    }

    if nested(&ast, top, ">", &|n| is_selector(n, "*")) {
        return Some((None, parse(cx, "*")?));
    }
    if nested(&ast, top, " ", &|n| is_selector(n, "*")) {
        return Some((None, parse(cx, "**")?));
    }
    if top
        && let [SNode::Compound(nodes)] = ast.as_slice()
        && let [first, SNode::Function(":has", inner)] = nodes.as_slice()
        && is_selector(first, "&")
        && matches!(inner.as_slice(), [SNode::Selector(_)])
    {
        return Some((None, parse(cx, &format!("has-[{}]", sp::to_string(inner, true)))?));
    }
    if top && ast.len() == 3 && matches!(ast[0], SNode::Selector(_)) && is_combinator(&ast[1], " ") && is_selector(&ast[2], "&") {
        return Some((None, parse(cx, &format!("in-[{}]", sp::to_string(&ast[..1], true)))?));
    }
    if top && matches!(ast.first(), Some(SNode::Selector("@media" | "@supports"))) {
        let target = cx.variant_signature(&print_variant(variant));
        let mut parsed = vp::parse(&sp::to_string(&ast, true), alloc);
        let mut contains_not = false;
        remove_not(&mut parsed, &mut contains_not);
        let mut parsed = vp::parse(&vp::to_string(&parsed), alloc);
        collapse_whitespace(&mut parsed);
        if contains_not {
            let hoisted = parse(cx, &format!("not-[{}]", vp::to_string(&parsed)))?;
            if target == cx.variant_signature(&print_variant(hoisted)) {
                return Some((None, hoisted));
            }
        }
    }

    let pseudo_or_attribute = |n: &SNode<'_>| matches!(n, SNode::Selector(v) if v.starts_with(':') || is_attribute(v));
    let mut prefix = None;
    for (combinator, prefix_variant) in [(">", "*"), (" ", "**")] {
        if nested(&ast, top, combinator, &pseudo_or_attribute) {
            let node = ast.swap_remove(2);
            ast.clear();
            ast.push(node);
            prefix = parse(cx, prefix_variant);
        }
    }

    strip_nesting(&mut ast);
    let [target] = ast.as_slice() else {
        return None;
    };
    let mut target = target;
    if let SNode::Function(":is", nodes) = target {
        let [inner] = nodes.as_slice() else {
            return None;
        };
        if matches!(inner, SNode::List(_)) || matches!(inner, SNode::Selector(v) if !is_attribute(v)) {
            return None;
        }
        target = inner;
    }

    let replacement = match target {
        SNode::Function(value, _) | SNode::Selector(value) if value.starts_with(':') => {
            let mut target_node = target;
            let mut compound_not = false;
            if let SNode::Function(":not", nodes) = target {
                compound_not = true;
                let [inner] = nodes.as_slice() else {
                    return None;
                };
                if !matches!(inner, SNode::Selector(v) | SNode::Function(v, _) if v.starts_with(':')) {
                    return None;
                }
                target_node = inner;
            }
            match pseudo_variant(cx, variant, target_node, &mut compound_not) {
                None => {
                    let prefix = prefix?;
                    return Some((Some(prefix), alloc.alloc(Variant::Arbitrary { selector: value, relative: false })));
                }
                Some(name) => parse(cx, &if compound_not { format!("not-{name}") } else { name })?,
            }
        }
        SNode::Selector(value) if is_attribute(value) => {
            let attribute = attribute_selector::parse(value)?;
            let rebuilt = |name: &str| {
                let quote = attribute.quote.map(String::from).unwrap_or_default();
                let sensitivity = attribute.sensitivity.map(|s| format!(" {s}")).unwrap_or_default();
                let (operator, value) = (attribute.operator.unwrap_or(""), attribute.value.unwrap_or(""));
                VariantValue::Arbitrary(alloc.str(&format!("{name}{operator}{quote}{value}{quote}{sensitivity}")))
            };
            let replacement = if let Some(name) = attribute.attribute.strip_prefix("data-") {
                let value = if attribute.value.is_none() { VariantValue::Named(name) } else { rebuilt(name) };
                Variant::Functional { root: "data", value: Some(value), modifier: None }
            } else if let Some(name) = attribute.attribute.strip_prefix("aria-") {
                let value = match attribute.value {
                    None => VariantValue::Arbitrary(name),
                    Some("true") if attribute.operator == Some("=") && attribute.sensitivity.is_none() => VariantValue::Named(name),
                    Some(_) => rebuilt(attribute.attribute),
                };
                Variant::Functional { root: "aria", value: Some(value), modifier: None }
            } else {
                Variant::Arbitrary { selector: value, relative: false }
            };
            alloc.alloc(replacement)
        }
        _ => variant,
    };
    Some((prefix, replacement))
}

fn parse<'a>(cx: &mut Canon<'a, '_>, input: &str) -> Option<&'a Variant<'a>> {
    let input = cx.ds.alloc.str(input);
    cx.ds.parse_variant(input)
}

const NTH: [(&str, &str); 4] =
    [(":nth-child", "nth"), (":nth-last-child", "nth-last"), (":nth-of-type", "nth-of-type"), (":nth-last-of-type", "nth-of-last-type")];

fn pseudo_variant<'a>(cx: &mut Canon<'a, '_>, variant: &'a Variant<'a>, target: &SNode<'_>, compound_not: &mut bool) -> Option<String> {
    let value = match target {
        SNode::Function(value, _) | SNode::Selector(value) => *value,
        _ => return None,
    };
    if let SNode::Function(":nth-child", nodes) = target
        && let [SNode::Value(keyword @ ("odd" | "even"))] = nodes.as_slice()
    {
        let flipped = if *keyword == "odd" { "even" } else { "odd" };
        let name = if *compound_not { flipped } else { keyword };
        *compound_not = false;
        return Some(name.to_owned());
    }
    if let SNode::Function(_, nodes) = target
        && nodes.len() == 1
        && let Some((_, name)) = NTH.iter().find(|(selector, _)| *selector == value)
    {
        return Some(match &nodes[0] {
            SNode::Value(v) if is_positive_integer(v) => format!("{name}-{v}"),
            _ => format!("{name}-[{}]", sp::to_string(nodes, true)),
        });
    }
    if *compound_not && cx.variant_signature(&print_variant(variant)) == cx.variant_signature(&format!("not-[{value}]")) {
        return Some(format!("[&{value}]"));
    }
    None
}

fn strip_nesting(nodes: &mut Vec<'_, SNode<'_>>) {
    let mut i = 0;
    while i < nodes.len() {
        if is_selector(&nodes[i], "&") {
            nodes.remove(i);
            continue;
        }
        if !matches!(nodes[i], SNode::Function(..)) {
            if let Some(children) = nodes[i].children_mut() {
                strip_nesting(children);
            }
            if let SNode::Compound(children) = &mut nodes[i]
                && children.len() == 1
            {
                let child = children.pop();
                nodes.splice(i..=i, child);
            }
        }
        i += 1;
    }
}

fn remove_not(nodes: &mut Vec<'_, ValueNode<'_>>, found: &mut bool) {
    let mut i = 0;
    while i < nodes.len() {
        if matches!(nodes[i], ValueNode::Word("not")) {
            *found = true;
            nodes.remove(i);
            continue;
        }
        if let ValueNode::Function(_, children) = &mut nodes[i] {
            remove_not(children, found);
        }
        i += 1;
    }
}

fn collapse_whitespace(nodes: &mut Vec<'_, ValueNode<'_>>) {
    for node in nodes.iter_mut() {
        match node {
            ValueNode::Separator(value) if *value != " " && value.trim().is_empty() => *value = " ",
            ValueNode::Function(_, children) => collapse_whitespace(children),
            _ => {}
        }
    }
}
