// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::parse::{Candidate, CandidateKind, Modifier, UtilityValue, Variant, VariantValue};
use orisu_allocator::Allocator;
use orisu_parser::value::{self as vp, ValueNode};

pub fn print_candidate(candidate: &Candidate<'_>, prefix: Option<&str>) -> String {
    print_candidate_with(candidate, prefix, &Allocator::new())
}

/// [`print_candidate`] with the arena the value parsing borrows from.
pub fn print_candidate_with(candidate: &Candidate<'_>, prefix: Option<&str>, alloc: &Allocator) -> String {
    let mut out = String::new();
    if let Some(prefix) = prefix.filter(|p| !p.is_empty()) {
        out.push_str(prefix);
        out.push(':');
    }
    for variant in candidate.variants.iter().rev() {
        write_variant(variant, &mut out, alloc);
        out.push(':');
    }
    match candidate.kind {
        CandidateKind::Static { root } => out.push_str(root),
        CandidateKind::Functional { root, value, modifier } => {
            out.push_str(root);
            match value {
                Some(UtilityValue::Arbitrary { data_type, value }) => {
                    let (open, close, value) = bracketed(value, alloc);
                    out.push('-');
                    out.push(open);
                    if let Some(data_type) = data_type {
                        out.push_str(data_type);
                        out.push(':');
                    }
                    out.push_str(&print_arbitrary_value(value, alloc));
                    out.push(close);
                }
                Some(UtilityValue::Named { value, .. }) => {
                    out.push('-');
                    out.push_str(value);
                }
                None => {}
            }
            write_modifier(modifier, &mut out, alloc);
        }
        CandidateKind::Arbitrary { property, value, modifier } => {
            out.push('[');
            out.push_str(property);
            out.push(':');
            out.push_str(&print_arbitrary_value(value, alloc));
            out.push(']');
            write_modifier(modifier, &mut out, alloc);
        }
    }
    if candidate.important {
        out.push('!');
    }
    out
}

pub fn print_variant(variant: &Variant<'_>) -> String {
    print_variant_with(variant, &Allocator::new())
}

/// [`print_variant`] with the arena the value parsing borrows from.
pub fn print_variant_with(variant: &Variant<'_>, alloc: &Allocator) -> String {
    let mut out = String::new();
    write_variant(variant, &mut out, alloc);
    out
}

pub fn print_modifier(modifier: Modifier<'_>) -> String {
    print_modifier_with(modifier, &Allocator::new())
}

/// [`print_modifier`] with the arena the value parsing borrows from.
pub fn print_modifier_with(modifier: Modifier<'_>, alloc: &Allocator) -> String {
    let mut out = String::new();
    write_modifier(Some(modifier), &mut out, alloc);
    out
}

fn write_variant(variant: &Variant<'_>, out: &mut String, alloc: &Allocator) {
    match *variant {
        Variant::Static { root } => out.push_str(root),
        Variant::Arbitrary { selector, .. } => {
            out.push('[');
            out.push_str(&print_arbitrary_value(simplify_arbitrary_variant(selector, alloc), alloc));
            out.push(']');
        }
        Variant::Functional { root, value, modifier } => {
            out.push_str(root);
            let dash = if root == "@" { "" } else { "-" };
            match value {
                Some(VariantValue::Arbitrary(value)) => {
                    let (open, close, value) = bracketed(value, alloc);
                    out.push_str(dash);
                    out.push(open);
                    out.push_str(&print_arbitrary_value(value, alloc));
                    out.push(close);
                }
                Some(VariantValue::Named(value)) => {
                    out.push_str(dash);
                    out.push_str(value);
                }
                None => {}
            }
            write_modifier(modifier, out, alloc);
        }
        Variant::Compound { root, modifier, variant } => {
            out.push_str(root);
            out.push('-');
            write_variant(variant, out, alloc);
            write_modifier(modifier, out, alloc);
        }
    }
}

fn write_modifier(modifier: Option<Modifier<'_>>, out: &mut String, alloc: &Allocator) {
    match modifier {
        Some(Modifier::Arbitrary(value)) => {
            let (open, close, value) = bracketed(value, alloc);
            out.push('/');
            out.push(open);
            out.push_str(&print_arbitrary_value(value, alloc));
            out.push(close);
        }
        Some(Modifier::Named(value)) => {
            out.push('/');
            out.push_str(value);
        }
        None => {}
    }
}

fn bracketed<'v>(value: &'v str, alloc: &Allocator) -> (char, char, &'v str) {
    let ast = vp::parse(value, alloc);
    if matches!(ast.as_slice(), [ValueNode::Function("var", _)]) { ('(', ')', &value[4..value.len() - 1]) } else { ('[', ']', value) }
}

fn simplify_arbitrary_variant<'v>(input: &'v str, alloc: &'v Allocator) -> &'v str {
    match vp::parse(input, alloc).as_slice() {
        [ValueNode::Word("&"), ValueNode::Separator(":"), ValueNode::Function("is", nodes)] => vp::to_css(nodes, alloc),
        _ => input,
    }
}

const SYMBOLS: [&str; 6] = ["~", ">", "+", "-", "*", "/"];

#[inline]
fn is_symbol(value: &str) -> bool {
    SYMBOLS.contains(&value)
}

pub fn print_arbitrary_value(input: &str, alloc: &Allocator) -> String {
    let ast = vp::parse(input, alloc);
    let mut out = String::new();
    write_value(&ast, &mut out, true, true);
    out
}

fn write_value(nodes: &[ValueNode<'_>], out: &mut String, normalize: bool, escape: bool) {
    let mut dropped = vec![false; nodes.len()];
    if normalize {
        for (idx, node) in nodes.iter().enumerate() {
            match node {
                ValueNode::Word(value) if is_symbol(value) && idx >= 1 => {
                    let previous_is_space = matches!(nodes[idx - 1], ValueNode::Separator(" "));
                    let next_is_space = matches!(nodes.get(idx + 1), Some(ValueNode::Separator(" ")));
                    if !previous_is_space || !next_is_space {
                        continue;
                    }
                    if idx >= 2 && is_symbol(nodes[idx - 2].value()) {
                        continue;
                    }
                    if nodes.get(idx + 2).is_some_and(|n| is_symbol(n.value())) {
                        continue;
                    }
                    dropped[idx - 1] = true;
                    dropped[idx + 1] = true;
                }
                ValueNode::Separator(value) if !value.is_empty() && value.trim().is_empty() && (idx == 0 || idx == nodes.len() - 1) => {
                    dropped[idx] = true;
                }
                _ => {}
            }
        }
    }
    for (idx, node) in nodes.iter().enumerate() {
        if dropped[idx] {
            continue;
        }
        match node {
            ValueNode::Word(value) => {
                if !escape || value.starts_with("--") {
                    out.push_str(value);
                } else {
                    escape_underscore(value, out);
                }
            }
            ValueNode::Separator(value) => {
                if normalize && value.trim() == "," {
                    out.push(',');
                } else if !escape {
                    out.push_str(value);
                } else {
                    escape_underscore(value, out);
                }
            }
            ValueNode::Function(name, children) => {
                let wrap = normalize
                    && name.starts_with("--")
                    && idx > 0
                    && !matches!(nodes[idx - 1], ValueNode::Separator(v) if v.trim() == ",")
                    && (idx < 2 || is_symbol(nodes[idx - 2].value()));
                if wrap {
                    out.push('(');
                }
                if escape {
                    escape_underscore(name, out);
                } else {
                    out.push_str(name);
                }
                out.push('(');
                let is_url = *name == "url" || name.ends_with("_url");
                write_value(children, out, normalize && !wrap, escape && !is_url);
                out.push(')');
                if wrap {
                    out.push(')');
                }
            }
        }
    }
}

fn escape_underscore(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '_' => out.push_str("\\_"),
            ' ' => out.push('_'),
            c => out.push(c),
        }
    }
}
