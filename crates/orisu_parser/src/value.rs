// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use orisu_allocator::{Allocator, ArenaString, Vec};
use orisu_ast::bytes::{ByteTable, normalize_newlines, string_end};
use smallvec::SmallVec;
use std::fmt::Write;

#[derive(Debug, PartialEq, Eq)]
pub enum ValueNode<'a> {
    Word(&'a str),
    Function(&'a str, Vec<'a, ValueNode<'a>>),
    Separator(&'a str),
}

impl<'a> ValueNode<'a> {
    pub fn value(&self) -> &'a str {
        match self {
            Self::Word(v) | Self::Function(v, _) | Self::Separator(v) => v,
        }
    }

    pub fn clone_in(&self, alloc: &'a Allocator) -> Self {
        match self {
            Self::Word(v) => Self::Word(v),
            Self::Function(name, nodes) => Self::Function(name, Vec::from_iter_in(nodes.iter().map(|n| n.clone_in(alloc)), alloc)),
            Self::Separator(v) => Self::Separator(v),
        }
    }
}

pub fn to_css<'a>(ast: &[ValueNode<'_>], alloc: &'a Allocator) -> &'a str {
    let mut out = ArenaString::new_in(alloc);
    write_css(ast, &mut out);
    out.into_bump_str()
}

pub fn to_string(ast: &[ValueNode<'_>]) -> String {
    let mut out = String::new();
    write_css(ast, &mut out);
    out
}

pub fn write_css(ast: &[ValueNode<'_>], out: &mut impl Write) {
    for node in ast {
        match node {
            ValueNode::Word(v) | ValueNode::Separator(v) => put(out, v),
            ValueNode::Function(name, nodes) => {
                put(out, name);
                put(out, "(");
                write_css(nodes, out);
                put(out, ")");
            }
        }
    }
}

/// Writes to a `String` or `ArenaString`, which cannot fail.
#[inline]
pub(crate) fn put(out: &mut impl Write, s: &str) {
    let _ = out.write_str(s);
}

static SEPARATOR: ByteTable = ByteTable::new(b":,=><\n \t");
static SPECIAL: ByteTable = ByteTable::new(b"\\/:,=><\n \t'\"()");

pub fn parse<'a>(input: &str, alloc: &'a Allocator) -> Vec<'a, ValueNode<'a>> {
    parse_in(normalize_newlines(input, alloc), alloc)
}

pub fn parse_in<'a>(input: &'a str, alloc: &'a Allocator) -> Vec<'a, ValueNode<'a>> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut ast = Vec::new_in(alloc);
    let mut stack: std::vec::Vec<(&'a str, Vec<'a, ValueNode<'a>>)> = std::vec::Vec::new();
    let mut start = 0;
    let mut i = 0;

    macro_rules! push {
        ($node:expr) => {{
            let node = $node;
            match stack.last_mut() {
                Some((_, nodes)) => nodes.push(node),
                None => ast.push(node),
            }
        }};
    }
    macro_rules! flush {
        () => {
            if start < i {
                push!(ValueNode::Word(&input[start..i]));
            }
        };
    }

    while let Some(at) = SPECIAL.find(bytes, i) {
        i = at;
        match bytes[i] {
            b'\\' => {
                i += 1 + input[i + 1..].chars().next().map_or(0, char::len_utf8);
                continue;
            }
            b'/' => {
                flush!();
                push!(ValueNode::Word("/"));
                i += 1;
                start = i;
            }
            b'\'' | b'"' => i = string_end(bytes, i).map_or(i + 1, |c| c + 1),
            b'(' => {
                stack.push((&input[start..i], Vec::new_in(alloc)));
                i += 1;
                start = i;
            }
            b')' => {
                if let Some((name, mut nodes)) = stack.pop() {
                    if start < i {
                        nodes.push(ValueNode::Word(&input[start..i]));
                    }
                    push!(ValueNode::Function(name, nodes));
                }
                i += 1;
                start = i;
            }
            _ => {
                flush!();
                let end = SEPARATOR.skip(bytes, i + 1);
                push!(ValueNode::Separator(&input[i..end]));
                i = end;
                start = i;
            }
        }
    }
    let trailing = (start < len).then(|| &input[start..]);
    while let Some((name, nodes)) = stack.pop() {
        push!(ValueNode::Function(name, nodes));
    }
    if let Some(word) = trailing {
        ast.push(ValueNode::Word(word));
    }
    ast
}

pub fn extract_used_variables<'a>(raw: &'a str, alloc: &'a Allocator) -> SmallVec<[&'a str; 4]> {
    let mut out = SmallVec::new();
    if !raw.contains("var(") {
        return out;
    }
    let ast = parse_in(raw, alloc);
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
