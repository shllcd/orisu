// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use super::Canon;
use orisu_allocator::{AllocExt, Vec};
use orisu_candidate::Modifier;
use orisu_parser::infer::{format_js_number, is_multiple_of_quarter, js_number};
use orisu_parser::misc::to_key_path;
use orisu_parser::segment::segment;
use orisu_parser::value::{self as vp, ValueNode};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Convert {
    All,
    MigrateModifier,
    ThemeOnly,
}

const SPECIAL_DEFAULT_KEYS: &[(&str, &str)] = &[
    ("borderWidth", "border-width"),
    ("outlineWidth", "outline-width"),
    ("ringColor", "ring-color"),
    ("ringWidth", "ring-width"),
    ("transitionDuration", "transition-duration"),
    ("transitionTimingFunction", "transition-timing-function"),
];

const OLD_TO_NEW_NAMESPACE: &[(&str, &str)] = &[
    ("animation", "animate"),
    ("aspectRatio", "aspect"),
    ("borderRadius", "radius"),
    ("boxShadow", "shadow"),
    ("colors", "color"),
    ("containers", "container"),
    ("fontFamily", "font"),
    ("fontSize", "text"),
    ("letterSpacing", "tracking"),
    ("lineHeight", "leading"),
    ("maxWidth", "container"),
    ("screens", "breakpoint"),
    ("transitionTimingFunction", "ease"),
];

fn lookup(table: &[(&str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn kebab(part: &str) -> String {
    let mut out = String::with_capacity(part.len() + 4);
    let bytes = part.as_bytes();
    for (i, ch) in part.char_indices() {
        if ch.is_ascii_uppercase() && i > 0 && bytes[i - 1].is_ascii_lowercase() {
            out.push('-');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `["fontSize", "xs", "1", "lineHeight"]` → `text-xs--line-height`, or `None` when the path has no CSS variable.
pub(super) fn key_path_to_css_property(path: &[String]) -> Option<String> {
    let first = path.first()?;
    if let Some(special) = lookup(SPECIAL_DEFAULT_KEYS, first)
        && path.get(1).is_some_and(|p| p == "DEFAULT")
    {
        return Some(format!("default-{special}"));
    }
    if first == "container" {
        return None;
    }
    let valid = |part: &String| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_%/.".contains(&b));
    if !path.iter().all(valid) {
        return None;
    }
    let last = path.len() - 1;
    let parts: std::vec::Vec<String> = path
        .iter()
        .enumerate()
        .map(|(i, part)| {
            let part = if i == 0 { lookup(OLD_TO_NEW_NAMESPACE, part).unwrap_or(part) } else { part };
            let part = if part == "1" && i != last { "" } else { part };
            let part = part.replace('.', "_");
            if i == 0 || part.starts_with('-') || part == "lineHeight" { kebab(&part) } else { part }
        })
        .enumerate()
        .filter(|(i, part)| part != "DEFAULT" || *i != last)
        .map(|(_, part)| part)
        .collect();
    Some(parts.join("-"))
}

fn eventually_unquote(value: &str) -> String {
    let Some(quote) = value.chars().next().filter(|c| *c == '\'' || *c == '"') else {
        return value.to_owned();
    };
    let chars: std::vec::Vec<char> = value.chars().collect();
    let mut out = String::new();
    let mut i = 1;
    while i + 1 < chars.len() {
        let current = chars[i];
        let next = chars[i + 1];
        if current == '\\' && (next == quote || next == '\\') {
            out.push(next);
            i += 1;
        } else {
            out.push(current);
        }
        i += 1;
    }
    out
}

impl<'a> Canon<'a, '_> {
    pub(super) fn convert(&self, input: &str, mode: Convert) -> (String, Option<Modifier<'a>>) {
        let alloc = self.ds.alloc;
        let mut ast = vp::parse(input, alloc);
        if mode == Convert::ThemeOnly {
            return (self.substitute(&mut ast, &mut |path, fallback| self.to_theme(path, fallback)), None);
        }

        let (mut usages, mut modifiers) = (0, 0);
        count_theme_usages(&ast, &mut usages, &mut modifiers);
        if usages == 0 {
            return (input.to_owned(), None);
        }
        if modifiers == 0 {
            return (self.substitute(&mut ast, &mut |path, fallback| self.to_var(path, fallback)), None);
        }
        if modifiers > 1 {
            return (self.substitute(&mut ast, &mut |path, fallback| self.to_theme(path, fallback)), None);
        }

        let single = ast.len() == 1;
        let mut modifier = None;
        let result = self.substitute(&mut ast, &mut |path, fallback| {
            let parts: std::vec::Vec<&str> = segment(path, b'/').into_iter().map(str::trim).collect();
            if parts.len() > 2 {
                return None;
            }
            let mut path = path;
            if single && parts.len() == 2 && mode == Convert::MigrateModifier {
                let part = parts[1];
                let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
                modifier = Some(if part.strip_suffix('%').is_some_and(digits) {
                    Modifier::Named(alloc.str(&part[..part.len() - 1]))
                } else if part.strip_prefix('0').unwrap_or(part).strip_prefix('.').is_some_and(digits) {
                    let value = js_number(part) * 100.0;
                    let printed = alloc.str(&format_js_number(value));
                    if value.fract() == 0.0 { Modifier::Named(printed) } else { Modifier::Arbitrary(printed) }
                } else {
                    Modifier::Arbitrary(alloc.str(part))
                });
                path = parts[0];
            }
            self.to_var(path, fallback).or_else(|| self.to_theme(path, fallback))
        });
        (result, modifier)
    }

    fn path_to_variable_name(&self, path: &str, should_prefix: bool) -> Option<String> {
        let variable = format!("--{}", key_path_to_css_property(&to_key_path(path)).unwrap_or_else(|| "null".to_owned()));
        self.ds.theme.get(&[&variable]).filter(|v| !v.is_empty())?;
        match self.prefix() {
            Some(prefix) if should_prefix => Some(format!("--{prefix}-{}", &variable[2..])),
            _ => Some(variable),
        }
    }

    fn to_var(&self, path: &str, fallback: Option<&str>) -> Option<String> {
        if let Some(variable) = self.path_to_variable_name(path, true) {
            return Some(match fallback {
                Some(fallback) => format!("var({variable}, {fallback})"),
                None => format!("var({variable})"),
            });
        }
        let key_path = to_key_path(path);
        if key_path.first().is_some_and(|k| k == "spacing") && self.ds.theme.get(&["--spacing"]).is_some_and(|v| !v.is_empty()) {
            let multiplier = key_path.get(1)?;
            if !is_multiple_of_quarter(multiplier) {
                return None;
            }
            return Some(format!("--spacing({multiplier})"));
        }
        None
    }

    fn to_theme(&self, path: &str, fallback: Option<&str>) -> Option<String> {
        let mut parts = segment(path, b'/').into_iter().map(str::trim);
        let path = parts.next().unwrap_or("");
        let rest: std::vec::Vec<&str> = parts.collect();
        let variable = self.path_to_variable_name(path, false)?;
        let modifier = if rest.is_empty() { String::new() } else { format!("/{}", rest.join("/")) };
        Some(match fallback {
            Some(fallback) => format!("--theme({variable}{modifier}, {fallback})"),
            None => format!("--theme({variable}{modifier})"),
        })
    }

    fn substitute(&self, ast: &mut Vec<'a, ValueNode<'a>>, handle: &mut dyn FnMut(&str, Option<&str>) -> Option<String>) -> String {
        self.substitute_nodes(ast, handle);
        vp::to_string(ast)
    }

    fn substitute_nodes(&self, nodes: &mut Vec<'a, ValueNode<'a>>, handle: &mut dyn FnMut(&str, Option<&str>) -> Option<String>) {
        let mut i = 0;
        while i < nodes.len() {
            if let ValueNode::Function("theme", children) = &mut nodes[i]
                && !children.is_empty()
            {
                if matches!(children[0], ValueNode::Separator(s) if s.trim().is_empty()) {
                    children.remove(0);
                }
                if let Some(ValueNode::Word(first)) = children.first() {
                    let mut path = (*first).to_owned();
                    let mut skip_until = 1;
                    for (j, child) in children.iter().enumerate().skip(1) {
                        if child.value().contains(',') {
                            break;
                        }
                        path.push_str(&vp::to_string(std::slice::from_ref(child)));
                        skip_until = j + 1;
                    }
                    let path = eventually_unquote(&path);
                    let fallback = children.get(skip_until + 1..).filter(|f| !f.is_empty()).map(vp::to_string);
                    if let Some(mut replacement) = handle(&path, fallback.as_deref()) {
                        let previous = nodes[..i].iter().rev().find(|n| !matches!(n, ValueNode::Separator(s) if s.trim().is_empty()));
                        if previous.is_some_and(|p| matches!(p.value().trim(), "-" | "+" | "*" | "/")) {
                            replacement = format!("({replacement})");
                        }
                        let parsed = vp::parse(&replacement, self.ds.alloc);
                        nodes.splice(i..=i, parsed);
                        continue;
                    }
                }
            }
            if let ValueNode::Function(_, children) = &mut nodes[i] {
                self.substitute_nodes(children, handle);
            }
            i += 1;
        }
    }
}

fn count_theme_usages(nodes: &[ValueNode<'_>], usages: &mut usize, modifiers: &mut usize) {
    for node in nodes {
        let ValueNode::Function(name, children) = node else {
            continue;
        };
        if *name == "theme" {
            *usages += 1;
            for child in children.iter() {
                match child {
                    ValueNode::Separator(s) if s.contains(',') => break,
                    ValueNode::Word("/") => {
                        *modifiers += 1;
                        break;
                    }
                    _ => {}
                }
            }
        }
        count_theme_usages(children, usages, modifiers);
    }
}
