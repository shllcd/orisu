// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::design_system::Session;
use memchr::memmem;
use orisu_allocator::{AllocExt, Vec};
use orisu_ast::Node;
use orisu_ast::bytes::trim;
use orisu_parser::compare::dimension;
use orisu_parser::segment::segment;
use orisu_parser::value::{self as vp, ValueNode};

#[inline]
pub fn has_theme_function(value: &str) -> bool {
    memmem::find(value.as_bytes(), b"theme(").is_some()
        || memmem::find(value.as_bytes(), b"--alpha(").is_some()
        || memmem::find(value.as_bytes(), b"--spacing(").is_some()
}

/// Replaces `--alpha()`, `--spacing()`, `--theme()` and `theme()` calls; returns whether any were present.
pub fn substitute_functions<'a>(ast: &mut [Node<'a>], ds: &Session<'_, 'a>) -> Result<bool, String> {
    let mut found = false;
    for node in ast {
        match node {
            Node::Decl { value: Some(value), .. } if has_theme_function(value) => {
                found = true;
                *value = substitute_functions_in_value(value, false, ds)?;
            }
            Node::AtRule { name, params, nodes } => {
                if matches!(*name, "@media" | "@custom-media" | "@container" | "@supports") && has_theme_function(params) {
                    found = true;
                    *params = substitute_functions_in_value(params, true, ds)?;
                }
                found |= substitute_functions(nodes, ds)?;
            }
            Node::Rule { nodes, .. } | Node::Context { nodes, .. } | Node::AtRoot(nodes) => {
                found |= substitute_functions(nodes, ds)?;
            }
            _ => {}
        }
    }
    Ok(found)
}

pub fn substitute_functions_in_value<'a>(value: &str, in_at_rule: bool, ds: &Session<'_, 'a>) -> Result<&'a str, String> {
    let ast = vp::parse(value, ds.alloc);
    let mut out = Vec::with_capacity_in(ast.len(), ds.alloc);
    for node in ast {
        substitute_node(node, &mut out, in_at_rule, ds)?;
    }
    Ok(vp::to_css(&out, ds.alloc))
}

fn substitute_node<'a>(
    node: ValueNode<'a>,
    out: &mut Vec<'a, ValueNode<'a>>,
    in_at_rule: bool,
    ds: &Session<'_, 'a>,
) -> Result<(), String> {
    let ValueNode::Function(name, nodes) = node else {
        out.push(node);
        return Ok(());
    };
    if !matches!(name, "--alpha" | "--spacing" | "--theme" | "theme") {
        let mut children = Vec::with_capacity_in(nodes.len(), ds.alloc);
        for child in nodes {
            substitute_node(child, &mut children, in_at_rule, ds)?;
        }
        out.push(ValueNode::Function(name, children));
        return Ok(());
    }
    let raw = vp::to_string(&nodes);
    let args: smallvec::SmallVec<[&str; 4]> = segment(trim(&raw), b',').into_iter().map(trim).collect();
    let (first, rest) = (args[0], &args[1..]);
    let result = match name {
        "--alpha" => alpha(ds, first, rest)?,
        "--spacing" => spacing(ds, first, rest)?,
        "--theme" => theme(ds, in_at_rule, first, rest)?,
        _ => legacy_theme(ds, first, rest)?,
    };
    for node in vp::parse(result, ds.alloc) {
        substitute_node(node, out, in_at_rule, ds)?;
    }
    Ok(())
}

fn alpha<'a>(ds: &Session<'_, 'a>, value: &str, rest: &[&str]) -> Result<&'a str, String> {
    let parts = segment(value, b'/');
    let color = parts.first().map_or("", |v| trim(v));
    let alpha = parts.get(1).map_or("", |v| trim(v));
    let example = || {
        format!(
            "`--alpha({} / {})`",
            if color.is_empty() { "var(--my-color)" } else { color },
            if alpha.is_empty() { "50%" } else { alpha }
        )
    };
    if color.is_empty() || alpha.is_empty() {
        return Err(format!("The --alpha(…) function requires a color and an alpha value, e.g.: {}", example()));
    }
    if !rest.is_empty() {
        return Err(format!("The --alpha(…) function only accepts one argument, e.g.: {}", example()));
    }
    let cx = ds.cx();
    Ok(cx.with_alpha(ds.alloc.str(color), ds.alloc.str(alpha)))
}

fn spacing<'a>(ds: &Session<'_, 'a>, value: &str, rest: &[&str]) -> Result<&'a str, String> {
    if value.is_empty() {
        return Err("The --spacing(…) function requires an argument, but received none.".into());
    }
    if !rest.is_empty() {
        return Err(format!("The --spacing(…) function only accepts a single argument, but received {}.", rest.len() + 1));
    }
    let Some(multiplier) = ds.theme.resolve(None, &["--spacing"], 0).filter(|v| !v.is_empty()) else {
        return Err("The --spacing(…) function requires that the `--spacing` theme variable exists, but it was not found.".into());
    };
    match dimension(value) {
        Some((0.0, _)) => Ok("0px"),
        Some((1.0, _)) => Ok(multiplier),
        _ => Ok(ds.alloc.concat(&["calc(", multiplier, " * ", value, ")"])),
    }
}

fn theme<'a>(ds: &Session<'_, 'a>, in_at_rule: bool, path: &str, fallback: &[&str]) -> Result<&'a str, String> {
    if !path.starts_with("--") {
        return Err("The --theme(…) function can only be used with CSS variables from your theme.".into());
    }
    let (path, inline) = match path.strip_suffix(" inline") {
        Some(path) => (path, true),
        None => (path, in_at_rule),
    };
    let Some(resolved) = ds.resolve_theme_value(path, inline).filter(|v| !v.is_empty()) else {
        if !fallback.is_empty() {
            return Ok(ds.alloc.str(&fallback.join(", ")));
        }
        return Err(format!(
            "Could not resolve value for theme function: `theme({path})`. Consider checking if the variable name is correct or provide a fallback value to silence this error."
        ));
    };
    if fallback.is_empty() {
        return Ok(resolved);
    }
    let joined = fallback.join(", ");
    if joined == "initial" {
        return Ok(resolved);
    }
    if resolved == "initial" {
        return Ok(ds.alloc.str(&joined));
    }
    if resolved.starts_with("var(") || resolved.starts_with("theme(") || resolved.starts_with("--theme(") {
        let mut ast = vp::parse(resolved, ds.alloc);
        inject_fallback(&mut ast, ds.alloc.str(&joined), ds);
        return Ok(vp::to_css(&ast, ds.alloc));
    }
    Ok(resolved)
}

fn inject_fallback<'a>(ast: &mut [ValueNode<'a>], fallback: &'a str, ds: &Session<'_, 'a>) {
    for node in ast {
        let ValueNode::Function(name, nodes) = node else {
            continue;
        };
        if matches!(*name, "var" | "theme" | "--theme") {
            if nodes.len() == 1 {
                nodes.push(ValueNode::Word(ds.alloc.concat(&[", ", fallback])));
            } else if let Some(last @ ValueNode::Word("initial")) = nodes.last_mut() {
                *last = ValueNode::Word(fallback);
            }
        }
        inject_fallback(nodes, fallback, ds);
    }
}

fn legacy_theme<'a>(ds: &Session<'_, 'a>, path: &str, fallback: &[&str]) -> Result<&'a str, String> {
    let path = unquote(path);
    match ds.resolve_theme_value(&path, true).filter(|v| !v.is_empty()) {
        Some(resolved) => Ok(resolved),
        None if !fallback.is_empty() => Ok(ds.alloc.str(&fallback.join(", "))),
        None => Err(format!(
            "Could not resolve value for theme function: `theme({path})`. Consider checking if the path is correct or provide a fallback value to silence this error."
        )),
    }
}

fn unquote(value: &str) -> std::borrow::Cow<'_, str> {
    let Some(quote) = value.chars().next().filter(|c| matches!(c, '\'' | '"')) else {
        return value.into();
    };
    let chars: std::vec::Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut i = 1;
    while i + 1 < chars.len() {
        let current = chars[i];
        let next = chars[i + 1];
        if current == '\\' && (next == quote || next == '\\') {
            out.push(next);
            i += 2;
        } else {
            out.push(current);
            i += 1;
        }
    }
    out.into()
}
