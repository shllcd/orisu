// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::segment::segment;
use crate::value::{self, ValueNode};
use indexmap::{IndexMap, IndexSet};
use orisu_allocator::Allocator;
use smallvec::SmallVec;
use std::hash::Hash;

pub fn extract_used_variables<'a>(raw: &'a str, alloc: &'a Allocator) -> SmallVec<[&'a str; 4]> {
    let mut out = SmallVec::new();
    if !raw.contains("var(") {
        return out;
    }
    let ast = value::parse_in(raw, alloc);
    fn collect_words<'a>(nodes: &[ValueNode<'a>], out: &mut SmallVec<[&'a str; 4]>) {
        for node in nodes {
            match node {
                ValueNode::Word(w) if w.starts_with("--") => out.push(w),
                ValueNode::Function(_, children) => collect_words(children, out),
                _ => {}
            }
        }
    }
    fn find_vars<'a>(nodes: &[ValueNode<'a>], out: &mut SmallVec<[&'a str; 4]>) {
        for node in nodes {
            if let ValueNode::Function(name, children) = node {
                if *name == "var" {
                    collect_words(children, out);
                } else {
                    find_vars(children, out);
                }
            }
        }
    }
    find_vars(&ast, &mut out);
    out
}

pub fn topological_sort<K: Clone + Eq + Hash>(graph: &IndexMap<K, IndexSet<K>>, mut on_cycle: impl FnMut(&[K], &K)) -> Vec<K> {
    let mut seen = IndexSet::new();
    let mut wip = IndexSet::new();
    let mut sorted = Vec::new();
    let mut path = Vec::new();
    fn visit<K: Clone + Eq + Hash>(
        node: &K,
        graph: &IndexMap<K, IndexSet<K>>,
        seen: &mut IndexSet<K>,
        wip: &mut IndexSet<K>,
        sorted: &mut Vec<K>,
        path: &mut Vec<K>,
        on_cycle: &mut dyn FnMut(&[K], &K),
    ) {
        let Some(deps) = graph.get(node) else { return };
        if seen.contains(node) {
            return;
        }
        if wip.contains(node) {
            on_cycle(path, node);
            return;
        }
        wip.insert(node.clone());
        for dep in deps {
            path.push(node.clone());
            visit(dep, graph, seen, wip, sorted, path, on_cycle);
            path.pop();
        }
        seen.insert(node.clone());
        wip.swap_remove(node);
        sorted.push(node.clone());
    }
    for node in graph.keys() {
        visit(node, graph, &mut seen, &mut wip, &mut sorted, &mut path, &mut on_cycle);
    }
    sorted
}

fn numeric_range(s: &str) -> Option<(i64, i64, Option<i64>)> {
    let parts: Vec<&str> = s.split("..").collect();
    if parts.len() != 2 && parts.len() != 3 {
        return None;
    }
    let num = |p: &str| -> Option<i64> {
        let digits = p.strip_prefix('-').unwrap_or(p);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse().ok()
    };
    Some((
        num(parts[0])?,
        num(parts[1])?,
        match parts.get(2) {
            Some(p) => Some(num(p)?),
            None => None,
        },
    ))
}

pub fn expand_braces(pattern: &str) -> Result<Vec<String>, String> {
    let Some(index) = pattern.find('{') else {
        return Ok(vec![pattern.to_owned()]);
    };
    let pre = &pattern[..index];
    let rest = &pattern[index..];
    let mut depth = 0i32;
    let mut end = rest.rfind('}');
    for (i, b) in rest.bytes().enumerate() {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                end = Some(i);
                break;
            }
        }
    }
    let Some(end) = end else {
        return Err(format!("The pattern `{pattern}` is not balanced."));
    };
    let inside = rest.get(1..end).unwrap_or("");
    let post = &rest[end + 1..];
    let parts: Vec<String> = match numeric_range(inside) {
        Some((start, stop, step)) => {
            let increasing = start < stop;
            let mut step = step.unwrap_or(if start <= stop { 1 } else { -1 });
            if step == 0 {
                return Err("Step cannot be zero in sequence expansion.".to_owned());
            }
            if increasing && step < 0 || !increasing && step > 0 {
                step = -step;
            }
            let mut out = Vec::new();
            let mut i = start;
            while if increasing { i <= stop } else { i >= stop } {
                out.push(i.to_string());
                i += step;
            }
            out
        }
        None => segment(inside, b',').into_iter().map(str::to_owned).collect(),
    };
    let mut expanded = Vec::new();
    for part in parts {
        expanded.extend(expand_braces(&part)?);
    }
    let mut result = Vec::new();
    for tail in expand_braces(post)? {
        for part in &expanded {
            result.push(format!("{pre}{part}{tail}"));
        }
    }
    Ok(result)
}

pub fn to_key_path(path: &str) -> Vec<String> {
    let mut keypath = Vec::new();
    for part in segment(path, b'.') {
        if !part.contains('[') {
            keypath.push(part.to_owned());
            continue;
        }
        let mut current = 0;
        while let Some(l) = part[current..].find('[').map(|i| i + current) {
            let Some(r) = part[l..].find(']').map(|i| i + l) else {
                break;
            };
            if l > current {
                keypath.push(part[current..l].to_owned());
            }
            keypath.push(part[l + 1..r].to_owned());
            current = r + 1;
        }
        if current < part.len() {
            keypath.push(part[current..].to_owned());
        }
    }
    keypath
}
