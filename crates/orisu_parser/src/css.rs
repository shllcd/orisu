// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use orisu_allocator::{Allocator, ArenaString, Vec};
use orisu_ast::bytes::{ByteTable, trim};
use orisu_ast::{Node, Nodes, parse_at_rule};
use smallvec::SmallVec;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssSyntaxError {
    pub message: String,
    pub offset: Option<usize>,
}

impl fmt::Display for CssSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CssSyntaxError {}

fn error(message: impl Into<String>, offset: Option<usize>) -> CssSyntaxError {
    CssSyntaxError { message: message.into(), offset }
}

struct Buf<'a> {
    input: &'a str,
    alloc: &'a Allocator,
    start: usize,
    end: usize,
    scratch: Option<ArenaString<'a>>,
}

impl<'a> Buf<'a> {
    #[inline]
    fn is_empty(&self) -> bool {
        match &self.scratch {
            Some(s) => s.is_empty(),
            None => self.start == self.end,
        }
    }

    #[inline]
    fn push_range(&mut self, from: usize, to: usize) {
        match &mut self.scratch {
            Some(s) => s.push_str(&self.input[from..to]),
            None if self.start == self.end => {
                self.start = from;
                self.end = to;
            }
            None if self.end == from => self.end = to,
            None => {
                let mut s = ArenaString::with_capacity_in(self.end - self.start + to - from + 16, self.alloc);
                s.push_str(&self.input[self.start..self.end]);
                s.push_str(&self.input[from..to]);
                self.scratch = Some(s);
            }
        }
    }

    fn push_space(&mut self) {
        let s = self.scratch.get_or_insert_with(|| {
            let mut s = ArenaString::with_capacity_in(self.end - self.start + 16, self.alloc);
            s.push_str(&self.input[self.start..self.end]);
            s
        });
        s.push(' ');
    }

    #[inline]
    fn first(&self) -> Option<u8> {
        self.as_bytes().first().copied()
    }

    #[inline]
    fn last(&self) -> Option<u8> {
        self.as_bytes().last().copied()
    }

    fn as_bytes(&self) -> &[u8] {
        match &self.scratch {
            Some(s) => s.as_bytes(),
            None => &self.input.as_bytes()[self.start..self.end],
        }
    }

    fn take(&mut self) -> &'a str {
        let out = match self.scratch.take() {
            Some(s) => s.into_bump_str(),
            None => &self.input[self.start..self.end],
        };
        self.start = 0;
        self.end = 0;
        out
    }

    fn clear(&mut self) {
        self.scratch = None;
        self.start = 0;
        self.end = 0;
    }
}

static SPECIAL: ByteTable = ByteTable::new(b"\r\\/'\" \n\t-;{}()");
static CUSTOM_PROPERTY: ByteTable = ByteTable::new(b"\\'\"/:;([{}])");

struct Open<'a> {
    rule: Node<'a>,
    first_child: usize,
}

pub fn parse<'a>(input: &'a str, alloc: &'a Allocator) -> Result<Nodes<'a>, CssSyntaxError> {
    let input: &'a str = match input.strip_prefix('\u{FEFF}') {
        Some(rest) => orisu_allocator::afmt!(alloc, " {rest}"),
        None => input,
    };
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut ast: Nodes<'a> = Vec::new_in(alloc);
    let mut license: Nodes<'a> = Vec::new_in(alloc);
    let mut stack: std::vec::Vec<Open<'a>> = std::vec::Vec::new();
    let mut children: std::vec::Vec<Node<'a>> = std::vec::Vec::new();
    let mut closing: SmallVec<[u8; 32]> = SmallVec::new();
    let mut buf = Buf { input, alloc, start: 0, end: 0, scratch: None };
    let at = |i: usize| bytes.get(i).copied().unwrap_or(0);

    macro_rules! push_node {
        ($node:expr) => {{
            let node = $node;
            if stack.is_empty() { ast.push(node) } else { children.push(node) }
        }};
    }

    let mut i = 0;
    while i < len {
        let ch = bytes[i];
        if !SPECIAL.has(ch) {
            let end = SPECIAL.find(bytes, i).unwrap_or(len);
            buf.push_range(i, end);
            i = end;
            continue;
        }
        match ch {
            b'\r' if at(i + 1) == b'\n' => {}
            b'\\' => {
                let end = (i + 1 + input[i + 1..].chars().next().map_or(0, char::len_utf8)).min(len);
                buf.push_range(i, end);
                i = end;
                continue;
            }
            b'/' if at(i + 1) == b'*' => {
                let start = i;
                if let Some(end) = comment_end(bytes, i + 2) {
                    i = end;
                }
                if i > start && at(start + 2) == b'!' {
                    license.push(Node::Comment(&input[start + 2..i - 1]));
                }
            }
            b'\'' | b'"' => {
                let end = parse_string(input, i, ch)?;
                buf.push_range(i, end + 1);
                i = end;
            }
            b' ' | b'\n' | b'\t' if matches!(at(i + 1), b' ' | b'\n' | b'\t') || (at(i + 1) == b'\r' && at(i + 2) == b'\n') => {}
            b'\n' => {
                if !buf.is_empty() && !matches!(buf.last(), Some(b' ' | b'\n' | b'\t')) {
                    buf.push_space();
                }
            }
            b'-' if at(i + 1) == b'-' && buf.is_empty() => {
                let mut nested: SmallVec<[u8; 16]> = SmallVec::new();
                let start = i;
                let mut colon = None;
                let mut j = i + 2;
                let mut value = None;
                let mut value_end = 0;
                while j < len {
                    let c = bytes[j];
                    if !CUSTOM_PROPERTY.has(c) {
                        let end = CUSTOM_PROPERTY.find(bytes, j).unwrap_or(len);
                        if let Some(last) = bytes[j..end].iter().rposition(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r')) {
                            value_end = j + last + 1 - start;
                        }
                        j = end;
                        continue;
                    }
                    match c {
                        b'\\' => j += 1,
                        b'\'' | b'"' => j = parse_string(input, j, c)?,
                        b'/' if at(j + 1) == b'*' => {
                            if let Some(end) = comment_end(bytes, j + 2) {
                                j = end;
                            }
                            j += 1;
                            continue;
                        }
                        b':' if colon.is_none() => colon = Some(j - start),
                        b';' if nested.is_empty() => {
                            value = Some(&input[start..j]);
                            i = j;
                            break;
                        }
                        b'(' => nested.push(b')'),
                        b'[' => nested.push(b']'),
                        b'{' => nested.push(b'}'),
                        b'}' if nested.is_empty() => {
                            value = Some(&input[start..j]);
                            i = j - 1;
                            break;
                        }
                        b')' | b']' | b'}' if nested.last() == Some(&c) => {
                            nested.pop();
                        }
                        _ => {}
                    }
                    value_end = (j + 1).min(len) - start;
                    j += 1;
                }
                let Some(colon) = colon else {
                    return Err(error("Invalid custom property, expected a value", Some(start)));
                };
                let value = value.unwrap_or_else(|| {
                    i = len;
                    &input[start..]
                });
                let declaration = parse_declaration(value, colon, value_end.min(value.len()));
                push_node!(declaration);
                buf.clear();
            }
            b';' if buf.first() == Some(b'@') => {
                let text = buf.take();
                push_node!(parse_at_rule(text, Vec::new_in(alloc)));
            }
            b';' if closing.last() != Some(&b')') => {
                let text = buf.take();
                match text.find(':') {
                    Some(colon) => push_node!(parse_declaration(text, colon, text.len())),
                    None if text.is_empty() => {}
                    None => {
                        return Err(error(format!("Invalid declaration: `{}`", trim(text)), Some(i)));
                    }
                }
            }
            b'{' if closing.last() != Some(&b')') => {
                closing.push(b'}');
                let text = trim(buf.take());
                let rule = if text.starts_with('@') {
                    parse_at_rule(text, Vec::new_in(alloc))
                } else {
                    Node::Rule { selector: text, nodes: Vec::new_in(alloc) }
                };
                stack.push(Open { rule, first_child: children.len() });
            }
            b'}' if closing.last() != Some(&b')') => {
                if closing.pop().is_none() {
                    return Err(error("Missing opening {", Some(i)));
                }
                if !buf.is_empty() {
                    if buf.first() == Some(b'@') {
                        let text = buf.take();
                        push_node!(parse_at_rule(text, Vec::new_in(alloc)));
                    } else {
                        let text = buf.take();
                        if !stack.is_empty() {
                            let Some(colon) = text.find(':') else {
                                return Err(error(format!("Invalid declaration: `{}`", trim(text)), Some(i)));
                            };
                            push_node!(parse_declaration(text, colon, text.len()));
                        }
                    }
                }
                if let Some(Open { mut rule, first_child }) = stack.pop() {
                    if let Some(nodes) = rule.children_mut() {
                        nodes.reserve_exact(children.len() - first_child);
                        nodes.extend(children.drain(first_child..));
                    }
                    push_node!(rule);
                }
                buf.clear();
            }
            b'(' => {
                closing.push(b')');
                buf.push_range(i, i + 1);
            }
            b')' => {
                if closing.last() != Some(&b')') {
                    return Err(error("Missing opening (", Some(i)));
                }
                closing.pop();
                buf.push_range(i, i + 1);
            }
            b' ' | b'\t' if buf.is_empty() => {}
            _ => buf.push_range(i, i + 1),
        }
        i += 1;
    }

    if buf.first() == Some(b'@') {
        let text = buf.take();
        ast.push(parse_at_rule(text, Vec::new_in(alloc)));
    } else if !buf.is_empty() && closing.is_empty() {
        let text = buf.take();
        if let Some(colon) = text.find(':') {
            ast.push(parse_declaration(text, colon, text.len()));
        }
    }

    if !closing.is_empty()
        && let Some(open) = stack.last()
    {
        let message = match &open.rule {
            Node::Rule { selector, .. } => format!("Missing closing }} at {selector}"),
            Node::AtRule { name, params, .. } => format!("Missing closing }} at {name} {params}"),
            _ => "Missing closing }".to_owned(),
        };
        return Err(error(message, None));
    }

    if license.is_empty() {
        Ok(ast)
    } else {
        license.extend(ast);
        Ok(license)
    }
}

/// `value_end` is where the value ends without the trailing whitespace and comments a custom property keeps;
/// `!important` is the flag only right before it.
fn parse_declaration(buffer: &str, colon: usize, value_end: usize) -> Node<'_> {
    let property = trim(&buffer[..colon]);
    let value = trim_value(buffer.get(colon + 1..value_end).unwrap_or(""));
    if let Some(rest) = value.strip_suffix("!important")
        && !is_escaped(value, rest.len())
    {
        return Node::Decl { property, value: Some(trim_value(rest)), important: true };
    }
    Node::Decl { property, value: Some(trim_value(&buffer[colon + 1..])), important: false }
}

/// Trims like [`trim`], but keeps an escaped whitespace at the end, as in `a\ !important`.
fn trim_value(value: &str) -> &str {
    let trimmed = trim(value);
    let start = trimmed.as_ptr().addr() - value.as_ptr().addr();
    let end = start + trimmed.len();
    if end < value.len() && is_escaped(value, end) {
        let escaped = value[end..].chars().next().map_or(0, char::len_utf8);
        return &value[start..end + escaped];
    }
    trimmed
}

/// Whether an odd number of `\` precedes `idx`.
fn is_escaped(value: &str, idx: usize) -> bool {
    value.as_bytes()[..idx].iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1
}

#[inline]
fn comment_end(bytes: &[u8], from: usize) -> Option<usize> {
    memchr::memmem::find(&bytes[from.min(bytes.len())..], b"*/").map(|offset| from + offset + 1)
}

fn parse_string(input: &str, start: usize, quote: u8) -> Result<usize, CssSyntaxError> {
    let bytes = input.as_bytes();
    let mut from = start + 1;
    while let Some(offset) = memchr::memchr3(quote, b'\\', b'\n', &bytes[from.min(bytes.len())..]) {
        let at = from + offset;
        match bytes[at] {
            b'\\' => from = at + 2,
            b'\n' => {
                let cr = at > from && bytes[at - 1] == b'\r';
                let semi = if cr { at - 1 } else { at };
                if semi > from && bytes[semi - 1] == b';' {
                    return Err(error(format!("Unterminated string: {}{}", &input[start..semi], quote as char), Some(start)));
                }
                let end = if cr { at - 1 } else { at };
                return Err(error(format!("Unterminated string: {}{}", &input[start..end], quote as char), Some(start)));
            }
            _ => return Ok(at),
        }
    }
    Ok(start)
}
