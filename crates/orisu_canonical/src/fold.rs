// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use orisu_allocator::{AllocExt, Allocator, Vec};
use orisu_parser::compare::dimension;
use orisu_parser::infer::{format_js_number, is_length};
use orisu_parser::segment::segment;
use orisu_parser::value::{self as vp, ValueNode};
use std::cmp::Ordering;

pub(super) type ValueNodes<'a> = Vec<'a, ValueNode<'a>>;

#[inline]
pub(super) fn js_round(n: f64) -> f64 {
    (n + 0.5).floor()
}

#[inline]
fn is_calc(name: &str) -> bool {
    name == "calc" || name.is_empty()
}

pub(super) fn constant_fold_declaration(input: &str, rem: Option<f64>, alloc: &Allocator) -> String {
    let mut ast = vp::parse(input, alloc);
    if constant_fold(&mut ast, rem, true, alloc) { vp::to_string(&ast) } else { input.to_owned() }
}

/// Folds `calc()` expressions with two operands and canonicalizes dimensions in place; `true` when anything changed.
pub(super) fn constant_fold<'a>(ast: &mut ValueNodes<'a>, rem: Option<f64>, normalize_unit: bool, alloc: &'a Allocator) -> bool {
    let mut folded = false;
    fold_nodes(ast, false, rem, normalize_unit, alloc, &mut folded);
    folded
}

fn fold_nodes<'a>(
    nodes: &mut ValueNodes<'a>,
    in_function: bool,
    rem: Option<f64>,
    normalize: bool,
    alloc: &'a Allocator,
    folded: &mut bool,
) {
    for i in 0..nodes.len() {
        if let ValueNode::Function(_, children) = &mut nodes[i] {
            fold_nodes(children, true, rem, normalize, alloc, folded);
        }
        if let Some(replacement) = fold_node(&nodes[i], in_function, rem, normalize, alloc) {
            nodes[i] = replacement;
            *folded = true;
        }
    }
}

fn word<'a>(value: String, alloc: &'a Allocator) -> ValueNode<'a> {
    ValueNode::Word(alloc.str(&value))
}

fn fold_node<'a>(
    node: &ValueNode<'a>,
    in_function: bool,
    rem: Option<f64>,
    normalize: bool,
    alloc: &'a Allocator,
) -> Option<ValueNode<'a>> {
    match node {
        ValueNode::Word(value) if *value != "0" => {
            let canonical = canonicalize_dimension(value, rem, normalize)?;
            if canonical == *value {
                return None;
            }
            if canonical == "0" && in_function {
                return Some(word(canonicalize_dimension(value, rem, false)?, alloc));
            }
            Some(word(canonical, alloc))
        }
        ValueNode::Function(name, nodes) if is_calc(name) => {
            if nodes.len() != 5 {
                return None;
            }
            let ValueNode::Word(operator) = nodes[2] else {
                return None;
            };
            let (lhs_node, rhs_node) = (&nodes[0], &nodes[4]);
            let lhs = as_dimension(lhs_node);
            let rhs = as_dimension(rhs_node);
            let unitless_zero = |d: Option<(f64, Option<&str>)>| d.is_some_and(|(v, u)| v == 0.0 && u.is_none());

            if operator == "*" && (unitless_zero(lhs) || unitless_zero(rhs)) {
                return Some(ValueNode::Word("0"));
            }
            if operator == "*" {
                let zero_unit = |zero: Option<(f64, Option<&'a str>)>, other: Option<(f64, Option<&str>)>| match (zero, other) {
                    (Some((0.0, Some(unit))), Some((_, None))) => Some(unit),
                    _ => None,
                };
                if let Some(unit) = zero_unit(lhs, rhs).or_else(|| zero_unit(rhs, lhs)) {
                    return Some(if in_function { word(format!("0{unit}"), alloc) } else { ValueNode::Word("0") });
                }
                if lhs == Some((1.0, None)) {
                    return Some(rhs_node.clone_in(alloc));
                }
                if rhs == Some((1.0, None)) {
                    return Some(lhs_node.clone_in(alloc));
                }
            }

            if operator == "*" || operator == "+" {
                let constant = lhs.or(rhs);
                let nested = if lhs.is_none() {
                    Some(lhs_node)
                } else if rhs.is_none() {
                    Some(rhs_node)
                } else {
                    None
                };
                if let (Some(constant), Some(ValueNode::Function(nested_name, nested))) = (constant, nested)
                    && is_calc(nested_name)
                    && nested.len() == 5
                    && matches!(nested[2], ValueNode::Word(op) if op == operator)
                {
                    let nested_lhs = as_dimension(&nested[0]);
                    let nested_rhs = as_dimension(&nested[4]);
                    let known = nested_lhs.or(nested_rhs);
                    let unknown = if nested_lhs.is_none() {
                        Some(&nested[0])
                    } else if nested_rhs.is_none() {
                        Some(&nested[4])
                    } else {
                        None
                    };
                    if let (Some(known), Some(unknown)) = (known, unknown) {
                        if operator == "*" && constant.1.is_some() && known.1.is_some() {
                            return None;
                        }
                        if operator == "+" && constant.1 != known.1 {
                            return None;
                        }
                        let number = if operator == "*" { constant.0 * known.0 } else { constant.0 + known.0 };
                        let combined = format!("{}{}", format_js_number(number), constant.1.or(known.1).unwrap_or(""));
                        if operator == "*" && combined == "1" {
                            return Some(unknown.clone_in(alloc));
                        }
                        let mut replacement = Vec::with_capacity_in(5, alloc);
                        replacement.push(word(combined, alloc));
                        replacement.push(nodes[1].clone_in(alloc));
                        replacement.push(nodes[2].clone_in(alloc));
                        replacement.push(nodes[3].clone_in(alloc));
                        replacement.push(unknown.clone_in(alloc));
                        return Some(ValueNode::Function(name, replacement));
                    }
                }
            }

            let ((lv, lu), (rv, ru)) = (lhs?, rhs?);
            let folded = match operator {
                "*" if lu == ru || lu.is_none() || ru.is_none() => Some((lv * rv, lu.or(ru))),
                "+" if lu == ru => Some((lv + rv, lu)),
                "-" if lu == ru => Some((lv - rv, lu)),
                "/" if rv != 0.0 && ru.is_none() => {
                    let computed = lv / rv;
                    (js_round(computed * 100.0) / 100.0 == computed).then_some((computed, lu))
                }
                _ => None,
            }?;
            Some(word(format!("{}{}", format_js_number(folded.0), folded.1.unwrap_or("")), alloc))
        }
        _ => None,
    }
}

#[inline]
pub(super) fn as_dimension<'a>(node: &ValueNode<'a>) -> Option<(f64, Option<&'a str>)> {
    match node {
        ValueNode::Word(value) => dimension(value),
        _ => None,
    }
}

fn canonicalize_dimension(input: &str, rem: Option<f64>, normalize_unit: bool) -> Option<String> {
    let (value, unit) = dimension(input)?;
    let Some(unit) = unit else {
        return Some(format_js_number(value));
    };
    if value == 0.0 && is_length(input) {
        return Some(if normalize_unit { "0".to_owned() } else { format!("0{unit}") });
    }
    if !normalize_unit {
        return Some(input.to_owned());
    }
    let n = format_js_number;
    Some(match unit.to_ascii_lowercase().as_str() {
        "in" => format!("{}px", n(value * 96.0)),
        "cm" => format!("{}px", n(value * 96.0 / 2.54)),
        "mm" => format!("{}px", n(value * 96.0 / 2.54 / 10.0)),
        "q" => format!("{}px", n(value * 96.0 / 2.54 / 10.0 / 4.0)),
        "pc" => format!("{}px", n(value * 96.0 / 6.0)),
        "pt" => format!("{}px", n(value * 96.0 / 72.0)),
        "rem" => format!("{}px", n(value * rem?)),
        "grad" => format!("{}deg", n(value * 0.9)),
        "rad" => format!("{}deg", n(value * 180.0 / std::f64::consts::PI)),
        "turn" => format!("{}deg", n(value * 360.0)),
        "ms" => format!("{}s", n(value / 1000.0)),
        "khz" => format!("{}hz", n(value * 1000.0)),
        _ => format!("{}{unit}", n(value)),
    })
}

/// Orders the operands of commutative `calc()` expressions so equivalent expressions print the same; `true` when anything changed.
pub(super) fn canonicalize_calc_expressions(ast: &mut ValueNodes<'_>) -> bool {
    let mut changed = false;
    for node in ast.iter_mut() {
        let ValueNode::Function(name, nodes) = node else {
            continue;
        };
        changed |= canonicalize_calc_expressions(nodes);
        if is_calc(name) && nodes.len() == 5 && matches!(nodes[2], ValueNode::Word("*" | "+")) && should_swap(&nodes[0], &nodes[4]) {
            nodes.swap(0, 4);
            changed = true;
        }
    }
    changed
}

fn should_swap(lhs: &ValueNode<'_>, rhs: &ValueNode<'_>) -> bool {
    match (as_dimension(lhs), as_dimension(rhs)) {
        (Some(_), None) => return true,
        (None, Some(_)) => return false,
        (Some((lv, lu)), Some((rv, ru))) => {
            if lu.is_none() && ru.is_some() {
                return true;
            }
            if lu.is_some() && ru.is_none() {
                return false;
            }
            if lv != rv {
                return lv - rv > 0.0;
            }
            if lu != ru {
                return locale_compare(lu.unwrap_or(""), ru.unwrap_or("")) == Ordering::Greater;
            }
        }
        (None, None) => {}
    }
    locale_compare(&vp::to_string(std::slice::from_ref(lhs)), &vp::to_string(std::slice::from_ref(rhs))) == Ordering::Greater
}

const PUNCTUATION: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

const ASCII_PRIMARY: [(u8, u32); 128] = {
    let mut table = [(4, 0); 128];
    let mut c = 0;
    while c < 128 {
        table[c] = ascii_primary(c as u8);
        c += 1;
    }
    table
};

const fn ascii_primary(c: u8) -> (u8, u32) {
    if matches!(c, b'\t'..=b'\r' | b' ') {
        return (0, c as u32);
    }
    let punctuation = PUNCTUATION.as_bytes();
    let mut i = 0;
    while i < punctuation.len() {
        if punctuation[i] == c {
            return (1, i as u32);
        }
        i += 1;
    }
    if c.is_ascii_digit() {
        return (2, c as u32);
    }
    if c.is_ascii_alphabetic() {
        return (3, c.to_ascii_lowercase() as u32);
    }
    (4, c as u32)
}

fn primary(c: char) -> (u8, u32) {
    if c.is_ascii() {
        return ASCII_PRIMARY[c as usize];
    }
    (if c.is_whitespace() { 0 } else { 4 }, c as u32)
}

/// Approximates `String.prototype.localeCompare` (ICU root collation) for the ASCII strings CSS produces.
pub(super) fn locale_compare(a: &str, b: &str) -> Ordering {
    a.chars()
        .map(primary)
        .cmp(b.chars().map(primary))
        .then_with(|| a.chars().map(|c| c.is_ascii_uppercase()).cmp(b.chars().map(|c| c.is_ascii_uppercase())))
        .then_with(|| a.cmp(b))
}

/// Longhand names, and per argument count which argument each longhand takes.
type Quad = (&'static [&'static str], &'static [&'static [usize]]);

const QUAD: &[&[usize]] = &[&[0, 0, 0, 0], &[0, 1, 0, 1], &[0, 1, 2, 1], &[0, 1, 2, 3]];
const PAIR: &[&[usize]] = &[&[0, 0], &[0, 1]];

const fn quad(names: &'static [&'static str; 4]) -> Quad {
    (names, QUAD)
}

const fn pair(names: &'static [&'static str; 2]) -> Quad {
    (names, PAIR)
}

const VARIADIC: &[(&str, Quad)] = &[
    ("inset", quad(&["top", "right", "bottom", "left"])),
    ("margin", quad(&["margin-top", "margin-right", "margin-bottom", "margin-left"])),
    ("padding", quad(&["padding-top", "padding-right", "padding-bottom", "padding-left"])),
    ("scroll-margin", quad(&["scroll-margin-top", "scroll-margin-right", "scroll-margin-bottom", "scroll-margin-left"])),
    ("scroll-padding", quad(&["scroll-padding-top", "scroll-padding-right", "scroll-padding-bottom", "scroll-padding-left"])),
    ("border-width", quad(&["border-top-width", "border-right-width", "border-bottom-width", "border-left-width"])),
    ("border-style", quad(&["border-top-style", "border-right-style", "border-bottom-style", "border-left-style"])),
    ("border-color", quad(&["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"])),
    ("gap", pair(&["row-gap", "column-gap"])),
    ("overflow", pair(&["overflow-x", "overflow-y"])),
    ("overscroll-behavior", pair(&["overscroll-behavior-x", "overscroll-behavior-y"])),
];

const VARIADIC_LOGICAL: &[(&str, Quad)] = &[
    ("inset-block", pair(&["top", "bottom"])),
    ("inset-inline", pair(&["left", "right"])),
    ("margin-block", pair(&["margin-top", "margin-bottom"])),
    ("margin-inline", pair(&["margin-left", "margin-right"])),
    ("padding-block", pair(&["padding-top", "padding-bottom"])),
    ("padding-inline", pair(&["padding-left", "padding-right"])),
    ("scroll-margin-block", pair(&["scroll-margin-top", "scroll-margin-bottom"])),
    ("scroll-margin-inline", pair(&["scroll-margin-left", "scroll-margin-right"])),
    ("scroll-padding-block", pair(&["scroll-padding-top", "scroll-padding-bottom"])),
    ("scroll-padding-inline", pair(&["scroll-padding-left", "scroll-padding-right"])),
];

const LOGICAL: &[(&str, &[&str])] = &[
    ("border-block", &["border-bottom", "border-top"]),
    ("border-block-color", &["border-bottom-color", "border-top-color"]),
    ("border-block-style", &["border-bottom-style", "border-top-style"]),
    ("border-block-width", &["border-bottom-width", "border-top-width"]),
    ("border-inline", &["border-left", "border-right"]),
    ("border-inline-color", &["border-left-color", "border-right-color"]),
    ("border-inline-style", &["border-left-style", "border-right-style"]),
    ("border-inline-width", &["border-left-width", "border-right-width"]),
];

fn expand_variadic<'v>(map: &[(&str, Quad)], property: &str, value: &'v str) -> Option<Option<std::vec::Vec<(&'static str, &'v str)>>> {
    let (_, (names, patterns)) = map.iter().find(|(p, _)| *p == property)?;
    let args = segment(value, b' ');
    let pattern = patterns.get(args.len().wrapping_sub(1));
    Some(pattern.map(|pattern| names.iter().zip(pattern.iter()).map(|(name, &i)| (*name, args[i])).collect()))
}

/// Splits shorthands into their longhands, `None` when `property` is not expandable with `value`.
pub(super) fn expand_declaration<'v>(
    property: &str,
    value: &'v str,
    logical_to_physical: bool,
) -> Option<std::vec::Vec<(&'static str, &'v str)>> {
    if logical_to_physical {
        if let Some(expanded) = expand_variadic(VARIADIC_LOGICAL, property, value) {
            return expanded;
        }
        if let Some((_, props)) = LOGICAL.iter().find(|(p, _)| *p == property) {
            return Some(props.iter().map(|p| (*p, value)).collect());
        }
    }
    expand_variadic(VARIADIC, property, value).flatten()
}
