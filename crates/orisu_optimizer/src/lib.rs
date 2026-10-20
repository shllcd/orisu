// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use indexmap::IndexSet;
use orisu_allocator::{Allocator, ArenaString};
use orisu_ast::{Builder, Node, Nodes};
use orisu_parser::misc::extract_used_variables;
use orisu_parser::segment::segment;
use orisu_parser::selector::{self as sp, SNode};
use orisu_parser::value::{self as vp, ValueNode};
use orisu_theme::{Theme, UsedVars, opts};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};

pub mod polyfills {
    pub const NONE: u8 = 0;
    pub const AT_PROPERTY: u8 = 1 << 0;
    pub const COLOR_MIX: u8 = 1 << 1;
    pub const ALL: u8 = AT_PROPERTY | COLOR_MIX;
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Root {
    Main,
    AtRoots,
    Temp,
}

struct Entry<'a> {
    root: Root,
    path: Vec<usize>,
    span: usize,
    property: &'a str,
    value: &'a str,
}

struct KeyframeEntry<'a> {
    root: Root,
    path: Vec<usize>,
    params: &'a str,
}

#[derive(Clone, Copy, Default)]
struct OCtx {
    theme: bool,
    keyframes: bool,
    supports_color_mix: bool,
}

/// The children of `at` are replaced by `nodes` while optimizing.
pub struct Splice<'r, 'a> {
    pub at: &'r Node<'a>,
    pub nodes: SpliceNodes<'r, 'a>,
}

pub enum SpliceNodes<'r, 'a> {
    Nodes(&'r [&'r Node<'a>]),
    Fragments(&'r [&'r Fragment<'a>]),
}

static FRAGMENTS: &str = "orisu:fragments";

/// Where a splice sits, when its nodes can be optimized one by one ahead of time.
pub struct SpliceContext<'a> {
    ctx: OCtx,
    depth: usize,
    wrappers: Vec<(&'a str, &'a str)>,
}

/// One spliced node, transformed and nested on its own.
pub struct Fragment<'a> {
    is_empty: bool,
    nested: Nodes<'a>,
    /// The containers of `nested` whose declarations get deduplicated.
    dedupe: Vec<Vec<usize>>,
    keyframes: Vec<&'a str>,
    at_roots: Nodes<'a>,
}

struct State<'a, 't, 'th> {
    b: Builder<'a>,
    theme: &'t Theme<'th>,
    used: &'t mut UsedVars,
    splice: Option<Splice<'t, 'a>>,
    polyfills: u8,
    at_roots: Nodes<'a>,
    seen_at_properties: FxHashSet<&'a str>,
    theme_vars: Vec<Entry<'a>>,
    keyframes: Vec<KeyframeEntry<'a>>,
    used_keyframes: FxHashSet<&'a str>,
    fallbacks_root: Nodes<'a>,
    fallbacks_universal: Nodes<'a>,
    deps: FxHashMap<&'a str, IndexSet<&'a str, FxBuildHasher>>,
    /// Where the node being transformed goes, as indices from the root.
    path: Vec<usize>,
    defer_at_roots: bool,
    fragments: &'t [&'t Fragment<'a>],
}

pub fn track_used_variables<'a>(theme: &Theme<'_>, used: &mut UsedVars, raw: &'a str, alloc: &'a Allocator) {
    for variable in extract_used_variables(raw, alloc) {
        theme.mark_used_variable(variable, used);
    }
}

fn keyframe_names(value: &str) -> impl Iterator<Item = &str> {
    value.split(|c: char| c.is_whitespace() || c == ',')
}

impl<'a, 'th: 'a> State<'a, '_, 'th> {
    fn transform(&mut self, node: &Node<'a>, parent: &mut Nodes<'a>, root: Root, ctx: OCtx, depth: usize) {
        match node {
            &Node::Decl { property, value, important } => {
                let Some(value) = value else { return };
                if property == "--tw-sort" {
                    return;
                }
                let is_theme_var = ctx.theme && property.starts_with("--");
                if is_theme_var && value == "initial" {
                    return;
                }
                let record = is_theme_var && !ctx.keyframes;
                if value.contains("var(") {
                    if is_theme_var {
                        for variable in extract_used_variables(value, self.b.alloc) {
                            self.deps.entry(variable).or_default().insert(property);
                        }
                    } else {
                        track_used_variables(self.theme, self.used, value, self.b.alloc);
                    }
                }
                if property == "animation" {
                    for name in keyframe_names(value) {
                        self.used_keyframes.insert(name);
                    }
                }
                let fallback = if self.polyfills & polyfills::COLOR_MIX != 0
                    && !ctx.supports_color_mix
                    && !ctx.keyframes
                    && value.contains("color-mix(")
                {
                    color_mix_fallback(value, self.theme, self.b.alloc)
                } else {
                    None
                };
                let index = parent.len();
                let declaration = Node::Decl { property, value: Some(value), important };
                let span = match fallback {
                    Some(fallback) => {
                        parent.push(Node::Decl { property, value: Some(fallback), important });
                        parent.push(self.b.rule("@supports (color: color-mix(in lab, red, red))", self.b.vec1(declaration)));
                        2
                    }
                    None => {
                        parent.push(declaration);
                        1
                    }
                };
                if record {
                    let mut path = self.path.clone();
                    path.push(index);
                    self.theme_vars.push(Entry { root, path, span, property, value });
                }
            }
            &Node::Rule { selector, ref nodes } => {
                self.path.push(parent.len());
                let mut out = orisu_allocator::Vec::with_capacity_in(nodes.len(), self.b.alloc);
                for child in nodes {
                    self.transform(child, &mut out, root, ctx, depth + 1);
                }
                self.path.pop();
                if !out.is_empty() {
                    parent.push(Node::Rule { selector, nodes: out });
                }
            }
            &Node::AtRule { name, params, ref nodes } if name == "@property" && depth == 0 => {
                if self.seen_at_properties.contains(params) {
                    return;
                }
                if self.polyfills & polyfills::AT_PROPERTY != 0 {
                    let mut initial = None;
                    let mut inherits = false;
                    for prop in nodes {
                        if let Node::Decl { property, value, .. } = prop {
                            if *property == "initial-value" {
                                initial = *value;
                            } else if *property == "inherits" {
                                inherits = *value == Some("true");
                            }
                        }
                    }
                    let fallback = Node::Decl { property: params, value: Some(initial.unwrap_or("initial")), important: false };
                    if inherits {
                        self.fallbacks_root.push(fallback);
                    } else {
                        self.fallbacks_universal.push(fallback);
                    }
                }
                self.seen_at_properties.insert(params);
                self.path.push(parent.len());
                let mut out = self.b.vec();
                for child in nodes {
                    self.transform(child, &mut out, root, ctx, depth + 1);
                }
                self.path.pop();
                parent.push(Node::AtRule { name, params, nodes: out });
            }
            &Node::AtRule { name, params, ref nodes } => {
                let mut ctx = ctx;
                if name == "@keyframes" {
                    ctx.keyframes = true;
                } else if name == "@supports" && params.contains("color-mix(") {
                    ctx.supports_color_mix = true;
                }
                self.path.push(parent.len());
                let mut out = self.b.vec();
                for child in nodes {
                    self.transform(child, &mut out, root, ctx, depth + 1);
                }
                let path = self.path.clone();
                self.path.pop();
                let keep = !out.is_empty() || matches!(name, "@layer" | "@charset" | "@custom-media" | "@namespace" | "@import" | "@apply");
                if name == "@keyframes" && ctx.theme && keep {
                    self.keyframes.push(KeyframeEntry { root, path, params });
                }
                if keep {
                    parent.push(Node::AtRule { name, params, nodes: out });
                }
            }
            Node::AtRoot(_) if self.defer_at_roots => self.at_roots.push(node.clone_in(self.b.alloc)),
            Node::AtRoot(nodes) => {
                for child in nodes {
                    let start_vars = self.theme_vars.len();
                    let start_kf = self.keyframes.len();
                    let mut temp = self.b.vec();
                    let path = std::mem::take(&mut self.path);
                    self.transform(child, &mut temp, Root::Temp, ctx, 0);
                    self.path = path;
                    let offset = self.at_roots.len();
                    for e in &mut self.theme_vars[start_vars..] {
                        if e.root == Root::Temp {
                            e.root = Root::AtRoots;
                            e.path[0] += offset;
                        }
                    }
                    for e in &mut self.keyframes[start_kf..] {
                        if e.root == Root::Temp {
                            e.root = Root::AtRoots;
                            e.path[0] += offset;
                        }
                    }
                    self.at_roots.extend(temp);
                }
            }
            Node::Context { ctx: node_ctx, nodes } => {
                if node_ctx.is_reference() {
                    return;
                }
                let mut ctx = ctx;
                if let Some(theme) = node_ctx.theme {
                    ctx.theme = theme;
                }
                if let Some(splice) = self.splice.take_if(|splice| std::ptr::eq(splice.at, node)) {
                    match splice.nodes {
                        SpliceNodes::Nodes(nodes) => {
                            for child in nodes {
                                self.transform(child, parent, root, ctx, depth);
                            }
                        }
                        SpliceNodes::Fragments(fragments) => {
                            for fragment in fragments {
                                self.used_keyframes.extend(&fragment.keyframes);
                                for at_root in &fragment.at_roots {
                                    self.transform(at_root, parent, root, ctx, depth);
                                }
                            }
                            if fragments.iter().any(|f| !f.is_empty) {
                                parent.push(Node::Comment(FRAGMENTS));
                            }
                            self.fragments = fragments;
                        }
                    }
                    return;
                }
                for child in nodes {
                    self.transform(child, parent, root, ctx, depth);
                }
            }
            &Node::Comment(c) => parent.push(Node::Comment(c)),
        }
    }

    fn is_variable_used(&self, variable: &'a str, seen: &mut FxHashSet<&'a str>) -> bool {
        if !seen.insert(variable) {
            return true;
        }
        if self.theme.get_options(variable, self.used) & (opts::STATIC | opts::USED) != 0 {
            return true;
        }
        if let Some(deps) = self.deps.get(variable) {
            for dep in deps {
                if self.is_variable_used(dep, seen) {
                    return true;
                }
            }
        }
        false
    }
}

fn container_mut<'r, 'a>(root: &'r mut Nodes<'a>, path: &[usize]) -> Option<&'r mut Nodes<'a>> {
    let mut cur = root;
    for &i in path {
        cur = cur.get_mut(i)?.children_mut()?;
    }
    Some(cur)
}

fn node_at<'r, 'a>(root: &'r [Node<'a>], path: &[usize]) -> Option<&'r Node<'a>> {
    let (first, rest) = path.split_first()?;
    let mut cur = root.get(*first)?;
    for &i in rest {
        cur = cur.children()?.get(i)?;
    }
    Some(cur)
}

pub fn optimize_ast<'a, 'th: 'a>(
    ast: &[Node<'a>],
    theme: &Theme<'th>,
    used: &mut UsedVars,
    polyfill_flags: u8,
    alloc: &'a Allocator,
) -> Nodes<'a> {
    optimize_spliced(ast, None, theme, used, polyfill_flags, alloc)
}

pub fn optimize_spliced<'a, 'th: 'a>(
    ast: &[Node<'a>],
    splice: Option<Splice<'_, 'a>>,
    theme: &Theme<'th>,
    used: &mut UsedVars,
    polyfill_flags: u8,
    alloc: &'a Allocator,
) -> Nodes<'a> {
    let b = Builder::new(alloc);
    let mut st = State {
        b,
        theme,
        used,
        splice,
        polyfills: polyfill_flags,
        at_roots: b.vec(),
        seen_at_properties: FxHashSet::default(),
        theme_vars: Vec::new(),
        keyframes: Vec::new(),
        used_keyframes: FxHashSet::default(),
        fallbacks_root: b.vec(),
        fallbacks_universal: b.vec(),
        deps: FxHashMap::default(),
        path: Vec::new(),
        defer_at_roots: false,
        fragments: &[],
    };
    let mut new_ast = orisu_allocator::Vec::with_capacity_in(ast.len(), alloc);
    for node in ast {
        st.transform(node, &mut new_ast, Root::Main, OCtx::default(), 0);
    }

    let animate_prefix = theme.prefix_key("--animate-", alloc);
    let mut removals: Vec<(Root, Vec<usize>, usize)> = Vec::new();
    let theme_vars = std::mem::take(&mut st.theme_vars);
    for entry in theme_vars {
        let mut seen = FxHashSet::default();
        if st.is_variable_used(entry.property, &mut seen) {
            if entry.property.starts_with(animate_prefix) {
                for name in keyframe_names(entry.value) {
                    st.used_keyframes.insert(name);
                }
            }
            continue;
        }
        removals.push((entry.root, entry.path, entry.span));
    }
    removals.sort_by(|a, b| b.cmp(a));
    let mut removed: Vec<Vec<usize>> = Vec::new();
    for (root, path, span) in removals {
        let tree = if root == Root::Main { &mut new_ast } else { &mut st.at_roots };
        if root == Root::Main && removed.iter().any(|r| path.starts_with(r)) {
            continue;
        }
        let (index, parent_path) = path.split_last().map(|(i, p)| (*i, p.to_vec())).unwrap_or((0, Vec::new()));
        let Some(container) = container_mut(tree, &parent_path) else {
            continue;
        };
        if index + span > container.len() {
            continue;
        }
        container.drain(index..index + span);
        if root != Root::Main || !container.is_empty() || parent_path.is_empty() {
            continue;
        }
        if !matches!(node_at(tree, &parent_path), Some(Node::Rule { .. })) {
            continue;
        }
        let mut chain = parent_path;
        while let Some(&last) = chain.last() {
            let holder = &chain[..chain.len() - 1];
            let removable = holder.is_empty() || matches!(node_at(tree, holder), Some(Node::AtRule { .. }));
            if !removable {
                break;
            }
            let Some(container) = container_mut(tree, holder) else {
                break;
            };
            if last < container.len() {
                container.remove(last);
            }
            removed.push(chain.clone());
            chain.pop();
        }
    }

    let mut kf_removals: Vec<usize> = st
        .keyframes
        .iter()
        .filter(|k| k.root == Root::AtRoots && k.path.len() == 1 && !st.used_keyframes.contains(k.params))
        .map(|k| k.path[0])
        .collect();
    kf_removals.sort_unstable_by(|a, b| b.cmp(a));
    kf_removals.dedup();
    for i in kf_removals {
        if i < st.at_roots.len() {
            st.at_roots.remove(i);
        }
    }

    new_ast.append(&mut st.at_roots);

    if polyfill_flags & polyfills::AT_PROPERTY != 0 {
        let mut fallback_ast = b.vec();
        if !st.fallbacks_root.is_empty() {
            fallback_ast.push(b.style_rule(":root, :host", std::mem::replace(&mut st.fallbacks_root, b.vec())));
        }
        if !st.fallbacks_universal.is_empty() {
            let nodes = std::mem::replace(&mut st.fallbacks_universal, b.vec());
            fallback_ast.push(b.style_rule("*, ::before, ::after, ::backdrop", nodes));
        }
        if !fallback_ast.is_empty() {
            let first_valid = new_ast
                .iter()
                .position(|n| match n {
                    Node::Comment(_) => false,
                    Node::AtRule { name, .. } => *name != "@charset" && *name != "@import",
                    _ => true,
                })
                .unwrap_or(new_ast.len());
            new_ast.insert(first_valid, b.at_rule("@layer", "properties", b.vec()));
            new_ast.push(b.at_rule(
                "@layer",
                "properties",
                b.vec1(b.at_rule(
                    "@supports",
                    "((-webkit-hyphens: none) and (not (margin-trim: inline))) or ((-moz-orient: inline) and (not (color:rgb(from red r g b))))",
                    fallback_ast,
                )),
            ));
        }
    }

    let (mut ast, paths) = nest(new_ast, Vec::new(), st.fragments, alloc);
    dedupe_decls(&mut ast, paths);
    ast
}

/// `None` when the splice at `path` is not directly inside hoistable at-rules, or a later sibling could continue its last rule.
pub fn splice_context<'a>(ast: &[Node<'a>], path: &[usize]) -> Option<SpliceContext<'a>> {
    let mut cx = SpliceContext { ctx: OCtx::default(), depth: 0, wrappers: Vec::new() };
    let mut nodes = ast;
    let mut in_context = false;
    let mut tail_ok = true;
    for &i in path {
        let node = nodes.get(i)?;
        let later_ok = nodes[i + 1..].iter().all(|n| matches!(n, Node::Rule { .. } | Node::AtRule { .. }));
        tail_ok = later_ok && (tail_ok || !in_context);
        match node {
            Node::Context { ctx, nodes: children } => {
                if ctx.is_reference() {
                    return None;
                }
                if let Some(theme) = ctx.theme {
                    cx.ctx.theme = theme;
                }
                in_context = true;
                nodes = children;
            }
            &Node::AtRule { name, params, nodes: ref children } if HOISTABLE.contains(&name) => {
                if name == "@supports" && params.contains("color-mix(") {
                    cx.ctx.supports_color_mix = true;
                }
                cx.wrappers.push((name, params));
                cx.depth += 1;
                in_context = false;
                nodes = children;
            }
            _ => return None,
        }
    }
    (in_context && tail_ok && !cx.wrappers.is_empty() && !cx.ctx.theme).then_some(cx)
}

/// One fragment per node, `None` where the node declares theme variables, which only the whole stylesheet can prune, or
/// becomes something other than rules.
pub fn fragments<'a, 'th: 'a, 'n>(
    nodes: impl IntoIterator<Item = &'n Node<'a>>,
    cx: &SpliceContext<'a>,
    theme: &Theme<'th>,
    used: &mut UsedVars,
    polyfill_flags: u8,
    alloc: &'a Allocator,
) -> Vec<Option<Fragment<'a>>>
where
    'a: 'n,
{
    let b = Builder::new(alloc);
    let mut st = State {
        b,
        theme,
        used,
        splice: None,
        polyfills: polyfill_flags,
        at_roots: b.vec(),
        seen_at_properties: FxHashSet::default(),
        theme_vars: Vec::new(),
        keyframes: Vec::new(),
        used_keyframes: FxHashSet::default(),
        fallbacks_root: b.vec(),
        fallbacks_universal: b.vec(),
        deps: FxHashMap::default(),
        path: Vec::new(),
        defer_at_roots: true,
        fragments: &[],
    };
    let mut n = Nester::new(b, cx.wrappers.clone(), &[], 0);
    nodes
        .into_iter()
        .map(|node| {
            let mut out = b.vec();
            st.transform(node, &mut out, Root::Main, cx.ctx, cx.depth);
            let at_roots = std::mem::replace(&mut st.at_roots, b.vec());
            let keyframes = st.used_keyframes.drain().collect();
            if !st.theme_vars.is_empty()
                || !st.keyframes.is_empty()
                || out.iter().any(|n| !matches!(n, Node::Rule { .. } | Node::AtRule { .. }))
            {
                st.theme_vars.clear();
                st.keyframes.clear();
                return None;
            }
            let is_empty = out.is_empty();
            for node in out {
                n.visit(node);
            }
            n.cursor = Cursor::None;
            let nested = std::mem::replace(&mut n.result, b.vec());
            let dedupe = n.dedupe.drain(..).collect();
            Some(Fragment { is_empty, nested, dedupe, keyframes, at_roots })
        })
        .collect()
}

fn color_mix_fallback<'a, 'th: 'a>(value: &'a str, theme: &Theme<'th>, alloc: &'a Allocator) -> Option<&'a str> {
    let mut ast = vp::parse_in(value, alloc);
    let mut requires = false;
    polyfill_nodes(&mut ast, theme, &mut requires, alloc);
    requires.then(|| vp::to_css(&ast, alloc))
}

fn polyfill_nodes<'a, 'th: 'a>(
    nodes: &mut orisu_allocator::Vec<'a, ValueNode<'a>>,
    theme: &Theme<'th>,
    requires: &mut bool,
    alloc: &'a Allocator,
) {
    let mut i = 0;
    while i < nodes.len() {
        let mut replacement = None;
        match &mut nodes[i] {
            ValueNode::Function(name, children) if *name == "color-mix" => {
                let mut unresolvable = false;
                let mut current_color = false;
                inline_vars(children, theme, requires, &mut unresolvable, &mut current_color, alloc);
                if unresolvable || current_color {
                    if let Some(sep) = children.iter().position(|n| matches!(n, ValueNode::Separator(v) if v.trim().contains(',')))
                        && let Some(first) = children.get(sep + 1)
                    {
                        replacement = Some(first.clone_in(alloc));
                    }
                } else if *requires
                    && let Some(ValueNode::Word(space)) = children.get_mut(2)
                    && matches!(*space, "oklab" | "oklch" | "lab" | "lch")
                {
                    *space = "srgb";
                }
                if replacement.is_none() {
                    polyfill_nodes(children, theme, requires, alloc);
                }
            }
            ValueNode::Function(_, children) => polyfill_nodes(children, theme, requires, alloc),
            _ => {}
        }
        match replacement {
            Some(node) => nodes[i] = node,
            None => i += 1,
        }
    }
}

fn inline_vars<'a, 'th: 'a>(
    nodes: &mut [ValueNode<'a>],
    theme: &Theme<'th>,
    requires: &mut bool,
    unresolvable: &mut bool,
    current_color: &mut bool,
    alloc: &'a Allocator,
) {
    for node in nodes.iter_mut() {
        if let ValueNode::Word(w) = node
            && w.eq_ignore_ascii_case("currentcolor")
        {
            *current_color = true;
            *requires = true;
            continue;
        }
        let mut replaced = None;
        if let ValueNode::Function(name, children) = node
            && *name == "var"
            && let Some(ValueNode::Word(first)) = children.first()
        {
            let mut seen: FxHashSet<&str> = FxHashSet::default();
            let mut var_name: &'a str = first;
            loop {
                if !seen.insert(var_name) {
                    *unresolvable = true;
                    break;
                }
                *requires = true;
                let Some(inlined) = theme.resolve_value(None, &[var_name]) else {
                    *unresolvable = true;
                    break;
                };
                if inlined.eq_ignore_ascii_case("currentcolor") {
                    *current_color = true;
                    break;
                }
                if inlined.starts_with("var(") {
                    match vp::parse_in(inlined, alloc).into_iter().next() {
                        Some(ValueNode::Function("var", c)) => match c.into_iter().next() {
                            Some(ValueNode::Word(w)) => var_name = w,
                            _ => break,
                        },
                        _ => break,
                    }
                } else {
                    replaced = Some(inlined);
                    break;
                }
            }
        }
        match replaced {
            Some(color) => *node = ValueNode::Word(color),
            None => {
                if let ValueNode::Function(_, children) = node {
                    inline_vars(children, theme, requires, unresolvable, current_color, alloc);
                }
            }
        }
    }
}

const HOISTABLE: &[&str] = &["@container", "@layer", "@media", "@page", "@starting-style", "@supports", "@view-transition"];
const DROPPABLE_IF_EMPTY: &[&str] = &["@container", "@media", "@page", "@starting-style", "@supports", "@view-transition"];

enum Cursor {
    None,
    At(Vec<usize>),
    Detached,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PKind {
    List,
    Complex,
    Compound,
    Function,
}

fn pkind(node: &SNode<'_>) -> Option<PKind> {
    match node {
        SNode::List(_) => Some(PKind::List),
        SNode::Complex(_) => Some(PKind::Complex),
        SNode::Compound(_) => Some(PKind::Compound),
        SNode::Function(..) => Some(PKind::Function),
        _ => None,
    }
}

struct NestCtx<'a, 'p> {
    last: &'a str,
    wrapped: &'a str,
    parent_ast: &'p [SNode<'a>],
}

impl<'a> NestCtx<'a, '_> {
    fn pa0_complex(&self) -> bool {
        matches!(self.parent_ast.first(), Some(SNode::Complex(_)))
    }

    fn pa0_universal_or_type(&self) -> bool {
        self.parent_ast.first().is_some_and(|n| n.is_universal() || n.is_type())
    }

    fn parent_is_list(&self) -> bool {
        self.parent_ast.len() == 1 && matches!(self.parent_ast[0], SNode::List(_))
    }

    fn replace(
        &self,
        nodes: &mut orisu_allocator::Vec<'a, SNode<'a>>,
        parent: Option<PKind>,
        grand: Option<(PKind, bool)>,
        changed: &mut bool,
    ) {
        let mut i = 0;
        while i < nodes.len() {
            if matches!(&nodes[i], SNode::Selector("&")) {
                *changed = true;
                let value = self.amp_value(nodes, i, parent, grand);
                nodes[i] = SNode::Selector(value);
            } else if let Some(kind) = pkind(&nodes[i]) {
                let grand_info = parent.map(|p| (p, i == 0));
                if let Some(children) = nodes[i].children_mut() {
                    self.replace(children, Some(kind), grand_info, changed);
                }
            }
            if i == 0 && nodes.len() > 1 && parent == Some(PKind::Compound) && nodes[0].is_universal() {
                let keep = matches!(&nodes[1], SNode::Selector(v) if v.starts_with('|'));
                if !keep {
                    nodes.remove(0);
                    continue;
                }
            }
            i += 1;
        }
    }

    fn amp_value(&self, siblings: &[SNode<'a>], index: usize, parent: Option<PKind>, grand: Option<(PKind, bool)>) -> &'a str {
        let Some(parent) = parent else {
            return self.wrapped;
        };
        if self.parent_is_list() {
            return self.wrapped;
        }
        match parent {
            PKind::Complex => {
                if index == 0 || !self.pa0_complex() {
                    self.last
                } else {
                    self.wrapped
                }
            }
            PKind::Compound => {
                if self.pa0_complex()
                    && let Some((PKind::Complex, parent_is_first)) = grand
                    && !parent_is_first
                {
                    return self.wrapped;
                }
                if siblings[index + 1..].iter().any(|s| s.is_universal() || s.is_type()) {
                    return self.wrapped;
                }
                if index == 0 {
                    return self.last;
                }
                if self.pa0_complex() || self.pa0_universal_or_type() {
                    return self.wrapped;
                }
                self.last
            }
            PKind::Function => self.last,
            PKind::List => self.wrapped,
        }
    }
}

struct Nester<'a, 'f> {
    b: Builder<'a>,
    selector_stack: Vec<&'a str>,
    at_rule_stack: Vec<(&'a str, &'a str)>,
    cursor: Cursor,
    seen_props: FxHashSet<&'a str>,
    dedupe: IndexSet<Vec<usize>, FxBuildHasher>,
    result: Nodes<'a>,
    parse_cache: FxHashMap<&'a str, orisu_allocator::Vec<'a, SNode<'a>>>,
    fragments: &'f [&'f Fragment<'a>],
}

impl<'a, 'f> Nester<'a, 'f> {
    fn new(b: Builder<'a>, at_rule_stack: Vec<(&'a str, &'a str)>, fragments: &'f [&'f Fragment<'a>], capacity: usize) -> Self {
        Nester {
            b,
            selector_stack: Vec::new(),
            at_rule_stack,
            cursor: Cursor::None,
            seen_props: FxHashSet::default(),
            dedupe: IndexSet::default(),
            result: orisu_allocator::Vec::with_capacity_in(capacity, b.alloc),
            parse_cache: FxHashMap::default(),
            fragments,
        }
    }

    fn splice(&mut self) {
        for fragment in self.fragments {
            self.merge(&fragment.nested, &fragment.dedupe);
        }
        self.cursor = Cursor::None;
    }

    /// Appends nodes nested on their own, joining the first of each level with the last node at that level like `emit` does.
    fn merge(&mut self, nodes: &[Node<'a>], dedupe: &[Vec<usize>]) {
        let alloc = self.b.alloc;
        let mut path = Vec::new();
        let mut levels: Vec<(usize, bool)> = Vec::new();
        let mut nodes = nodes;
        while let Some((first, rest)) = nodes.split_first() {
            let Some(target) = container_mut(&mut self.result, &path) else {
                return;
            };
            let len = target.len();
            match (target.last_mut(), first) {
                (Some(Node::AtRule { name: ln, params: lp, .. }), Node::AtRule { name, params, nodes: children })
                    if ln == name && lp == params && !children.is_empty() =>
                {
                    target.extend(rest.iter().map(|n| n.clone_in(alloc)));
                    levels.push((len - 1, true));
                    path.push(len - 1);
                    nodes = children;
                }
                (Some(Node::Rule { selector: ls, nodes: existing }), Node::Rule { selector, nodes: children }) if ls == selector => {
                    let before = existing.len();
                    existing.extend(children.iter().map(|n| n.clone_in(alloc)));
                    target.extend(rest.iter().map(|n| n.clone_in(alloc)));
                    levels.push((len - 1, true));
                    levels.push((before, false));
                    path.push(len - 1);
                    self.dedupe.insert(path);
                    break;
                }
                _ => {
                    target.extend(nodes.iter().map(|n| n.clone_in(alloc)));
                    levels.push((len, false));
                    break;
                }
            }
        }
        for fpath in dedupe {
            let mut on_chain = true;
            let mut rpath = Vec::with_capacity(fpath.len());
            for (depth, &i) in fpath.iter().enumerate() {
                match levels.get(depth) {
                    Some(&(base, merged)) if on_chain => {
                        rpath.push(base + i);
                        on_chain = merged && i == 0;
                    }
                    _ => {
                        on_chain = false;
                        rpath.push(i);
                    }
                }
            }
            self.dedupe.insert(rpath);
        }
    }

    fn nested_selector(&mut self, selector: &'a str, last: &'a str) -> &'a str {
        let alloc = self.b.alloc;
        let parent_ast = self.parse_cache.entry(last).or_insert_with(|| sp::parse_in(last, alloc));
        let last_is_list = parent_ast.len() == 1 && matches!(parent_ast[0], SNode::List(_));
        let wrapped: &'a str = orisu_allocator::afmt!(alloc, ":is({last})");
        let prefix = if last_is_list { wrapped } else { last };
        let nctx = NestCtx { last, wrapped, parent_ast };
        let mut out = ArenaString::with_capacity_in(selector.len() + last.len() * 2, alloc);
        for (n, part) in segment(selector, b',').into_iter().enumerate() {
            if n > 0 {
                out.push_str(", ");
            }
            if part.contains('&') {
                let mut ast = sp::parse_in(part, alloc);
                let mut changed = false;
                nctx.replace(&mut ast, None, None, &mut changed);
                if changed {
                    sp::write_css(&ast, &mut out, false);
                    continue;
                }
            }
            out.push_str(prefix);
            out.push(' ');
            out.push_str(part);
        }
        out.into_bump_str()
    }

    fn enter_rule(&mut self, selector: &'a str) -> bool {
        if self.selector_stack.is_empty() {
            if selector.contains('&') {
                let mut ast = sp::parse_in(selector, self.b.alloc);
                let mut changed = false;
                sp::walk_mut(&mut ast, &mut |n| {
                    if matches!(n, SNode::Selector("&")) {
                        changed = true;
                        *n = SNode::Selector(":scope");
                    }
                });
                self.selector_stack.push(if changed { sp::to_css(&ast, self.b.alloc) } else { selector });
            } else {
                self.selector_stack.push(selector);
            }
            return true;
        }
        if selector == "&" {
            return false;
        }
        let last = self.selector_stack.last().copied().unwrap_or_default();
        let nested = self.nested_selector(selector, last);
        self.selector_stack.push(nested);
        true
    }

    fn visit(&mut self, node: Node<'a>) {
        match node {
            Node::Rule { selector, nodes } => {
                self.cursor = Cursor::None;
                if !self.enter_rule(selector) {
                    for child in nodes {
                        self.visit(child);
                    }
                    return;
                }
                if nodes.iter().any(|c| matches!(c, Node::Decl { .. })) {
                    for child in nodes {
                        self.emit(child);
                    }
                } else {
                    for child in nodes {
                        self.visit(child);
                    }
                }
                self.cursor = Cursor::None;
                self.selector_stack.pop();
            }
            Node::AtRule { name, params, nodes } => {
                self.cursor = Cursor::None;
                if (nodes.is_empty() && !DROPPABLE_IF_EMPTY.contains(&name)) || !HOISTABLE.contains(&name) {
                    self.emit(Node::AtRule { name, params, nodes });
                    return;
                }
                self.at_rule_stack.push((name, params));
                for child in nodes {
                    self.visit(child);
                }
                self.cursor = Cursor::None;
                self.at_rule_stack.pop();
            }
            Node::Comment(c) if std::ptr::eq(c, FRAGMENTS) => self.splice(),
            node @ (Node::Decl { .. } | Node::Comment(_)) => self.emit(node),
            Node::Context { nodes, .. } | Node::AtRoot(nodes) => {
                for child in nodes {
                    self.visit(child);
                }
            }
        }
    }

    fn emit(&mut self, node: Node<'a>) {
        match &self.cursor {
            Cursor::At(path) => {
                if let Node::Decl { property, .. } = &node
                    && !self.seen_props.insert(property)
                {
                    self.dedupe.insert(path.clone());
                }
                if let Some(c) = container_mut(&mut self.result, path) {
                    c.push(node);
                }
                return;
            }
            Cursor::Detached => {
                if let Node::Decl { property, .. } = &node {
                    self.seen_props.insert(property);
                }
                return;
            }
            Cursor::None => {}
        }

        if self.selector_stack.is_empty() && self.at_rule_stack.is_empty() {
            if let (Some(Node::AtRule { name: ln, params: lp, nodes: lc }), Node::AtRule { name, params, nodes }) =
                (self.result.last(), &node)
                && lc.is_empty()
                && nodes.is_empty()
                && ln == name
                && lp == params
            {
                return;
            }
            self.result.push(node);
            return;
        }

        self.seen_props.clear();
        if let Node::Decl { property, .. } = &node {
            self.seen_props.insert(property);
        }

        let mut target_path: Vec<usize> = Vec::new();
        let mut offset = 0;
        {
            let mut target: &Nodes<'a> = &self.result;
            if matches!(target.last(), Some(Node::AtRule { .. })) {
                for (name, params) in &self.at_rule_stack {
                    let Some(Node::AtRule { name: ln, params: lp, nodes: children }) = target.last() else {
                        break;
                    };
                    if ln != name || lp != params {
                        break;
                    }
                    offset += 1;
                    target_path.push(target.len() - 1);
                    target = children;
                }
            }
        }

        let k = self.at_rule_stack.len() - offset;
        let mut built: Option<Node<'a>> = None;
        let mut nodes = Some(self.b.vec1(node));
        if let Some(&selector) = self.selector_stack.last() {
            if k == 0
                && let Some(target) = container_mut(&mut self.result, &target_path)
                && let Some(Node::Rule { selector: ls, nodes: children }) = target.last_mut()
                && *ls == selector
            {
                children.extend(nodes.take().into_iter().flatten());
                let mut path = target_path;
                path.push(target.len() - 1);
                self.dedupe.insert(path.clone());
                self.cursor = Cursor::At(path);
                return;
            }
            built = Some(Node::Rule { selector, nodes: nodes.take().unwrap_or_else(|| self.b.vec()) });
        }
        for i in (offset..self.at_rule_stack.len()).rev() {
            let (name, params) = self.at_rule_stack[i];
            let children = match built.take() {
                Some(r) => self.b.vec1(r),
                None => nodes.take().unwrap_or_else(|| self.b.vec()),
            };
            built = Some(Node::AtRule { name, params, nodes: children });
        }
        let Some(target) = container_mut(&mut self.result, &target_path) else {
            return;
        };
        match built {
            Some(root) => {
                target.push(root);
                let mut path = target_path;
                path.push(target.len() - 1);
                let depth = if self.selector_stack.is_empty() { k.saturating_sub(1) } else { k };
                path.extend(std::iter::repeat_n(0, depth));
                self.cursor = Cursor::At(path);
            }
            None => {
                target.extend(nodes.take().into_iter().flatten());
                self.cursor = Cursor::Detached;
            }
        }
    }
}

fn nest<'a>(
    ast: Nodes<'a>,
    at_rule_stack: Vec<(&'a str, &'a str)>,
    fragments: &[&Fragment<'a>],
    alloc: &'a Allocator,
) -> (Nodes<'a>, Vec<Vec<usize>>) {
    let mut n = Nester::new(Builder::new(alloc), at_rule_stack, fragments, ast.len());
    for node in ast {
        n.visit(node);
    }
    (n.result, n.dedupe.into_iter().collect())
}

fn dedupe_decls(ast: &mut Nodes<'_>, mut paths: Vec<Vec<usize>>) {
    paths.sort_by(|a, b| b.cmp(a));
    for path in paths {
        let Some(nodes) = container_mut(ast, &path) else {
            continue;
        };
        let mut seen = FxHashSet::default();
        let mut i = nodes.len();
        while i > 0 {
            i -= 1;
            if let Node::Decl { property, value, important } = &nodes[i]
                && !seen.insert((*property, *value, *important))
            {
                nodes.remove(i);
            }
        }
    }
}
