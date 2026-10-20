// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::value::Sink;
use orisu_allocator::{Allocator, ArenaString, Vec};
use orisu_ast::bytes::ByteTable;

#[derive(Debug, PartialEq, Eq)]
pub enum SNode<'a> {
    List(Vec<'a, SNode<'a>>),
    Combinator(&'a str),
    Complex(Vec<'a, SNode<'a>>),
    Compound(Vec<'a, SNode<'a>>),
    Selector(&'a str),
    Function(&'a str, Vec<'a, SNode<'a>>),
    Value(&'a str),
}

impl<'a> SNode<'a> {
    pub fn clone_in(&self, alloc: &'a Allocator) -> Self {
        let copy = |nodes: &Vec<'a, SNode<'a>>| Vec::from_iter_in(nodes.iter().map(|n| n.clone_in(alloc)), alloc);
        match self {
            Self::List(n) => Self::List(copy(n)),
            Self::Combinator(v) => Self::Combinator(v),
            Self::Complex(n) => Self::Complex(copy(n)),
            Self::Compound(n) => Self::Compound(copy(n)),
            Self::Selector(v) => Self::Selector(v),
            Self::Function(name, n) => Self::Function(name, copy(n)),
            Self::Value(v) => Self::Value(v),
        }
    }

    fn first_byte(&self) -> Option<u8> {
        match self {
            Self::Selector(v) => v.as_bytes().first().copied(),
            _ => None,
        }
    }

    pub fn is_universal(&self) -> bool {
        self.first_byte() == Some(b'*')
    }

    pub fn is_type(&self) -> bool {
        match self {
            Self::Selector(v) => !matches!(v.as_bytes().first(), Some(b'*' | b'&' | b'.' | b'#' | b':' | b'[')),
            _ => false,
        }
    }

    pub fn is_pseudo(&self) -> bool {
        self.first_byte() == Some(b':')
    }

    pub fn is_class(&self) -> bool {
        self.first_byte() == Some(b'.')
    }

    pub fn children(&self) -> Option<&Vec<'a, SNode<'a>>> {
        match self {
            Self::List(n) | Self::Complex(n) | Self::Compound(n) | Self::Function(_, n) => Some(n),
            _ => None,
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<'a, SNode<'a>>> {
        match self {
            Self::List(n) | Self::Complex(n) | Self::Compound(n) | Self::Function(_, n) => Some(n),
            _ => None,
        }
    }
}

pub fn to_css<'a>(ast: &[SNode<'_>], alloc: &'a Allocator) -> &'a str {
    let mut out = ArenaString::new_in(alloc);
    write_css(ast, &mut out, false);
    out.into_bump_str()
}

pub fn to_string(ast: &[SNode<'_>], minify: bool) -> String {
    let mut out = String::new();
    write_css(ast, &mut out, minify);
    out
}

pub fn write_css(ast: &[SNode<'_>], out: &mut impl Sink, minify: bool) {
    for node in ast {
        match node {
            SNode::Selector(v) | SNode::Value(v) => out.put(v),
            SNode::Combinator(v) => {
                if minify || *v == " " {
                    out.put(v);
                } else {
                    out.put(" ");
                    out.put(v);
                    out.put(" ");
                }
            }
            SNode::Function(name, nodes) => {
                out.put(name);
                out.put("(");
                write_css(nodes, out, minify);
                out.put(")");
            }
            SNode::Complex(nodes) | SNode::Compound(nodes) => write_css(nodes, out, minify),
            SNode::List(nodes) => {
                for (i, n) in nodes.iter().enumerate() {
                    if i > 0 {
                        out.put(if minify { "," } else { ", " });
                    }
                    write_css(std::slice::from_ref(n), out, minify);
                }
            }
        }
    }
}

struct Frame<'a> {
    name: &'a str,
    target: Vec<'a, SNode<'a>>,
    list: Option<Vec<'a, SNode<'a>>>,
    contains_combinator: bool,
}

impl<'a> Frame<'a> {
    fn new(name: &'a str, alloc: &'a Allocator) -> Self {
        Self { name, target: Vec::new_in(alloc), list: None, contains_combinator: false }
    }

    fn current(&mut self, alloc: &'a Allocator) -> SNode<'a> {
        let mut nodes = std::mem::replace(&mut self.target, Vec::new_in(alloc));
        if nodes.len() == 1 {
            nodes.pop().unwrap_or(SNode::Compound(Vec::new_in(alloc)))
        } else if self.contains_combinator {
            SNode::Complex(nodes)
        } else {
            SNode::Compound(nodes)
        }
    }

    fn append(&mut self, node: SNode<'a>, alloc: &'a Allocator) {
        match self.target.last_mut() {
            Some(SNode::Compound(nodes)) => nodes.push(node),
            Some(existing @ (SNode::Selector(_) | SNode::Function(..) | SNode::Value(_) | SNode::Complex(_))) => {
                let mut pair = Vec::with_capacity_in(2, alloc);
                pair.push(std::mem::replace(existing, SNode::Value("")));
                pair.push(node);
                *existing = SNode::Compound(pair);
            }
            _ => self.target.push(node),
        }
    }

    fn finish(mut self, alloc: &'a Allocator) -> Vec<'a, SNode<'a>> {
        if let Some(mut list) = self.list.take() {
            list.push(self.current(alloc));
            let mut out = Vec::with_capacity_in(1, alloc);
            out.push(SNode::List(list));
            out
        } else if self.contains_combinator {
            let mut out = Vec::with_capacity_in(1, alloc);
            out.push(SNode::Complex(self.target));
            out
        } else {
            self.target
        }
    }
}

static COMBINATOR: ByteTable = ByteTable::new(b">\n +\t~");
static SPECIAL: ByteTable = ByteTable::new(b",>\n +\t~()[.:#'\"&*\\");

pub fn parse<'a>(input: &str, alloc: &'a Allocator) -> Vec<'a, SNode<'a>> {
    let owned;
    let input = if input.contains("\r\n") {
        owned = input.replace("\r\n", "\n");
        owned.as_str()
    } else {
        input
    };
    parse_in(alloc.alloc_str(input), alloc)
}

pub fn parse_in<'a>(input: &'a str, alloc: &'a Allocator) -> Vec<'a, SNode<'a>> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut stack: std::vec::Vec<Frame<'a>> = std::vec::Vec::new();
    let mut frame = Frame::new("", alloc);
    let mut buf: Option<usize> = None;
    let mut i = 0;

    macro_rules! flush {
        () => {
            if let Some(start) = buf.take()
                && start < i
            {
                frame.append(SNode::Selector(&input[start..i]), alloc);
            }
        };
    }
    macro_rules! close_frame {
        ($parent:expr) => {{
            let child = std::mem::replace(&mut frame, $parent);
            let name = child.name;
            let nodes = child.finish(alloc);
            frame.append(SNode::Function(name, nodes), alloc);
        }};
    }

    while i < len {
        let next = SPECIAL.find(bytes, i).unwrap_or(len);
        if next > i {
            buf.get_or_insert(i);
            i = next;
            if i >= len {
                break;
            }
        }
        let ch = bytes[i];
        match ch {
            b',' => {
                flush!();
                while i + 1 < len && matches!(bytes[i + 1], b'\n' | b' ' | b'\t') {
                    i += 1;
                }
                let item = frame.current(alloc);
                match &mut frame.list {
                    Some(list) => list.push(item),
                    None => {
                        let mut list = Vec::new_in(alloc);
                        list.push(item);
                        frame.list = Some(list);
                    }
                }
                frame.contains_combinator = false;
            }
            b'>' | b'\n' | b' ' | b'+' | b'\t' | b'~' => {
                flush!();
                let start = i;
                let end = COMBINATOR.skip(bytes, i + 1);
                i = end - 1;
                let value = input[start..end].trim();
                if value.is_empty() && (frame.target.is_empty() || end >= len || bytes[end] == b',') {
                    i += 1;
                    continue;
                }
                frame.target.push(SNode::Combinator(if value.is_empty() { " " } else { value }));
                frame.contains_combinator = true;
            }
            b'(' => {
                let name = buf.take().map_or("", |s| &input[s..i]);
                if !matches!(name, ":not" | ":where" | ":has" | ":is") {
                    let start = i + 1;
                    let mut nesting = 0;
                    let mut j = i + 1;
                    while j < len {
                        match bytes[j] {
                            b'(' => nesting += 1,
                            b')' if nesting == 0 => {
                                i = j;
                                break;
                            }
                            b')' => nesting -= 1,
                            _ => {}
                        }
                        j += 1;
                    }
                    let contents = input.get(start..i).unwrap_or("");
                    let mut nodes = Vec::new_in(alloc);
                    if (name == ":nth-child" || name == ":nth-last-child")
                        && let Some(idx) = contents.find("of ")
                    {
                        nodes.push(SNode::Value(&contents[..idx + 3]));
                        nodes.extend(parse_in(&contents[idx + 3..], alloc));
                    } else {
                        nodes.push(SNode::Value(contents));
                    }
                    frame.append(SNode::Function(name, nodes), alloc);
                } else {
                    let parent = std::mem::replace(&mut frame, Frame::new(name, alloc));
                    stack.push(parent);
                }
            }
            b')' => {
                flush!();
                match stack.pop() {
                    Some(parent) => close_frame!(parent),
                    None => {
                        let done = std::mem::replace(&mut frame, Frame::new("", alloc)).finish(alloc);
                        frame.target = done;
                    }
                }
            }
            b'.' | b':' | b'#' => {
                let extends = ch == b':' && buf.is_some_and(|s| &input[s..i] == ":");
                if !extends {
                    flush!();
                    buf = Some(i);
                }
            }
            b'[' => {
                flush!();
                let start = i;
                let mut nesting = 0;
                let mut j = i + 1;
                while j < len {
                    match bytes[j] {
                        b'[' => nesting += 1,
                        b']' if nesting == 0 => {
                            i = j;
                            break;
                        }
                        b']' => nesting -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                frame.append(SNode::Selector(&input[start..=i]), alloc);
            }
            b'\'' | b'"' => {
                buf.get_or_insert(i);
                let mut j = i + 1;
                while j < len {
                    if bytes[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if bytes[j] == ch {
                        i = j;
                        break;
                    }
                    j += 1;
                }
            }
            b'&' | b'*' => {
                flush!();
                frame.append(SNode::Selector(&input[i..=i]), alloc);
            }
            _ => {
                buf.get_or_insert(i);
                i += input[i + 1..].chars().next().map_or(0, char::len_utf8);
            }
        }
        i += 1;
    }
    let i = len;
    if let Some(start) = buf.take()
        && start < i
    {
        frame.append(SNode::Selector(&input[start..i]), alloc);
    }
    while let Some(parent) = stack.pop() {
        close_frame!(parent);
    }
    frame.finish(alloc)
}

pub fn walk_mut<'a>(ast: &mut [SNode<'a>], f: &mut impl FnMut(&mut SNode<'a>)) {
    for node in ast {
        f(node);
        if let Some(children) = node.children_mut() {
            walk_mut(children, f);
        }
    }
}

pub fn walk<'a>(ast: &[SNode<'a>], f: &mut impl FnMut(&SNode<'a>)) {
    for node in ast {
        f(node);
        if let Some(children) = node.children() {
            walk(children, f);
        }
    }
}
