// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::bytes::trim;
use orisu_allocator::{Allocator, Vec};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ctx<'a> {
    pub base: Option<&'a str>,
    pub source_base: Option<&'a str>,
    pub reference: Option<bool>,
    pub theme: Option<bool>,
}

impl<'a> Ctx<'a> {
    pub fn merge(self, other: Ctx<'a>) -> Ctx<'a> {
        Ctx {
            base: other.base.or(self.base),
            source_base: other.source_base.or(self.source_base),
            reference: other.reference.or(self.reference),
            theme: other.theme.or(self.theme),
        }
    }

    pub fn is_reference(&self) -> bool {
        self.reference == Some(true)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Node<'a> {
    Rule { selector: &'a str, nodes: Vec<'a, Node<'a>> },
    AtRule { name: &'a str, params: &'a str, nodes: Vec<'a, Node<'a>> },
    Decl { property: &'a str, value: Option<&'a str>, important: bool },
    Comment(&'a str),
    Context { ctx: Ctx<'a>, nodes: Vec<'a, Node<'a>> },
    AtRoot(Vec<'a, Node<'a>>),
}

pub type Nodes<'a> = Vec<'a, Node<'a>>;

#[derive(Clone, Copy)]
pub struct Builder<'a> {
    pub alloc: &'a Allocator,
}

impl<'a> Builder<'a> {
    pub fn new(alloc: &'a Allocator) -> Self {
        Self { alloc }
    }

    #[inline]
    pub fn vec(self) -> Nodes<'a> {
        Vec::new_in(self.alloc)
    }

    #[inline]
    pub fn vec1(self, node: Node<'a>) -> Nodes<'a> {
        let mut v = Vec::with_capacity_in(1, self.alloc);
        v.push(node);
        v
    }

    pub fn vec_from(self, nodes: impl IntoIterator<Item = Node<'a>>) -> Nodes<'a> {
        let mut v = Vec::new_in(self.alloc);
        v.extend(nodes);
        v
    }

    #[inline]
    pub fn decl(self, property: &'a str, value: &'a str) -> Node<'a> {
        Node::Decl { property, value: Some(value), important: false }
    }

    #[inline]
    pub fn rule(self, selector: &'a str, nodes: Nodes<'a>) -> Node<'a> {
        if selector.starts_with('@') { parse_at_rule(selector, nodes) } else { Node::Rule { selector, nodes } }
    }

    #[inline]
    pub fn style_rule(self, selector: &'a str, nodes: Nodes<'a>) -> Node<'a> {
        Node::Rule { selector, nodes }
    }

    #[inline]
    pub fn at_rule(self, name: &'a str, params: &'a str, nodes: Nodes<'a>) -> Node<'a> {
        Node::AtRule { name, params, nodes }
    }

    #[inline]
    pub fn at_root(self, nodes: Nodes<'a>) -> Node<'a> {
        Node::AtRoot(nodes)
    }

    #[inline]
    pub fn context(self, ctx: Ctx<'a>, nodes: Nodes<'a>) -> Node<'a> {
        Node::Context { ctx, nodes }
    }
}

impl<'a> Node<'a> {
    pub fn children(&self) -> Option<&Nodes<'a>> {
        match self {
            Node::Rule { nodes, .. } | Node::AtRule { nodes, .. } | Node::Context { nodes, .. } | Node::AtRoot(nodes) => Some(nodes),
            _ => None,
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Nodes<'a>> {
        match self {
            Node::Rule { nodes, .. } | Node::AtRule { nodes, .. } | Node::Context { nodes, .. } | Node::AtRoot(nodes) => Some(nodes),
            _ => None,
        }
    }

    pub fn is_at_rule(&self, at: &str) -> bool {
        matches!(self, Node::AtRule { name, .. } if *name == at)
    }

    /// Deep-copies the tree into `alloc`, which may be shorter-lived than the one holding it.
    pub fn clone_in<'s>(&self, alloc: &'s Allocator) -> Node<'s>
    where
        'a: 's,
    {
        match self {
            Node::Rule { selector, nodes } => Node::Rule { selector, nodes: clone_in(nodes, alloc) },
            Node::AtRule { name, params, nodes } => Node::AtRule { name, params, nodes: clone_in(nodes, alloc) },
            Node::Decl { property, value, important } => Node::Decl { property, value: *value, important: *important },
            Node::Comment(text) => Node::Comment(text),
            Node::Context { ctx, nodes } => Node::Context { ctx: *ctx, nodes: clone_in(nodes, alloc) },
            Node::AtRoot(nodes) => Node::AtRoot(clone_in(nodes, alloc)),
        }
    }
}

pub fn clone_in<'a: 's, 's>(nodes: &[Node<'a>], alloc: &'s Allocator) -> Nodes<'s> {
    let mut out = Vec::with_capacity_in(nodes.len(), alloc);
    out.extend(nodes.iter().map(|node| node.clone_in(alloc)));
    out
}

pub fn walk<'a>(ast: &[Node<'a>], f: &mut impl FnMut(&Node<'a>) -> bool) {
    for node in ast {
        if f(node)
            && let Some(children) = node.children()
        {
            walk(children, f);
        }
    }
}

pub fn walk_mut<'a>(ast: &mut [Node<'a>], f: &mut impl FnMut(&mut Node<'a>) -> bool) {
    for node in ast {
        if f(node)
            && let Some(children) = node.children_mut()
        {
            walk_mut(children, f);
        }
    }
}

pub enum Walk<'a> {
    Continue,
    Skip,
    Stop,
    /// Splices in the nodes and visits them next.
    Replace(Nodes<'a>),
    /// Splices in the nodes and continues after them.
    ReplaceSkip(Nodes<'a>),
    ReplaceStop(Nodes<'a>),
}

/// Merged `Context` values of the ancestors and whether any non-context ancestor exists.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scope<'a> {
    pub ctx: Ctx<'a>,
    pub nested: bool,
}

/// Port of Tailwind's `walk` with replacement actions; returns `Ok(true)` once stopped.
pub fn walk_scoped<'a, E>(
    nodes: &mut Nodes<'a>,
    scope: Scope<'a>,
    f: &mut impl FnMut(&mut Node<'a>, Scope<'a>) -> Result<Walk<'a>, E>,
) -> Result<bool, E> {
    let mut i = 0;
    while i < nodes.len() {
        match f(&mut nodes[i], scope)? {
            Walk::Continue => {
                let node = &mut nodes[i];
                let inner = match node {
                    Node::Context { ctx, .. } => Scope { ctx: scope.ctx.merge(*ctx), nested: scope.nested },
                    _ => Scope { ctx: scope.ctx, nested: true },
                };
                if let Some(children) = node.children_mut()
                    && walk_scoped(children, inner, f)?
                {
                    return Ok(true);
                }
                i += 1;
            }
            Walk::Skip => i += 1,
            Walk::Stop => return Ok(true),
            Walk::Replace(new) => {
                nodes.splice(i..=i, new);
            }
            Walk::ReplaceSkip(new) => {
                let len = new.len();
                nodes.splice(i..=i, new);
                i += len;
            }
            Walk::ReplaceStop(new) => {
                nodes.splice(i..=i, new);
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn to_css(ast: &[Node<'_>]) -> String {
    let mut out = String::with_capacity(ast.len() * 96);
    write_css(ast, &mut out);
    out
}

pub fn write_css(ast: &[Node<'_>], out: &mut String) {
    for node in ast {
        stringify(node, 0, out);
    }
}

const INDENT: &str = "                                                                ";

#[inline]
fn indent(out: &mut String, depth: usize) {
    let n = depth * 2;
    if n <= INDENT.len() {
        out.push_str(&INDENT[..n]);
    } else {
        out.extend(std::iter::repeat_n(' ', n));
    }
}

fn stringify(node: &Node<'_>, depth: usize, out: &mut String) {
    match node {
        Node::Decl { property, value, important } => {
            indent(out, depth);
            out.push_str(property);
            out.push_str(": ");
            out.push_str(value.unwrap_or("undefined"));
            out.push_str(if *important { " !important;\n" } else { ";\n" });
        }
        Node::Rule { selector, nodes } => {
            indent(out, depth);
            out.push_str(selector);
            out.push_str(" {\n");
            for child in nodes {
                stringify(child, depth + 1, out);
            }
            indent(out, depth);
            out.push_str("}\n");
        }
        Node::AtRule { name, params, nodes } => {
            indent(out, depth);
            out.push_str(name);
            if nodes.is_empty() {
                out.push(' ');
                out.push_str(params);
                out.push_str(";\n");
                return;
            }
            if !params.is_empty() {
                out.push(' ');
                out.push_str(params);
            }
            out.push_str(" {\n");
            for child in nodes {
                stringify(child, depth + 1, out);
            }
            indent(out, depth);
            out.push_str("}\n");
        }
        Node::Comment(value) => {
            indent(out, depth);
            out.push_str("/*");
            out.push_str(value);
            out.push_str("*/\n");
        }
        Node::Context { .. } | Node::AtRoot(_) => {}
    }
}

/// Splits `@name params` the way Tailwind does: at the first space, tab or `(` past the shortest at-rule name.
pub fn parse_at_rule<'a>(buffer: &'a str, nodes: Nodes<'a>) -> Node<'a> {
    let bytes = buffer.as_bytes();
    let split = (5..bytes.len()).find(|&i| matches!(bytes[i], b' ' | b'\t' | b'('));
    let (name, params) = match split {
        Some(i) => (&buffer[..i], &buffer[i..]),
        None => (buffer, ""),
    };
    Node::AtRule { name: trim(name), params: trim(params), nodes }
}
