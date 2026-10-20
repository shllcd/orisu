// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use orisu_allocator::{AllocExt, Allocator, ArenaString, Vec};
use orisu_ast::bytes::ByteTable;

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

pub trait Sink {
    fn put(&mut self, s: &str);
}

impl Sink for String {
    #[inline]
    fn put(&mut self, s: &str) {
        self.push_str(s);
    }
}

impl Sink for ArenaString<'_> {
    #[inline]
    fn put(&mut self, s: &str) {
        self.push_str(s);
    }
}

pub fn write_css(ast: &[ValueNode<'_>], out: &mut impl Sink) {
    for node in ast {
        match node {
            ValueNode::Word(v) | ValueNode::Separator(v) => out.put(v),
            ValueNode::Function(name, nodes) => {
                out.put(name);
                out.put("(");
                write_css(nodes, out);
                out.put(")");
            }
        }
    }
}

static SEPARATOR: ByteTable = ByteTable::new(b":,=><\n \t");
static SPECIAL: ByteTable = ByteTable::new(b"\\/:,=><\n \t'\"()");

pub fn parse<'a>(input: &str, alloc: &'a Allocator) -> Vec<'a, ValueNode<'a>> {
    let input: &'a str = if input.contains("\r\n") { alloc.str(&input.replace("\r\n", "\n")) } else { alloc.str(input) };
    parse_in(input, alloc)
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
            b'\'' | b'"' => {
                let quote = bytes[i];
                let mut j = i + 1;
                let mut close = None;
                while j < len {
                    match bytes[j] {
                        b'\\' => j += 2,
                        b if b == quote => {
                            close = Some(j);
                            break;
                        }
                        _ => j += 1,
                    }
                }
                i = close.map_or(i + 1, |c| c + 1);
            }
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
