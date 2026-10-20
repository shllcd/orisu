// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::math::add_whitespace_around_math_operators;
use crate::value::{self, ValueNode};
use orisu_allocator::{Allocator, Vec};

pub fn decode_arbitrary_value<'a>(input: &'a str, alloc: &'a Allocator) -> &'a str {
    if !input.contains('(') {
        return convert_underscores(input, false, alloc);
    }
    let mut ast = value::parse_in(input, alloc);
    decode_nodes(&mut ast, alloc);
    add_whitespace_around_math_operators(value::to_css(&ast, alloc), alloc)
}

pub fn convert_underscores<'a>(input: &'a str, skip_underscore_to_space: bool, alloc: &'a Allocator) -> &'a str {
    let bytes = input.as_bytes();
    let Some(first) = memchr::memchr(b'_', bytes) else {
        return input;
    };
    if skip_underscore_to_space && !input.contains("\\_") {
        return input;
    }
    let mut out = Vec::with_capacity_in(bytes.len(), alloc);
    out.extend_from_slice(&bytes[..first.saturating_sub(1)]);
    let mut i = first.saturating_sub(1);
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\\' && bytes.get(i + 1) == Some(&b'_') {
            out.push(b'_');
            i += 2;
            continue;
        }
        out.push(if ch == b'_' && !skip_underscore_to_space { b' ' } else { ch });
        i += 1;
    }
    std::str::from_utf8(out.into_bump_slice()).unwrap_or(input)
}

fn decode_nodes<'a>(ast: &mut [ValueNode<'a>], alloc: &'a Allocator) {
    for node in ast {
        match node {
            ValueNode::Function(name, nodes) => {
                if *name == "url" || name.ends_with("_url") {
                    *name = convert_underscores(name, false, alloc);
                    continue;
                }
                if *name == "var" || name.ends_with("_var") || *name == "theme" || name.ends_with("_theme") {
                    *name = convert_underscores(name, false, alloc);
                    for (i, child) in nodes.iter_mut().enumerate() {
                        if i == 0
                            && let ValueNode::Word(w) = child
                        {
                            *w = convert_underscores(w, true, alloc);
                            continue;
                        }
                        decode_nodes(std::slice::from_mut(child), alloc);
                    }
                    continue;
                }
                *name = convert_underscores(name, false, alloc);
                decode_nodes(nodes, alloc);
            }
            ValueNode::Word(v) | ValueNode::Separator(v) => *v = convert_underscores(v, false, alloc),
        }
    }
}
