// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::attributify::Attributify;
use crate::compile::{Entry, compile_candidates, compile_entries, insert_sorted};
use crate::css_functions::substitute_functions;
use crate::design_system::{Caches, DesignSystem, ExtensionRef, Session};
use indexmap::{IndexMap, IndexSet};
use orisu_allocator::{AllocExt, Allocator, Vec};
use orisu_ast::{Ctx, Node, Nodes, Scope, Walk, to_css, walk, walk_scoped};
use orisu_candidate::CandidateKind;
use orisu_diagnostics::{Code, Report, Span};
use orisu_optimizer::{Splice, SpliceContext, SpliceNodes, fragments, optimize_spliced, polyfills, splice_context};
use orisu_parser::css as css_parser;
use orisu_parser::escape::{escape, unescape};
use orisu_parser::misc::{expand_braces, topological_sort};
use orisu_parser::segment::segment;
use orisu_parser::value::{self as vp, ValueNode};
use orisu_registry::utilities::{CssUtility, is_valid_functional_utility_name, is_valid_static_utility_name};
use orisu_registry::variants::{Compounds, body_compounds, register_custom_body, register_custom_selectors, substitute_at_variant};
use orisu_theme::{Theme, UsedVars, opts};
use rustc_hash::FxBuildHasher;

pub const VERSION: &str = "4.3.3";

pub mod features {
    pub const NONE: u8 = 0;
    pub const AT_APPLY: u8 = 1 << 0;
    pub const AT_IMPORT: u8 = 1 << 1;
    pub const THEME_FUNCTION: u8 = 1 << 3;
    pub const UTILITIES: u8 = 1 << 4;
    pub const VARIANTS: u8 = 1 << 5;
    pub const AT_THEME: u8 = 1 << 6;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source<'a> {
    pub base: &'a str,
    pub pattern: &'a str,
    pub negated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRoot<'a> {
    /// `source(none)`
    Disabled,
    Path {
        base: &'a str,
        pattern: &'a str,
    },
}

pub struct Stylesheet {
    pub path: String,
    pub base: String,
    pub content: String,
}

pub type LoadStylesheet<'l> = dyn FnMut(&str, &str) -> Result<Stylesheet, String> + 'l;

pub struct Options<'l, 'a> {
    pub base: &'l str,
    pub polyfills: u8,
    pub load_stylesheet: Option<Box<LoadStylesheet<'l>>>,
    pub extension: Option<ExtensionRef<'a>>,
}

impl Default for Options<'_, '_> {
    fn default() -> Self {
        Self { base: "", polyfills: polyfills::ALL, load_stylesheet: None, extension: None }
    }
}

pub struct Compiler<'a> {
    pub design_system: DesignSystem<'a>,
    caches: Caches<'a>,
    pub sources: std::vec::Vec<Source<'a>>,
    pub root: Option<SourceRoot<'a>>,
    pub features: u8,
    alloc: &'a Allocator,
    input: &'a str,
    ast: Nodes<'a>,
    utilities_path: Option<std::vec::Vec<usize>>,
    /// Set when entries can be optimized one by one. Their fragments are built from the second render on, as a single
    /// build gains nothing from them.
    splice_context: Option<SpliceContext<'a>>,
    utility_entries: std::vec::Vec<Entry<'a>>,
    scratch: Allocator,
    polyfills: u8,
    valid_candidates: IndexSet<&'a str, FxBuildHasher>,
    used_variables: UsedVars,
    default_did_change: bool,
    /// How many of `valid_candidates` were compiled into `utility_entries`.
    compiled_candidates: usize,
    css: Option<String>,
    renders: u32,
    delivered: Option<u32>,
}

pub fn compile<'a>(css: &'a str, alloc: &'a Allocator, mut options: Options<'_, 'a>) -> Result<Compiler<'a>, String> {
    let ast = css_parser::parse(css, alloc).map_err(|e| e.to_string())?;
    let parsed = parse_css(ast, alloc, &mut options)?;
    let Parsed { design_system, caches, mut ast, sources, root, mut utilities_path, features, inline_candidates } = parsed;
    if let Some(first) = utilities_path.as_mut().and_then(|p| p.first_mut()) {
        *first += 1;
    }
    if let Some(banner) = design_system.extension().and_then(|e| e.banner()) {
        ast.insert(0, Node::Comment(banner));
    }
    let splice_context = utilities_path.as_deref().and_then(|path| splice_context(&ast, path));
    let mut valid_candidates = IndexSet::default();
    let mut default_did_change = false;
    for candidate in inline_candidates {
        if !design_system.invalid_candidates.contains(candidate) {
            valid_candidates.insert(candidate);
            default_did_change = true;
        }
    }
    Ok(Compiler {
        design_system,
        caches,
        sources,
        root,
        features,
        alloc,
        input: css,
        ast,
        utilities_path,
        splice_context,
        utility_entries: std::vec::Vec::new(),
        scratch: Allocator::new(),
        polyfills: options.polyfills,
        valid_candidates,
        used_variables: UsedVars::default(),
        default_did_change,
        compiled_candidates: 0,
        css: None,
        renders: 0,
        delivered: None,
    })
}

impl<'a> Compiler<'a> {
    pub fn alloc(&self) -> &'a Allocator {
        self.alloc
    }

    pub fn session(&mut self) -> Session<'_, 'a> {
        Session::new(&self.design_system, self.alloc, &mut self.caches)
    }

    pub fn build<'c>(&mut self, candidates: impl IntoIterator<Item = &'c str>) -> &str {
        if self.features == features::NONE {
            return self.input;
        }
        if self.utilities_path.is_none() {
            return self.render();
        }

        let mut did_change = std::mem::take(&mut self.default_did_change);
        let mut did_add_external_variable = false;
        let ds = &self.design_system;
        let previous_size = self.valid_candidates.len();
        for candidate in candidates {
            if ds.invalid_candidates.contains(candidate) {
                continue;
            }
            if candidate.starts_with("--") {
                let marked = ds.theme.mark_used_variable(candidate, &mut self.used_variables);
                did_change |= marked;
                did_add_external_variable |= marked;
            } else if !self.valid_candidates.contains(candidate) {
                self.valid_candidates.insert(self.alloc.str(candidate));
            }
        }
        did_change |= self.valid_candidates.len() != previous_size;
        if !did_change {
            return self.render();
        }

        let mut invalid = std::vec::Vec::new();
        let fresh = self.valid_candidates.get_range(self.compiled_candidates..).into_iter().flatten().copied();
        let alloc = self.alloc;
        let mut session = Session::new(ds, alloc, &mut self.caches);
        let entries = compile_entries(fresh, &mut session, true, |c| invalid.push(alloc.str(c)));
        self.compiled_candidates = self.valid_candidates.len();
        self.design_system.invalid_candidates.extend(invalid);
        if !did_add_external_variable && entries.is_empty() {
            return self.render();
        }
        insert_sorted(&mut self.utility_entries, entries, &self.design_system);
        if let Some(cx) = &self.splice_context
            && self.renders > 0
        {
            let pending: std::vec::Vec<usize> =
                (0..self.utility_entries.len()).filter(|&i| self.utility_entries[i].fragment.is_none()).collect();
            let nodes = pending.iter().map(|&i| self.utility_entries[i].node_ref(self.alloc));
            let built = fragments(nodes, cx, &self.design_system.theme, &mut self.used_variables, self.polyfills, self.alloc);
            for (i, fragment) in pending.into_iter().zip(built) {
                self.utility_entries[i].fragment = fragment;
            }
        }
        self.css = None;
        self.render()
    }

    /// Like [`Compiler::build`], but `None` when the output is still the one the previous call returned.
    pub fn build_changed<'c>(&mut self, candidates: impl IntoIterator<Item = &'c str>) -> Option<&str> {
        self.build(candidates);
        if self.delivered.replace(self.renders) == Some(self.renders) {
            return None;
        }
        Some(self.build([]))
    }

    fn render(&mut self) -> &str {
        if self.css.is_none() {
            self.renders += 1;
            self.scratch.reset();
            let scratch = &self.scratch;
            let fragments: Option<std::vec::Vec<_>> = self.utility_entries.iter().map(|entry| entry.fragment.as_ref()).collect();
            let utilities: std::vec::Vec<_> = match fragments {
                Some(_) => std::vec::Vec::new(),
                None => self.utility_entries.iter().map(|entry| entry.node_ref(scratch)).collect(),
            };
            let nodes = match &fragments {
                Some(fragments) => SpliceNodes::Fragments(fragments),
                None => SpliceNodes::Nodes(&utilities),
            };
            let splice = self.utilities_path.as_deref().and_then(|path| node_at(&self.ast, path)).map(|at| Splice { at, nodes });
            let ast = optimize_spliced(&self.ast, splice, &self.design_system.theme, &mut self.used_variables, self.polyfills, scratch);
            self.css = Some(to_css(&ast));
        }
        self.css.as_deref().unwrap_or_default()
    }
}

fn node_at<'n, 'a>(ast: &'n [Node<'a>], path: &[usize]) -> Option<&'n Node<'a>> {
    let (&first, rest) = path.split_first()?;
    let mut node = ast.get(first)?;
    for &i in rest {
        node = node.children()?.get(i)?;
    }
    Some(node)
}

fn node_at_mut<'n, 'a>(ast: &'n mut Nodes<'a>, path: &[usize]) -> Option<&'n mut Node<'a>> {
    let (&first, rest) = path.split_first()?;
    let mut node = ast.get_mut(first)?;
    for &i in rest {
        node = node.children_mut()?.get_mut(i)?;
    }
    Some(node)
}

fn find_path(ast: &[Node<'_>], pred: &impl Fn(&Node<'_>) -> bool, path: &mut std::vec::Vec<usize>) -> bool {
    for (i, node) in ast.iter().enumerate() {
        path.push(i);
        if pred(node) || node.children().is_some_and(|c| find_path(c, pred, path)) {
            return true;
        }
        path.pop();
    }
    false
}

struct CustomUtility<'a> {
    params: &'a str,
    name: &'a str,
    index: usize,
}

enum CustomVariant<'a> {
    Selectors { style_rules: std::vec::Vec<&'a str>, at_rules: std::vec::Vec<&'a str> },
    Body(Nodes<'a>),
}

struct Parsed<'a> {
    design_system: DesignSystem<'a>,
    caches: Caches<'a>,
    ast: Nodes<'a>,
    sources: std::vec::Vec<Source<'a>>,
    root: Option<SourceRoot<'a>>,
    utilities_path: Option<std::vec::Vec<usize>>,
    features: u8,
    inline_candidates: std::vec::Vec<&'a str>,
}

fn parse_theme_options(params: &str) -> (u8, Option<&str>) {
    let mut options = opts::NONE;
    let mut prefix = None;
    for option in segment(params, b' ') {
        match option {
            "reference" => options |= opts::REFERENCE,
            "inline" => options |= opts::INLINE,
            "default" => options |= opts::DEFAULT,
            "static" => options |= opts::STATIC,
            _ if option.starts_with("prefix(") && option.ends_with(')') => prefix = Some(&option[7..option.len() - 1]),
            _ => {}
        }
    }
    (options, prefix)
}

#[inline]
fn is_valid_prefix(prefix: &str) -> bool {
    !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_lowercase())
}

fn is_valid_variant_name(name: &str) -> bool {
    let name = name.strip_prefix('@').unwrap_or(name);
    let bytes = name.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && rest.iter().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        && !matches!(bytes.last(), Some(b'_' | b'-'))
}

#[inline]
fn is_quoted_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    match bytes.first() {
        Some(q @ (b'"' | b'\'')) => bytes.last() == Some(q),
        _ => false,
    }
}

#[inline]
fn unquote(path: &str) -> &str {
    path.get(1..path.len().saturating_sub(1)).unwrap_or("")
}

#[inline]
fn function_arg(param: &str, prefix_len: usize) -> &str {
    param.get(prefix_len..param.len().saturating_sub(1)).unwrap_or("")
}

fn theme_snippet(name: &str, params: &str, child: &Node<'_>, alloc: &Allocator) -> String {
    let wrapped = Node::AtRule { name, params, nodes: Vec::from_iter_in([child.clone_in(alloc)], alloc) };
    let css = to_css(&[wrapped]);
    let lines: std::vec::Vec<&str> = css.split('\n').collect();
    let len = lines.len();
    lines
        .iter()
        .enumerate()
        .map(|(idx, line)| format!("{} {line}", if idx == 0 || idx + 2 >= len { ' ' } else { '>' }))
        .collect::<std::vec::Vec<_>>()
        .join("\n")
}

fn parse_css<'a>(ast: Nodes<'a>, alloc: &'a Allocator, options: &mut Options<'_, 'a>) -> Result<Parsed<'a>, String> {
    let base = alloc.str(options.base);
    let mut ast = Vec::from_iter_in([Node::Context { ctx: Ctx { base: Some(base), ..Ctx::default() }, nodes: ast }], alloc);
    let extension = options.extension;
    let mut features = substitute_at_imports(&mut ast, base, &mut options.load_stylesheet, extension, 0, alloc)?;

    let mut important = false;
    let mut attributify = None;
    let mut theme = Theme::new();
    let mut custom_variants: IndexMap<&'a str, CustomVariant<'a>> = IndexMap::new();
    let mut custom_variant_dependencies: IndexMap<&'a str, IndexSet<&'a str>> = IndexMap::new();
    let mut first_theme_rule: Option<&'a str> = None;
    let mut utilities_params: Option<*const u8> = None;
    let mut sources = std::vec::Vec::new();
    let mut inline_candidates = std::vec::Vec::new();
    let mut ignored_candidates = std::vec::Vec::new();
    let mut root = None;

    walk_scoped(&mut ast, Scope::default(), &mut |node, scope| {
        let Node::AtRule { name, params, nodes } = node else {
            return Ok(Walk::Continue);
        };
        let ctx = scope.ctx;

        if *name == "@tailwind" && params.starts_with("utilities") {
            if utilities_params.is_some() || ctx.is_reference() {
                return Ok(Walk::Replace(Vec::new_in(alloc)));
            }
            for param in segment(params, b' ') {
                if !param.starts_with("source(") {
                    continue;
                }
                let path = function_arg(param, 7);
                if path == "none" {
                    root = Some(SourceRoot::Disabled);
                    continue;
                }
                if !is_quoted_path(path) {
                    return Err("`source(…)` paths must be quoted.".to_owned());
                }
                root = Some(SourceRoot::Path { base: ctx.source_base.or(ctx.base).unwrap_or(""), pattern: unquote(path) });
            }
            utilities_params = Some(params.as_ptr());
            features |= features::UTILITIES;
        }

        if *name == "@shortcut" {
            if !nodes.is_empty() {
                return Err("`@shortcut` cannot have a body.".to_owned());
            }
            let (utility, classes) = params.split_once(char::is_whitespace).unwrap_or((params, ""));
            let classes = classes.trim();
            let classes = if is_quoted_path(classes) { unquote(classes) } else { classes };
            if classes.is_empty() {
                return Err(format!("`@shortcut {params}` is empty. Shortcuts should list at least one class."));
            }
            *name = "@utility";
            *params = utility;
            nodes.push(Node::AtRule { name: "@apply", params: classes, nodes: Vec::new_in(alloc) });
        }

        if *name == "@apply"
            && let Some(expanded) = orisu_candidate::group::expand_str(params)
        {
            *params = alloc.str(&expanded);
        }

        if *name == "@utility" {
            if scope.nested {
                return Err("`@utility` cannot be nested.".to_owned());
            }
            if nodes.is_empty() {
                return Err(format!("`@utility {params}` is empty. Utilities should include at least one property."));
            }
            let unescaped = unescape(params);
            if !is_valid_functional_utility_name(&unescaped) && !is_valid_static_utility_name(&unescaped) {
                if !params.ends_with("-*") {
                    if params.ends_with('*') {
                        return Err(format!("`@utility {params}` defines an invalid utility name. A functional utility must end in `-*`."));
                    } else if params.contains('*') {
                        return Err(format!(
                            "`@utility {params}` defines an invalid utility name. The dynamic portion marked by `-*` must appear once at the end."
                        ));
                    }
                }
                return Err(format!(
                    "`@utility {params}` defines an invalid utility name. Utilities should be alphanumeric and start with a lowercase letter."
                ));
            }
        }

        if *name == "@attributify" {
            if !nodes.is_empty() {
                return Err("`@attributify` cannot have a body.".to_owned());
            }
            if scope.nested {
                return Err("`@attributify` cannot be nested.".to_owned());
            }
            attributify = Some(Attributify::parse(params)?);
            return Ok(Walk::ReplaceSkip(Vec::new_in(alloc)));
        }

        if *name == "@source" {
            if !nodes.is_empty() {
                return Err("`@source` cannot have a body.".to_owned());
            }
            if scope.nested {
                return Err("`@source` cannot be nested.".to_owned());
            }
            let mut path: &'a str = params;
            let negated = path.starts_with("not ");
            if negated {
                path = &path[4..];
            }
            let inline = path.starts_with("inline(");
            if inline {
                path = function_arg(path, 7).trim();
            }
            if !is_quoted_path(path) {
                return Err("`@source` paths must be quoted.".to_owned());
            }
            let source = unquote(path);
            if inline {
                let destination = if negated { &mut ignored_candidates } else { &mut inline_candidates };
                for pattern in segment(source, b' ') {
                    for candidate in expand_braces(pattern)? {
                        destination.push(alloc.str(&candidate));
                    }
                }
            } else {
                sources.push(Source { base: ctx.base.unwrap_or(""), pattern: source, negated });
            }
            return Ok(Walk::ReplaceSkip(Vec::new_in(alloc)));
        }

        if *name == "@variant"
            && !scope.nested
            && (nodes.is_empty() || {
                let mut has_slot = false;
                walk(nodes, &mut |child| {
                    has_slot |= child.is_at_rule("@slot");
                    !has_slot
                });
                has_slot
            })
        {
            *name = "@custom-variant";
        }

        if *name == "@custom-variant" {
            if scope.nested {
                return Err("`@custom-variant` cannot be nested.".to_owned());
            }
            let parts = segment(params, b' ');
            let variant = parts[0];
            let selector = parts.get(1).copied().filter(|s| !s.is_empty());
            if !is_valid_variant_name(variant) {
                return Err(format!(
                    "`@custom-variant {variant}` defines an invalid variant name. Variants should only contain alphanumeric, dashes, or underscore characters and start with a lowercase letter or number."
                ));
            }
            if !nodes.is_empty() && selector.is_some() {
                return Err(format!("`@custom-variant {variant}` cannot have both a selector and a body."));
            }
            if nodes.is_empty() {
                let Some(selector) = selector else {
                    return Err(format!("`@custom-variant {variant}` has no selector or body."));
                };
                let selectors = segment(function_arg(selector, 1), b',');
                if selectors.iter().any(|s| s.trim().is_empty()) {
                    return Err(format!(
                        "`@custom-variant {variant} ({})` selector is invalid.",
                        selectors.iter().copied().collect::<std::vec::Vec<_>>().join(",")
                    ));
                }
                let (mut style_rules, mut at_rules) = (std::vec::Vec::new(), std::vec::Vec::new());
                for selector in selectors {
                    let selector = selector.trim();
                    if selector.starts_with('@') {
                        at_rules.push(selector);
                    } else {
                        style_rules.push(selector);
                    }
                }
                custom_variants.insert(variant, CustomVariant::Selectors { style_rules, at_rules });
                custom_variant_dependencies.insert(variant, IndexSet::new());
            } else {
                let mut dependencies = IndexSet::new();
                walk(nodes, &mut |child| {
                    if let Node::AtRule { name: "@variant", params, .. } = child {
                        dependencies.insert(*params);
                    }
                    true
                });
                custom_variants.insert(variant, CustomVariant::Body(std::mem::replace(nodes, Vec::new_in(alloc))));
                custom_variant_dependencies.insert(variant, dependencies);
            }
            return Ok(Walk::ReplaceSkip(Vec::new_in(alloc)));
        }

        if *name == "@media" {
            let params_list = segment(params, b' ');
            let mut unknown: std::vec::Vec<&str> = std::vec::Vec::new();
            for param in params_list.iter().copied() {
                if param.starts_with("source(") {
                    let path = function_arg(param, 7);
                    walk_scoped(nodes, Scope::default(), &mut |child, _| {
                        Ok::<_, String>(match child {
                            Node::AtRule { name: "@tailwind", params: "utilities", .. } => {
                                let mut child = std::mem::replace(child, Node::Comment(""));
                                if let Node::AtRule { params, .. } = &mut child {
                                    *params = alloc.concat(&["utilities source(", path, ")"]);
                                }
                                let ctx = Ctx { source_base: ctx.base, ..Ctx::default() };
                                Walk::ReplaceStop(Vec::from_iter_in(
                                    [Node::Context { ctx, nodes: Vec::from_iter_in([child], alloc) }],
                                    alloc,
                                ))
                            }
                            _ => Walk::Continue,
                        })
                    })?;
                } else if param.starts_with("theme(") {
                    let theme_params = function_arg(param, 6);
                    let has_reference = theme_params.contains("reference");
                    walk_scoped(nodes, Scope::default(), &mut |child, _| {
                        match child {
                        Node::Context { .. } => Ok(Walk::Continue),
                        Node::AtRule { name: "@theme", params, .. } => {
                            *params = alloc.concat(&[params, " ", theme_params]);
                            Ok(Walk::Skip)
                        }
                        Node::AtRule { .. } => Ok(Walk::Continue),
                        _ if has_reference => Err(
                            "Files imported with `@import \"…\" theme(reference)` must only contain `@theme` blocks.\nUse `@reference \"…\";` instead."
                                .to_owned(),
                        ),
                        _ => Ok(Walk::Continue),
                    }
                    })?;
                } else if param.starts_with("prefix(") {
                    let prefix = function_arg(param, 7);
                    walk_scoped(nodes, Scope::default(), &mut |child, _| {
                        Ok::<_, String>(match child {
                            Node::AtRule { name: "@theme", params, .. } => {
                                *params = alloc.concat(&[params, " prefix(", prefix, ")"]);
                                Walk::Skip
                            }
                            _ => Walk::Continue,
                        })
                    })?;
                } else if param == "important" {
                    important = true;
                } else if param == "reference" {
                    let inner = std::mem::replace(nodes, Vec::new_in(alloc));
                    let ctx = Ctx { reference: Some(true), ..Ctx::default() };
                    nodes.push(Node::Context { ctx, nodes: inner });
                } else {
                    unknown.push(param);
                }
            }
            if !unknown.is_empty() {
                *params = alloc.str(&unknown.join(" "));
            } else if !params_list.is_empty() {
                return Ok(Walk::Replace(std::mem::replace(nodes, Vec::new_in(alloc))));
            }
            return Ok(Walk::Continue);
        }

        if *name == "@theme" {
            let (mut theme_options, theme_prefix) = parse_theme_options(params);
            features |= features::AT_THEME;
            if ctx.is_reference() {
                theme_options |= opts::REFERENCE;
            }
            if let Some(prefix) = theme_prefix.filter(|p| !p.is_empty()) {
                if !is_valid_prefix(prefix) {
                    return Err(format!("The prefix \"{prefix}\" is invalid. Prefixes must be lowercase ASCII letters (a-z) only."));
                }
                theme.set_prefix(alloc, Some(prefix));
            }
            let (theme_name, theme_params) = (*name, *params);
            walk_scoped(nodes, Scope::default(), &mut |child, _| match child {
                Node::AtRule { name: "@keyframes", .. } => {
                    theme.add_keyframes(child.clone_in(alloc));
                    Ok(Walk::Skip)
                }
                Node::Comment(_) => Ok(Walk::Continue),
                Node::Decl { property, value, .. } if property.starts_with("--") => {
                    theme.add(alloc, alloc.str(&unescape(property)), value.unwrap_or(""), theme_options)?;
                    Ok(Walk::Continue)
                }
                _ => Err(format!(
                    "`@theme` blocks must only contain custom properties or `@keyframes`.\n\n{}",
                    theme_snippet(theme_name, theme_params, child, alloc)
                )),
            })?;
            if first_theme_rule.is_none() {
                let selector = alloc.str(":root, :host");
                first_theme_rule = Some(selector);
                return Ok(Walk::ReplaceSkip(Vec::from_iter_in([Node::Rule { selector, nodes: Vec::new_in(alloc) }], alloc)));
            }
            return Ok(Walk::ReplaceSkip(Vec::new_in(alloc)));
        }

        Ok(Walk::Continue)
    })?;

    let mut css_utilities = std::vec::Vec::new();
    walk_scoped(&mut ast, Scope::default(), &mut |node, _| {
        Ok::<_, String>(match node {
            Node::AtRule { name: "@utility", params, nodes } => {
                if let Some(utility) = CssUtility::new(params, std::mem::replace(nodes, Vec::new_in(alloc)), alloc) {
                    css_utilities.push((*params, utility));
                }
                Walk::Skip
            }
            _ => Walk::Continue,
        })
    })?;

    let mut ds = DesignSystem::new(alloc, theme, extension);
    let mut caches = Caches::default();
    ds.important |= important;
    ds.attributify = attributify;
    ds.invalid_candidates.extend(ignored_candidates);

    reject_modules(&ast)?;

    for &name in custom_variants.keys() {
        ds.variants.static_variant(name, Compounds::STYLE_RULES, |_, _, _| Some(()));
    }
    let mut cycle = None;
    let order = topological_sort(&custom_variant_dependencies, |path, start| {
        cycle.get_or_insert_with(|| circular_variants_error(path, start, alloc));
    });
    if let Some(err) = cycle {
        return Err(err);
    }
    for name in order {
        match custom_variants.swap_remove(name) {
            Some(CustomVariant::Selectors { style_rules, at_rules }) => {
                register_custom_selectors(&mut ds.variants, alloc, name, &style_rules, &at_rules);
            }
            Some(CustomVariant::Body(mut body)) => {
                let (compounds, uses_at_variant) = body_compounds(&body);
                let mut session = Session::new(&ds, alloc, &mut caches);
                if uses_at_variant && substitute_at_variant(&mut body, &session.cx(), &mut session, &ds.variants).is_err() {
                    ds.variants.static_variant(name, compounds, |_, _, _| None);
                } else {
                    register_custom_body(&mut ds.variants, name, body, compounds);
                }
            }
            None => {}
        }
    }

    let utilities: std::vec::Vec<CustomUtility<'a>> = css_utilities
        .into_iter()
        .map(|(params, utility)| {
            let name = utility.name;
            CustomUtility { params, name, index: utility.register(&mut ds.utilities, alloc) }
        })
        .collect();

    if let Some(selector) = first_theme_rule {
        let mut decls = Vec::new_in(alloc);
        for (key, value) in ds.theme.entries(alloc) {
            if value.options & opts::REFERENCE != 0 {
                continue;
            }
            decls.push(Node::Decl { property: alloc.str(&escape(key)), value: Some(value.value), important: false });
        }
        let theme_ctx = Ctx { theme: Some(true), ..Ctx::default() };
        for keyframes in ds.theme.keyframes() {
            let at_root = Node::AtRoot(Vec::from_iter_in([keyframes.clone_in(alloc)], alloc));
            ast.push(Node::Context { ctx: theme_ctx, nodes: Vec::from_iter_in([at_root], alloc) });
        }
        let mut path = std::vec::Vec::new();
        let is_theme_rule = |n: &Node<'_>| matches!(n, Node::Rule { selector: s, .. } if std::ptr::eq(s.as_ptr(), selector.as_ptr()));
        if find_path(&ast, &is_theme_rule, &mut path)
            && let Some(Node::Rule { nodes, .. }) = node_at_mut(&mut ast, &path)
        {
            nodes.push(Node::Context { ctx: theme_ctx, nodes: decls });
        }
    }

    let mut session = Session::new(&ds, alloc, &mut caches);
    if substitute_at_variant(&mut ast, &session.cx(), &mut session, &ds.variants)? {
        features |= features::VARIANTS;
    }
    for utility in &utilities {
        if with_body(&mut ds, &mut caches, alloc, utility, |body, ds| substitute_at_variant(body, &ds.cx(), ds, &ds.ds.variants))? {
            features |= features::VARIANTS;
        }
    }
    if substitute_functions(&mut ast, &Session::new(&ds, alloc, &mut caches))? {
        features |= features::THEME_FUNCTION;
    }
    for utility in &utilities {
        if with_body(&mut ds, &mut caches, alloc, utility, |body, ds| substitute_functions(body, ds))? {
            features |= features::THEME_FUNCTION;
        }
    }
    if substitute_at_apply(&mut ast, &utilities, &mut ds, &mut caches, alloc)? {
        features |= features::AT_APPLY;
    }

    walk_scoped(&mut ast, Scope::default(), &mut |node, _| {
        Ok::<_, String>(match node {
            Node::AtRule { name: "@utility", .. } => Walk::Replace(Vec::new_in(alloc)),
            Node::AtRule { .. } => Walk::Skip,
            _ => Walk::Continue,
        })
    })?;

    let utilities_path = utilities_params.and_then(|ptr| {
        let mut path = std::vec::Vec::new();
        let is_marker = |n: &Node<'_>| matches!(n, Node::AtRule { name: "@tailwind", params, .. } if params.as_ptr() == ptr);
        find_path(&ast, &is_marker, &mut path).then_some(path)
    });
    if let Some(path) = &utilities_path
        && let Some(node) = node_at_mut(&mut ast, path)
    {
        *node = Node::Context { ctx: Ctx::default(), nodes: Vec::new_in(alloc) };
    }

    Ok(Parsed { design_system: ds, caches, ast, sources, root, utilities_path, features, inline_candidates })
}

/// Runs `f` on the body of `utility`, taken out of the registry so `f` can read the rest of `ds`.
fn with_body<'a, R>(
    ds: &mut DesignSystem<'a>,
    caches: &mut Caches<'a>,
    alloc: &'a Allocator,
    utility: &CustomUtility<'a>,
    f: impl FnOnce(&mut Nodes<'a>, &mut Session<'_, 'a>) -> Result<R, String>,
) -> Result<R, String> {
    let Some(mut body) = ds.utilities.replace_body(utility.name, utility.index, Vec::new_in(alloc), alloc) else {
        return f(&mut Vec::new_in(alloc), &mut Session::new(ds, alloc, caches));
    };
    let result = f(&mut body, &mut Session::new(ds, alloc, caches));
    ds.utilities.replace_body(utility.name, utility.index, body, alloc);
    result
}

fn reject_modules(ast: &Nodes<'_>) -> Result<(), String> {
    let mut found = None;
    walk(ast, &mut |node| {
        if let Node::AtRule { name: name @ ("@plugin" | "@config"), params, .. } = node
            && found.is_none()
        {
            found = Some((&name[1..], params.get(1..params.len().saturating_sub(1)).unwrap_or("")));
        }
        found.is_none()
    });
    let Some((directive, id)) = found else {
        return Ok(());
    };
    let hint = match id {
        "@tailwindcss/typography" => " Import `orisu/presets/tailwind-typography.css` instead.",
        "@tailwindcss/forms" => " Import `orisu/presets/tailwind-forms.css` instead.",
        _ => "",
    };
    Err(Report::new(Code::UnsupportedPlugin, Span::new(0, 0)).arg("directive", directive).arg("id", id).arg("hint", hint).message())
}

fn circular_variants_error(path: &[&str], start: &str, alloc: &Allocator) -> String {
    let nodes: std::vec::Vec<Node<'_>> = path
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            let dependency =
                Node::AtRule { name: "@variant", params: path.get(idx + 1).copied().unwrap_or(start), nodes: Vec::new_in(alloc) };
            Node::AtRule { name: "@custom-variant", params: name, nodes: Vec::from_iter_in([dependency], alloc) }
        })
        .collect();
    let output = to_css(&nodes).replace(';', " { … }").replacen(
        &format!("@custom-variant {start} {{"),
        &format!("@custom-variant {start} {{ /* ← */"),
        1,
    );
    format!("Circular dependency detected in custom variants:\n\n{output}")
}

fn substitute_at_imports<'a>(
    ast: &mut Nodes<'a>,
    base: &'a str,
    load: &mut Option<Box<LoadStylesheet<'_>>>,
    extension: Option<ExtensionRef<'a>>,
    depth: usize,
    alloc: &'a Allocator,
) -> Result<u8, String> {
    let mut features = features::NONE;
    walk_scoped(ast, Scope::default(), &mut |node, _| {
        let Node::AtRule { name: name @ ("@import" | "@reference"), params, .. } = node else {
            return Ok(Walk::Continue);
        };
        let Some(mut import) = parse_import_params(&vp::parse(params, alloc), alloc)? else {
            return Ok(Walk::Continue);
        };
        if *name == "@reference" {
            import.media = Some("reference");
        }
        features |= features::AT_IMPORT;
        let uri = import.uri;
        if uri.starts_with("data:") || uri.starts_with("http://") || uri.starts_with("https://") {
            return Ok(Walk::Continue);
        }
        if depth > 100 {
            return Err(format!("Exceeded maximum recursion depth while resolving `{uri}` in `{base}`)"));
        }
        let (content, loaded_base) = match (extension.and_then(|e| e.stylesheet(uri)), load.as_mut()) {
            (Some(content), _) => (content, base),
            (None, Some(loader)) => {
                let loaded = loader(uri, base)?;
                (alloc.str(&loaded.content), alloc.str(&loaded.base))
            }
            (None, None) => return Err("No `loadStylesheet` function provided to `compile`".to_owned()),
        };
        let mut imported = css_parser::parse(content, alloc).map_err(|e| e.to_string())?;
        features |= substitute_at_imports(&mut imported, loaded_base, load, extension, depth + 1, alloc)?;
        let ctx = Ctx { base: Some(loaded_base), ..Ctx::default() };
        let mut root = Vec::from_iter_in([Node::Context { ctx, nodes: imported }], alloc);
        if let Some(layer) = import.layer {
            root = Vec::from_iter_in([Node::AtRule { name: "@layer", params: layer, nodes: root }], alloc);
        }
        if let Some(media) = import.media {
            root = Vec::from_iter_in([Node::AtRule { name: "@media", params: media, nodes: root }], alloc);
        }
        if let Some(supports) = import.supports {
            let params = if supports.starts_with('(') { supports } else { alloc.concat(&["(", supports, ")"]) };
            root = Vec::from_iter_in([Node::AtRule { name: "@supports", params, nodes: root }], alloc);
        }
        let wrapper = Node::Context { ctx: Ctx::default(), nodes: root };
        Ok(Walk::ReplaceSkip(Vec::from_iter_in([wrapper], alloc)))
    })?;
    Ok(features)
}

struct Import<'a> {
    uri: &'a str,
    layer: Option<&'a str>,
    media: Option<&'a str>,
    supports: Option<&'a str>,
}

fn parse_import_params<'a>(params: &[ValueNode<'a>], alloc: &'a Allocator) -> Result<Option<Import<'a>>, String> {
    let mut import = Import { uri: "", layer: None, media: None, supports: None };
    for (i, node) in params.iter().enumerate() {
        match node {
            ValueNode::Separator(_) => continue,
            ValueNode::Word(value) if import.uri.is_empty() => {
                if !matches!(value.as_bytes().first(), Some(b'"' | b'\'')) {
                    return Ok(None);
                }
                import.uri = unquote(value);
                continue;
            }
            ValueNode::Function(name, _) if name.eq_ignore_ascii_case("url") => return Ok(None),
            _ => {}
        }
        if import.uri.is_empty() {
            return Ok(None);
        }
        if node.value().eq_ignore_ascii_case("layer") {
            if import.layer.is_some() {
                return Ok(None);
            }
            if import.supports.is_some() {
                return Err("`layer(…)` in an `@import` should come before any other functions or conditions".to_owned());
            }
            import.layer = Some(match node {
                ValueNode::Function(_, nodes) => vp::to_css(nodes, alloc),
                _ => "",
            });
            continue;
        }
        if let ValueNode::Function(name, nodes) = node
            && name.eq_ignore_ascii_case("supports")
        {
            if import.supports.is_some() {
                return Ok(None);
            }
            import.supports = Some(vp::to_css(nodes, alloc));
            continue;
        }
        import.media = Some(vp::to_css(&params[i..], alloc));
        break;
    }
    Ok((!import.uri.is_empty()).then_some(import))
}

fn utility_root(params: &str) -> &str {
    params.strip_suffix("-*").unwrap_or(params)
}

fn apply_dependencies<'a>(nodes: &[Node<'a>], ds: &mut Session<'_, 'a>) -> IndexSet<&'a str> {
    let mut dependencies = IndexSet::new();
    walk(nodes, &mut |node| {
        if let Node::AtRule { name: "@apply", params, .. } = node {
            for candidate in params.split(|c: char| c.is_whitespace()) {
                for parsed in ds.parse_candidate(candidate).1 {
                    if let CandidateKind::Static { root } | CandidateKind::Functional { root, .. } = parsed.kind {
                        dependencies.insert(root);
                    }
                }
            }
        }
        true
    });
    dependencies
}

fn contains_apply(nodes: &[Node<'_>]) -> bool {
    let mut found = false;
    walk(nodes, &mut |node| {
        found |= node.is_at_rule("@apply");
        !found
    });
    found
}

struct ApplySort<'u, 'a> {
    utilities: &'u [CustomUtility<'a>],
    dependencies: std::vec::Vec<IndexSet<&'a str>>,
    definitions: IndexMap<&'a str, std::vec::Vec<usize>>,
    seen: std::vec::Vec<bool>,
    wip: std::vec::Vec<bool>,
    sorted: std::vec::Vec<usize>,
}

impl<'a> ApplySort<'_, 'a> {
    fn visit(&mut self, node: usize, path: &mut std::vec::Vec<usize>, ds: &mut Session<'_, 'a>) -> Result<(), String> {
        if self.seen[node] {
            return Ok(());
        }
        if self.wip[node] {
            let position = path.iter().position(|n| *n == node).unwrap_or(0);
            let next = path[(position + 1) % path.len()];
            let next_root = utility_root(self.utilities[next].params);
            let mut error = None;
            walk(self.body(node, ds.ds), &mut |child| {
                if let Node::AtRule { name: "@apply", params, .. } = child {
                    for candidate in params.split(|c: char| c.is_whitespace()) {
                        if ds.parse_candidate(candidate).1.iter().any(|c| c.root() == Some(next_root)) {
                            error.get_or_insert_with(|| {
                                format!("You cannot `@apply` the `{candidate}` utility here because it creates a circular dependency.")
                            });
                        }
                    }
                }
                error.is_none()
            });
            if let Some(error) = error {
                return Err(error);
            }
            return Err(format!("Circular dependency detected:\n\n{}\nRelies on:\n\n{}", self.print(node, ds), self.print(next, ds)));
        }
        self.wip[node] = true;
        for dependency in self.dependencies[node].clone() {
            for definition in self.definitions.get(dependency).cloned().unwrap_or_default() {
                path.push(node);
                self.visit(definition, path, ds)?;
                path.pop();
            }
        }
        self.seen[node] = true;
        self.wip[node] = false;
        self.sorted.push(node);
        Ok(())
    }

    fn body<'d>(&self, node: usize, ds: &'d DesignSystem<'a>) -> &'d [Node<'a>] {
        let utility = &self.utilities[node];
        ds.utilities.body(utility.name, utility.index).map_or(&[], |b| b.as_slice())
    }

    fn print(&self, node: usize, ds: &Session<'_, 'a>) -> String {
        let nodes = orisu_ast::clone_in(self.body(node, ds.ds), ds.alloc);
        to_css(&[Node::AtRule { name: "@utility", params: self.utilities[node].params, nodes }])
    }
}

fn substitute_at_apply<'a>(
    ast: &mut Nodes<'a>,
    utilities: &[CustomUtility<'a>],
    ds: &mut DesignSystem<'a>,
    caches: &mut Caches<'a>,
    alloc: &'a Allocator,
) -> Result<bool, String> {
    let mut session = Session::new(ds, alloc, caches);
    let design_system = session.ds;
    let empty = Vec::new_in(alloc);
    let body = |u: &CustomUtility<'a>| design_system.utilities.body(u.name, u.index).unwrap_or(&empty);
    let mut sort = ApplySort {
        utilities,
        dependencies: utilities.iter().map(|u| apply_dependencies(body(u), &mut session)).collect(),
        definitions: IndexMap::new(),
        seen: vec![false; utilities.len()],
        wip: vec![false; utilities.len()],
        sorted: std::vec::Vec::new(),
    };
    for (i, utility) in utilities.iter().enumerate() {
        sort.definitions.entry(utility_root(utility.params)).or_default().push(i);
    }
    let mut found = false;
    for (i, utility) in utilities.iter().enumerate() {
        if contains_apply(body(utility)) {
            found = true;
            sort.visit(i, &mut std::vec::Vec::new(), &mut session)?;
        }
    }
    for i in sort.sorted {
        with_body(ds, caches, alloc, &utilities[i], replace_applies)?;
    }
    found |= replace_applies(ast, &mut Session::new(ds, alloc, caches))?;
    Ok(found)
}

pub fn replace_applies<'a>(nodes: &mut Nodes<'a>, ds: &mut Session<'_, 'a>) -> Result<bool, String> {
    let mut found = false;
    let mut i = 0;
    while i < nodes.len() {
        match &mut nodes[i] {
            Node::AtRule { name: "@keyframes", nodes: children, .. } => {
                if contains_apply(children) {
                    return Err("You cannot use `@apply` inside `@keyframes`.".to_owned());
                }
            }
            Node::AtRule { name: "@utility", .. } => {}
            Node::AtRule { name: "@apply", params, nodes: children } => {
                found = true;
                if let Some(replacement) = expand_apply(params, !children.is_empty(), ds)? {
                    let len = replacement.len();
                    nodes.splice(i..=i, replacement);
                    i += len;
                    continue;
                }
            }
            node => {
                if let Some(children) = node.children_mut() {
                    found |= replace_applies(children, ds)?;
                }
            }
        }
        i += 1;
    }
    Ok(found)
}

fn expand_apply<'a>(params: &'a str, has_body: bool, ds: &mut Session<'_, 'a>) -> Result<Option<Nodes<'a>>, String> {
    let mut candidates: IndexSet<&str> = IndexSet::new();
    let (mut normal, mut dashed) = (std::vec::Vec::new(), std::vec::Vec::new());
    for part in split_whitespace_runs(params) {
        if part.starts_with("--") {
            dashed.push(part);
        } else {
            normal.push(part);
        }
        candidates.insert(part);
    }
    if !dashed.is_empty() {
        if normal.is_empty() {
            return Ok(None);
        }
        return Err(format!(
            "You cannot use `@apply` with both mixins and utilities. Please move `@apply {}` into a separate rule.",
            dashed.join(" ")
        ));
    }
    if has_body && !normal.is_empty() {
        return Err(format!("The rule `@apply {}` must not have a body.", normal.join(" ")));
    }
    let mut invalid: Option<String> = None;
    let compiled = compile_candidates(candidates.iter().copied(), ds, false, |c| {
        invalid.get_or_insert_with(|| c.to_owned());
    });
    if let Some(candidate) = invalid {
        return Err(apply_error(&candidate, ds));
    }
    let mut out = Vec::new_in(ds.alloc);
    for node in compiled.ast_nodes {
        match node {
            Node::Rule { nodes, .. } => out.extend(nodes),
            node => out.push(node),
        }
    }
    Ok(Some(out))
}

fn split_whitespace_runs(input: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(input);
    std::iter::from_fn(move || {
        let current = rest?;
        match current.find(char::is_whitespace) {
            Some(start) => {
                let tail = &current[start..];
                let end = tail.find(|c: char| !c.is_whitespace()).unwrap_or(tail.len());
                rest = Some(&tail[end..]);
                Some(&current[..start])
            }
            None => {
                rest = None;
                Some(current)
            }
        }
    })
}

fn apply_error(candidate: &str, ds: &mut Session<'_, '_>) -> String {
    if let Some(prefix) = ds.theme.prefix()
        && !candidate.starts_with(prefix)
    {
        return format!("Cannot apply unprefixed utility class `{candidate}`. Did you mean `{prefix}:{candidate}`?");
    }
    if ds.invalid_candidates.contains(candidate) {
        return format!(
            "Cannot apply utility class `{candidate}` because it has been explicitly disabled: https://tailwindcss.com/docs/detecting-classes-in-source-files#explicitly-excluding-classes"
        );
    }
    let parts = segment(candidate, b':');
    if let Some((utility, variants)) = parts.split_last()
        && !variants.is_empty()
        && ds.candidates_to_css(&[utility])[0].is_some()
    {
        let checks: std::vec::Vec<String> = variants.iter().map(|v| format!("{v}:[--tw-variant-check:1]")).collect();
        let checks: std::vec::Vec<&str> = checks.iter().map(String::as_str).collect();
        let compiled = ds.candidates_to_css(&checks);
        let unknown: std::vec::Vec<String> =
            variants.iter().zip(&compiled).filter(|(_, c)| c.is_none()).map(|(v, _)| format!("`{v}`")).collect();
        match unknown.len() {
            0 => {}
            1 => {
                return format!("Cannot apply utility class `{candidate}` because the {} variant does not exist.", unknown[0]);
            }
            2 => {
                return format!(
                    "Cannot apply utility class `{candidate}` because the {} and {} variants do not exist.",
                    unknown[0], unknown[1]
                );
            }
            n => {
                return format!(
                    "Cannot apply utility class `{candidate}` because the {}, and {} variants do not exist.",
                    unknown[..n - 1].join(", "),
                    unknown[n - 1]
                );
            }
        }
    }
    if ds.theme.is_empty() {
        return format!(
            "Cannot apply unknown utility class `{candidate}`. Are you using CSS modules or similar and missing `@reference`? https://tailwindcss.com/docs/functions-and-directives#reference-directive"
        );
    }
    format!("Cannot apply unknown utility class `{candidate}`")
}
