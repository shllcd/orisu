// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use super::{Compile, Cx, DEFAULT_SPACING_SUGGESTIONS, Suggest, Utilities, Utility, UtilityKind};
use indexmap::IndexSet;
use orisu_allocator::{AllocExt, Allocator, Vec};
use orisu_ast::{Node, Nodes, clone_in};
use orisu_candidate::{Candidate, CandidateKind, Modifier, UtilityValue};
use orisu_parser::escape::unescape;
use orisu_parser::infer::{DataType, infer_data_type, is_multiple_of_quarter, is_positive_integer, js_number};
use orisu_parser::segment::segment;
use orisu_parser::value::{self as vp, ValueNode};
use rustc_hash::FxBuildHasher;

const BARE_VALUE_DATA_TYPES: [&str; 4] = ["number", "integer", "ratio", "percentage"];

pub struct CssUtility<'a> {
    pub name: &'a str,
    pub functional: bool,
    pub body: Nodes<'a>,
}

impl<'a> CssUtility<'a> {
    /// `None` when `params` is not a valid static or functional utility name.
    pub fn new(params: &str, body: Nodes<'a>, alloc: &'a Allocator) -> Option<Self> {
        let name = alloc.str(&unescape(params));
        if is_valid_functional_utility_name(name) {
            return Some(Self { name: &name[..name.len() - 2], functional: true, body });
        }
        is_valid_static_utility_name(name).then_some(Self { name, functional: false, body })
    }

    /// Registers the utility; its body stays reachable through [`Utilities::body_mut`] at the returned index.
    pub fn register(self, utilities: &mut Utilities<'a>, alloc: &'a Allocator) -> usize {
        let Self { name, functional, mut body } = self;
        if !functional {
            return utilities.add(
                name,
                Utility {
                    kind: UtilityKind::Static,
                    compile: Compile::Css(body),
                    types: None,
                    functional: None,
                    negative: false,
                    declarations: None,
                },
            );
        }

        let mut storage = [Storage::default(), Storage::default()];
        normalize(&mut body, &mut storage, alloc);
        let template = CssTemplate::new(body, alloc);
        let index = utilities.add(
            name,
            Utility {
                kind: UtilityKind::Functional,
                compile: Compile::CssFunctional(template),
                types: None,
                functional: None,
                negative: false,
                declarations: None,
            },
        );

        let [value_storage, modifier_storage] = storage;
        let value_keys = arena_keys(&value_storage.theme_keys, alloc);
        let modifier_keys = arena_keys(&modifier_storage.theme_keys, alloc);
        let values = value_storage.suggestions(alloc);
        let modifiers = modifier_storage.suggestions(alloc);
        utilities.suggest(name, [Suggest::values(values).modifiers(modifiers).keys(value_keys).modifier_keys(modifier_keys)]);
        index
    }
}

/// A functional `@utility` body with the values of `--value()`/`--modifier()` declarations parsed once, in declaration order.
pub struct CssTemplate<'a> {
    pub body: Nodes<'a>,
    values: &'a [Option<&'a [ValueNode<'a>]>],
}

impl<'a> CssTemplate<'a> {
    pub fn new(body: Nodes<'a>, alloc: &'a Allocator) -> Self {
        let mut values = Vec::new_in(alloc);
        parse_decls(&body, &mut values, alloc);
        Self { body, values: values.into_bump_slice() }
    }
}

fn parse_decls<'a>(nodes: &Nodes<'a>, out: &mut Vec<'a, Option<&'a [ValueNode<'a>]>>, alloc: &'a Allocator) {
    for node in nodes {
        match node {
            Node::Decl { value, .. } => out
                .push(value.filter(|v| v.contains("--value(") || v.contains("--modifier(")).map(|v| vp::parse(v, alloc).into_bump_slice())),
            _ => {
                if let Some(children) = node.children() {
                    parse_decls(children, out, alloc);
                }
            }
        }
    }
}

#[derive(Default)]
struct Storage<'a> {
    used_spacing_integer: bool,
    used_spacing_number: bool,
    theme_keys: IndexSet<&'a str, FxBuildHasher>,
    literals: IndexSet<&'a str, FxBuildHasher>,
}

impl<'a> Storage<'a> {
    fn suggestions(&self, alloc: &'a Allocator) -> &'a [&'a str] {
        let mut out = Vec::new_in(alloc);
        out.extend(self.literals.iter().copied());
        if self.used_spacing_number {
            out.extend(DEFAULT_SPACING_SUGGESTIONS);
        } else if self.used_spacing_integer {
            out.extend(DEFAULT_SPACING_SUGGESTIONS.into_iter().filter(|v| is_positive_integer(v)));
        }
        out.into_bump_slice()
    }
}

fn arena_keys<'a>(keys: &IndexSet<&'a str, FxBuildHasher>, alloc: &'a Allocator) -> &'a [&'a str] {
    alloc.alloc_slice_fill_iter(keys.iter().copied())
}

#[inline]
fn storage_index(name: &str) -> Option<usize> {
    match name {
        "--value" => Some(0),
        "--modifier" => Some(1),
        _ => None,
    }
}

fn normalize<'a>(nodes: &mut Nodes<'a>, storage: &mut [Storage<'a>; 2], alloc: &'a Allocator) {
    for node in nodes.iter_mut() {
        match node {
            Node::Decl { value: Some(value), .. } if value.contains("--value(") || value.contains("--modifier(") => {
                let mut ast = vp::parse(value, alloc);
                normalize_value(&mut ast, storage, alloc);
                *value = vp::to_css(&ast, alloc);
            }
            _ => {
                if let Some(children) = node.children_mut() {
                    normalize(children, storage, alloc);
                }
            }
        }
    }
}

fn normalize_value<'a>(ast: &mut [ValueNode<'a>], storage: &mut [Storage<'a>; 2], alloc: &'a Allocator) {
    for node in ast {
        let ValueNode::Function(name, nodes) = node else {
            continue;
        };
        if *name == "--spacing" {
            if !(storage[0].used_spacing_number && storage[1].used_spacing_number) {
                track_spacing(nodes, storage);
            }
            normalize_value(nodes, storage, alloc);
            continue;
        }
        let Some(index) = storage_index(name) else {
            normalize_value(nodes, storage, alloc);
            continue;
        };
        let raw = vp::to_string(nodes);
        let args: std::vec::Vec<String> = segment(&raw, b',').into_iter().map(normalize_arg).collect();
        *nodes = vp::parse(&args.join(","), alloc);
        for arg in nodes.iter() {
            let ValueNode::Word(word) = *arg else {
                continue;
            };
            let bytes = word.as_bytes();
            if matches!(bytes.first(), Some(b'"' | b'\'')) && bytes.first() == bytes.last() {
                storage[index].literals.insert(&word[1..word.len().max(2) - 1]);
            } else if word.starts_with("--") {
                let key = word.find("-*").map_or(word, |i| &word[..i]);
                storage[index].theme_keys.insert(key);
            }
        }
        normalize_value(nodes, storage, alloc);
    }
}

fn track_spacing(nodes: &[ValueNode<'_>], storage: &mut [Storage<'_>; 2]) -> bool {
    for node in nodes {
        let ValueNode::Function(name, children) = node else {
            continue;
        };
        if let Some(index) = storage_index(name) {
            for arg in children {
                match *arg {
                    ValueNode::Word("integer") => storage[index].used_spacing_integer = true,
                    ValueNode::Word("number") => {
                        storage[index].used_spacing_number = true;
                        if storage[0].used_spacing_number && storage[1].used_spacing_number {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }
        if track_spacing(children, storage) {
            return true;
        }
    }
    false
}

fn normalize_arg(arg: &str) -> String {
    let arg = arg.replace("\\*", "*");
    let arg = join_theme_keys(&arg);
    let mut out: String = arg.chars().filter(|c| !is_js_whitespace(*c)).collect();
    while let Some(i) = out.find("-*-*") {
        let mut end = i + 4;
        while out[end..].starts_with("-*") {
            end += 2;
        }
        out.replace_range(i..end, "-*");
    }
    if out.starts_with("--") && !out.contains('(') && !out.contains("-*") {
        out.push_str("-*");
    }
    out
}

fn join_theme_keys(arg: &str) -> String {
    let bytes = arg.as_bytes();
    let mut out = String::with_capacity(arg.len() + 4);
    let (mut copied, mut p) = (0, 0);
    while p + 1 < bytes.len() {
        if !arg[p..].starts_with("--") {
            p += 1;
            continue;
        }
        let mut q = p + 2;
        let mut found = None;
        while q < bytes.len() {
            let ch = arg[q..].chars().next().unwrap_or(' ');
            if is_js_whitespace(ch) && arg[q + ch.len_utf8()..].starts_with("--") {
                found = Some((q, ch.len_utf8()));
                break;
            }
            if matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                break;
            }
            q += ch.len_utf8();
        }
        match found {
            Some((q, len)) => {
                out.push_str(&arg[copied..q]);
                out.push_str("-*--");
                p = q + len + 2;
                copied = p;
            }
            None => p += 1,
        }
    }
    out.push_str(&arg[copied..]);
    out
}

#[inline]
fn is_js_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

#[derive(Default)]
struct Resolution {
    used_value: bool,
    resolved_value: bool,
    used_modifier: bool,
    resolved_modifier: bool,
    resolved_ratio: bool,
}

enum Input<'a> {
    Value(Option<UtilityValue<'a>>),
    Modifier(Option<Modifier<'a>>),
}

pub(super) fn compile<'a>(cx: &Cx<'a, '_>, candidate: &Candidate<'a>, template: &CssTemplate<'a>) -> Option<Nodes<'a>> {
    let CandidateKind::Functional { value, modifier, .. } = candidate.kind else {
        return None;
    };
    let mut nodes = clone_in(&template.body, cx.alloc);
    let mut state = Resolution::default();
    let mut non_ratio = std::vec::Vec::new();
    let mut decls = template.values.iter();
    substitute(cx, &mut nodes, &mut decls, value, modifier, &mut state, &mut non_ratio);

    if !state.used_value || !state.resolved_value {
        return None;
    }
    if state.used_modifier && !state.resolved_modifier && modifier.is_some() {
        return None;
    }
    if state.resolved_ratio && state.resolved_modifier {
        return None;
    }
    if modifier.is_some() && !state.resolved_ratio && !state.resolved_modifier {
        return None;
    }
    if state.resolved_ratio {
        let mut index = 0;
        remove_tagged(&mut nodes, &non_ratio, &mut index);
    }
    Some(nodes)
}

fn substitute<'a>(
    cx: &Cx<'a, '_>,
    nodes: &mut Nodes<'a>,
    decls: &mut std::slice::Iter<'_, Option<&'a [ValueNode<'a>]>>,
    value: Option<UtilityValue<'a>>,
    modifier: Option<Modifier<'a>>,
    state: &mut Resolution,
    non_ratio: &mut std::vec::Vec<bool>,
) {
    nodes.retain_mut(|node| match node {
        Node::Decl { value: decl, .. } => {
            let Some(Some(parsed)) = decls.next() else {
                non_ratio.push(false);
                return true;
            };
            let mut ast = Vec::from_iter_in(parsed.iter().map(|n| n.clone_in(cx.alloc)), cx.alloc);
            let mut resolved_non_ratio = false;
            let keep = resolve_functions(cx, &mut ast, value, modifier, state, &mut resolved_non_ratio);
            if keep {
                *decl = Some(vp::to_css(&ast, cx.alloc));
                non_ratio.push(resolved_non_ratio);
            }
            keep
        }
        _ => {
            if let Some(children) = node.children_mut() {
                substitute(cx, children, decls, value, modifier, state, non_ratio);
            }
            true
        }
    });
}

fn remove_tagged(nodes: &mut Nodes<'_>, tags: &[bool], index: &mut usize) {
    nodes.retain_mut(|node| match node {
        Node::Decl { .. } => {
            *index += 1;
            !tags[*index - 1]
        }
        _ => {
            if let Some(children) = node.children_mut() {
                remove_tagged(children, tags, index);
            }
            true
        }
    });
}

fn resolve_functions<'a>(
    cx: &Cx<'a, '_>,
    ast: &mut Vec<'a, ValueNode<'a>>,
    value: Option<UtilityValue<'a>>,
    modifier: Option<Modifier<'a>>,
    state: &mut Resolution,
    resolved_non_ratio: &mut bool,
) -> bool {
    let mut i = 0;
    while i < ast.len() {
        let ValueNode::Function(name, args) = &mut ast[i] else {
            i += 1;
            continue;
        };
        let input = match *name {
            "--value" => {
                state.used_value = true;
                Input::Value(value)
            }
            "--modifier" => {
                state.used_modifier = true;
                Input::Modifier(modifier)
            }
            _ => {
                if !resolve_functions(cx, args, value, modifier, state, resolved_non_ratio) {
                    return false;
                }
                i += 1;
                continue;
            }
        };
        let Some((nodes, ratio)) = resolve_value_function(cx, &input, args) else {
            return false;
        };
        match input {
            Input::Value(_) => {
                state.resolved_value = true;
                if ratio {
                    state.resolved_ratio = true;
                } else {
                    *resolved_non_ratio = true;
                }
            }
            Input::Modifier(_) => state.resolved_modifier = true,
        }
        let len = nodes.len();
        ast.splice(i..=i, nodes);
        i += len;
    }
    true
}

fn resolve_value_function<'a>(cx: &Cx<'a, '_>, input: &Input<'a>, args: &[ValueNode<'a>]) -> Option<(Vec<'a, ValueNode<'a>>, bool)> {
    let parse = |v: &str| vp::parse(v, cx.alloc);
    let (named, arbitrary, fraction, data_type) = match *input {
        Input::Value(None) | Input::Modifier(None) => {
            return args.iter().find_map(|arg| match arg {
                ValueNode::Function("--default", nodes) => {
                    Some((Vec::from_iter_in(nodes.iter().map(|n| n.clone_in(cx.alloc)), cx.alloc), false))
                }
                _ => None,
            });
        }
        Input::Value(Some(UtilityValue::Named { value, fraction })) => (Some(value), None, fraction, None),
        Input::Value(Some(UtilityValue::Arbitrary { value, data_type })) => (None, Some(value), None, data_type),
        Input::Modifier(Some(Modifier::Named(value))) => (Some(value), None, Some(value), None),
        Input::Modifier(Some(Modifier::Arbitrary(value))) => (None, Some(value), None, None),
    };

    for arg in args {
        let ValueNode::Word(word) = *arg else {
            continue;
        };
        let bytes = word.as_bytes();
        if let Some(named) = named {
            if matches!(bytes.first(), Some(b'"' | b'\'')) && bytes.first() == bytes.last() {
                if word.len() >= 2 && &word[1..word.len() - 1] == named {
                    return Some((parse(named), false));
                }
                continue;
            }
            if let Some(theme_key) = word.strip_suffix("-*").filter(|_| word.starts_with("--")) {
                if let Some(resolved) = cx.resolve(Some(named), &[theme_key]).filter(|v| !v.is_empty()) {
                    return Some((parse(resolved), false));
                }
                continue;
            }
            if word.starts_with("--") {
                let mut keys: std::vec::Vec<&str> = word.split("-*").collect();
                if keys.len() <= 1 {
                    continue;
                }
                let root = keys.remove(0);
                let last = keys[keys.len() - 1];
                if let Some((_, options)) = cx.theme.resolve_with(named, &[root], &keys, cx.options)
                    && let Some(&(_, resolved)) = options.iter().find(|(k, _)| *k == last)
                    && !resolved.is_empty()
                {
                    return Some((parse(resolved), false));
                }
                continue;
            }
            if !BARE_VALUE_DATA_TYPES.contains(&word) {
                continue;
            }
            let Some(resolved) = (if word == "ratio" { fraction } else { Some(named) }).filter(|v| !v.is_empty()) else {
                continue;
            };
            let Some(kind) = DataType::from_name(word).and_then(|t| infer_data_type(resolved, &[t])) else {
                continue;
            };
            match kind {
                DataType::Ratio => {
                    let parts = segment(resolved, b'/');
                    let integer = |v: &str| {
                        let n = js_number(v);
                        n.fract() == 0.0 && n >= 0.0
                    };
                    if !integer(parts[0]) || !integer(parts.get(1).copied().unwrap_or("")) {
                        continue;
                    }
                    let lhs = parts[0].trim();
                    let rhs = parts.get(1).map_or("", |v| v.trim());
                    return Some((parse(cx.cat(&[lhs, " / ", rhs])), true));
                }
                DataType::Number if !is_multiple_of_quarter(resolved) => continue,
                DataType::Percentage if !is_positive_integer(&resolved[..resolved.len() - 1]) => continue,
                _ => return Some((parse(resolved), false)),
            }
        } else if let Some(value) = arbitrary
            && bytes.first() == Some(&b'[')
            && bytes.last() == Some(&b']')
        {
            let expected = &word[1..word.len() - 1];
            if expected == "*" {
                return Some((parse(value), false));
            }
            match data_type {
                Some(hint) if !hint.is_empty() && hint != expected => continue,
                Some(hint) if !hint.is_empty() => return Some((parse(value), false)),
                _ => {}
            }
            if DataType::from_name(expected).and_then(|t| infer_data_type(value, &[t])).is_some() {
                return Some((parse(value), false));
            }
        }
    }
    None
}

pub fn is_valid_static_utility_name(name: &str) -> bool {
    let Some(root_len) = utility_root_len(name) else {
        return false;
    };
    let (root, value) = name.split_at(root_len);
    if value.is_empty() {
        return !root.ends_with('-');
    }
    let bytes = value.as_bytes();
    let prev = |i: usize| if i > 0 { bytes[i - 1] } else { *root.as_bytes().last().unwrap_or(&0) };
    let mut seen_slash = false;
    for (i, &ch) in bytes.iter().enumerate() {
        match ch {
            b'%' => {
                if i != bytes.len() - 1 || !prev(i).is_ascii_digit() {
                    return false;
                }
            }
            b'/' => {
                if i == bytes.len() - 1 || seen_slash {
                    return false;
                }
                seen_slash = true;
            }
            b'.' => {
                if !prev(i).is_ascii_digit() || !bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
                    return false;
                }
            }
            b'_' | b'-' => {}
            _ if ch.is_ascii_alphanumeric() => {}
            _ => return false,
        }
    }
    true
}

pub fn is_valid_functional_utility_name(name: &str) -> bool {
    let Some(name) = name.strip_suffix("-*") else {
        return false;
    };
    utility_root_len(name) == Some(name.len())
}

fn utility_root_len(name: &str) -> Option<usize> {
    let bytes = name.as_bytes();
    let start = usize::from(bytes.first() == Some(&b'-'));
    if !bytes.get(start).is_some_and(u8::is_ascii_lowercase) {
        return None;
    }
    let rest = bytes[start + 1..].iter().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')).count();
    Some(start + 1 + rest)
}
