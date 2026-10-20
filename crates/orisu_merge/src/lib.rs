use indexmap::{IndexMap, IndexSet};
use orisu_ast::Node;
use orisu_compiler::compile::compile_candidates;
use orisu_compiler::design_system::Session;
use orisu_parser::segment::segment;
use rustc_hash::{FxBuildHasher, FxHashMap};

const SIDES: [&str; 4] = ["bs", "ie", "be", "is"];
const CORNERS: [&str; 4] = ["ss", "se", "ee", "es"];

/// Variants whose relative order changes the generated selector meaningfully.
const ORDER_SENSITIVE: &[&str] = &[
    "*",
    "**",
    "before",
    "after",
    "backdrop",
    "placeholder",
    "file",
    "marker",
    "selection",
    "first-line",
    "first-letter",
    "details-content",
];

fn side_index(side: &str) -> Option<&'static [usize]> {
    Some(match side {
        "top" | "block-start" => &[0],
        "right" | "inline-end" => &[1],
        "bottom" | "block-end" => &[2],
        "left" | "inline-start" => &[3],
        "block" => &[0, 2],
        "inline" => &[1, 3],
        _ => return None,
    })
}

fn corner_index(corner: &str) -> Option<&'static [usize]> {
    Some(match corner {
        "top-left" | "start-start" => &[0],
        "top-right" | "start-end" => &[1],
        "bottom-right" | "end-end" => &[2],
        "bottom-left" | "end-start" => &[3],
        _ => return None,
    })
}

fn push_sides(out: &mut Vec<String>, base: &str, sides: &[usize]) {
    out.extend(sides.iter().map(|&i| format!("{base}:{}", SIDES[i])));
}

/// Expands a CSS property into the longhand slots it writes, so shorthands cover their longhands.
pub fn property_slots(property: &str) -> Vec<String> {
    let mut out = Vec::new();
    if property.starts_with("--") {
        out.push(property.to_owned());
        return out;
    }
    let property = property.strip_prefix("-webkit-").unwrap_or(property);
    for base in ["padding", "margin", "scroll-margin", "scroll-padding"] {
        if property == base {
            push_sides(&mut out, base, &[0, 1, 2, 3]);
            return out;
        }
        if let Some(sides) = property.strip_prefix(base).and_then(|r| r.strip_prefix('-')).and_then(side_index) {
            push_sides(&mut out, base, sides);
            return out;
        }
    }
    if property == "inset" {
        push_sides(&mut out, "inset", &[0, 1, 2, 3]);
        return out;
    }
    if let Some(sides) = property.strip_prefix("inset-").and_then(side_index).or_else(|| side_index(property)) {
        push_sides(&mut out, "inset", sides);
        return out;
    }
    if let Some(rest) = property.strip_prefix("border") {
        if let Some(rest) = rest.strip_suffix("-radius") {
            let corners: &[usize] = match rest.strip_prefix('-') {
                None => &[0, 1, 2, 3],
                Some(corner) => match corner_index(corner) {
                    Some(c) => c,
                    None => return vec![property.to_owned()],
                },
            };
            out.extend(corners.iter().map(|&i| format!("border-radius:{}", CORNERS[i])));
            return out;
        }
        let (sides, part): (&[usize], &str) = match rest {
            "" => (&[0, 1, 2, 3], ""),
            "-width" | "-style" | "-color" => (&[0, 1, 2, 3], rest),
            _ => {
                let rest = &rest[1..];
                let (side, part) = match ["-width", "-style", "-color"].iter().find(|s| rest.ends_with(**s)) {
                    Some(part) => (&rest[..rest.len() - part.len()], *part),
                    None => (rest, ""),
                };
                match side_index(side) {
                    Some(sides) => (sides, part),
                    None => return vec![property.to_owned()],
                }
            }
        };
        let parts: &[&str] = if part.is_empty() { &["-width", "-style", "-color"] } else { &[part] };
        for part in parts {
            push_sides(&mut out, &format!("border{part}"), sides);
        }
        return out;
    }
    let longhands: &[&str] = match property {
        "gap" => &["row-gap", "column-gap"],
        "grid-gap" => &["row-gap", "column-gap"],
        "overflow" => &["overflow-x", "overflow-y"],
        "overscroll-behavior" => &["overscroll-behavior-x", "overscroll-behavior-y"],
        "place-content" => &["align-content", "justify-content"],
        "place-items" => &["align-items", "justify-items"],
        "place-self" => &["align-self", "justify-self"],
        "flex" => &["flex-grow", "flex-shrink", "flex-basis"],
        "flex-flow" => &["flex-direction", "flex-wrap"],
        "grid-column" => &["grid-column-start", "grid-column-end"],
        "grid-row" => &["grid-row-start", "grid-row-end"],
        "grid-area" => &["grid-row-start", "grid-column-start", "grid-row-end", "grid-column-end"],
        "grid-template" => &["grid-template-rows", "grid-template-columns", "grid-template-areas"],
        "outline" => &["outline-width", "outline-style", "outline-color"],
        "text-decoration" => &["text-decoration-line", "text-decoration-style", "text-decoration-color", "text-decoration-thickness"],
        "columns" => &["column-width", "column-count"],
        "list-style" => &["list-style-type", "list-style-position", "list-style-image"],
        "transition" => {
            &["transition-property", "transition-duration", "transition-timing-function", "transition-delay", "transition-behavior"]
        }
        "container" => &["container-name", "container-type"],
        _ => &[],
    };
    if longhands.is_empty() {
        out.push(property.to_owned());
    } else {
        out.extend(longhands.iter().map(|l| (*l).to_owned()));
    }
    out
}

/// Splits a raw class into its normalized variant key, important flag and utility base.
pub fn split_class<'c>(raw: &'c str, prefix: Option<&str>) -> (String, bool, &'c str) {
    let mut parts = segment(raw, b':');
    let mut base = parts.pop().unwrap_or(raw);
    if let Some(prefix) = prefix.filter(|p| !p.is_empty())
        && parts.first() == Some(&prefix)
    {
        parts.remove(0);
    }
    let mut important = false;
    if let Some(rest) = base.strip_suffix('!') {
        base = rest;
        important = true;
    } else if let Some(rest) = base.strip_prefix('!') {
        base = rest;
        important = true;
    }
    let mut key = String::new();
    let mut run: Vec<&str> = Vec::new();
    let flush = |run: &mut Vec<&str>, key: &mut String| {
        run.sort_unstable();
        for v in run.drain(..) {
            key.push_str(v);
            key.push(':');
        }
    };
    for part in parts.iter().copied() {
        if part.starts_with('[') || ORDER_SENSITIVE.contains(&part) {
            flush(&mut run, &mut key);
            key.push_str(part);
            key.push(':');
        } else {
            run.push(part);
        }
    }
    flush(&mut run, &mut key);
    (key, important, base)
}

/// What a utility writes: the longhand slots it sets, the custom properties it sets and the ones it reads.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Footprint {
    props: Vec<String>,
    vars: Vec<String>,
    refs: Vec<String>,
}

impl Footprint {
    /// Whether this (later) footprint leaves nothing observable of `earlier`.
    pub fn erases(&self, earlier: &Footprint) -> bool {
        if earlier.props.is_empty() {
            return earlier.vars.iter().all(|v| self.vars.binary_search(v).is_ok());
        }
        earlier.props.iter().all(|p| self.props.binary_search(p).is_ok())
            && earlier
                .vars
                .iter()
                .all(|v| self.vars.binary_search(v).is_ok() || self.refs.binary_search_by(|r| r.as_str().cmp(var_name(v))).is_err())
    }
}

fn var_name(slot: &str) -> &str {
    slot.rsplit_once('|').map_or(slot, |(_, name)| name)
}

fn collect_refs(value: &str, out: &mut IndexSet<String, FxBuildHasher>) {
    let mut rest = value;
    while let Some(at) = rest.find("var(") {
        rest = rest[at + 4..].trim_start();
        let end = rest.find([',', ')', ' ']).unwrap_or(rest.len());
        if rest.starts_with("--") {
            out.insert(rest[..end].to_owned());
        }
        rest = &rest[end..];
    }
}

#[derive(Default)]
struct Collector {
    props: IndexSet<String, FxBuildHasher>,
    vars: IndexSet<String, FxBuildHasher>,
    refs: IndexSet<String, FxBuildHasher>,
}

impl Collector {
    fn walk(&mut self, nodes: &[Node<'_>], path: &str) {
        for node in nodes {
            match node {
                Node::Decl { property, value: Some(value), .. } => {
                    collect_refs(value, &mut self.refs);
                    let target = if property.starts_with("--") { &mut self.vars } else { &mut self.props };
                    for slot in property_slots(property) {
                        target.insert(if path.is_empty() { slot } else { format!("{path}|{slot}") });
                    }
                }
                Node::Rule { selector, nodes } => {
                    let nested = if *selector == "&" {
                        path.to_owned()
                    } else if path.is_empty() {
                        (*selector).to_owned()
                    } else {
                        format!("{path} {selector}")
                    };
                    self.walk(nodes, &nested);
                }
                Node::AtRule { name, nodes, .. } if !matches!(*name, "@property" | "@keyframes" | "@font-face") => {
                    self.walk(nodes, path);
                }
                Node::Context { nodes, .. } => self.walk(nodes, path),
                _ => {}
            }
        }
    }

    fn finish(self) -> Option<Footprint> {
        if self.props.is_empty() && self.vars.is_empty() {
            return None;
        }
        let sorted = |set: IndexSet<String, FxBuildHasher>| {
            let mut v: Vec<String> = set.into_iter().collect();
            v.sort_unstable();
            v
        };
        Some(Footprint { props: sorted(self.props), vars: sorted(self.vars), refs: sorted(self.refs) })
    }
}

/// What a utility (without variants) writes, or `None` when it is not a known class.
pub fn footprint(ds: &mut Session<'_, '_>, base: &str) -> Option<Footprint> {
    let candidate = match ds.theme.prefix().filter(|p| !p.is_empty()) {
        Some(prefix) => format!("{prefix}:{base}"),
        None => base.to_owned(),
    };
    let mut valid = true;
    let compiled = compile_candidates([candidate.as_str()], ds, false, |_| valid = false);
    if !valid || compiled.ast_nodes.is_empty() {
        return None;
    }
    let mut collector = Collector::default();
    for node in &compiled.ast_nodes {
        match node {
            Node::Rule { nodes, .. } => collector.walk(nodes, ""),
            Node::AtRoot(_) => {}
            node => collector.walk(std::slice::from_ref(node), ""),
        }
    }
    collector.finish()
}

/// Merges class lists the way `cn` does: a class is dropped when a single later class with the same variants erases it.
pub struct Merger<'s, 'a> {
    ds: Session<'s, 'a>,
    cache: FxHashMap<String, Option<u32>>,
    shapes: IndexSet<Footprint, FxBuildHasher>,
}

impl<'s, 'a> Merger<'s, 'a> {
    pub fn new(ds: Session<'s, 'a>) -> Self {
        Self { ds, cache: FxHashMap::default(), shapes: IndexSet::default() }
    }

    fn shape(&mut self, base: &str) -> Option<u32> {
        if let Some(cached) = self.cache.get(base) {
            return *cached;
        }
        let id = footprint(&mut self.ds, base).map(|f| self.shapes.insert_full(f).0 as u32);
        self.cache.insert(base.to_owned(), id);
        id
    }

    pub fn merge<'c>(&mut self, classes: impl IntoIterator<Item = &'c str>) -> String {
        let classes: Vec<&str> = classes.into_iter().flat_map(str::split_ascii_whitespace).collect();
        let mut later: FxHashMap<(String, bool), Vec<u32>> = FxHashMap::default();
        let mut keep = vec![true; classes.len()];
        for (i, class) in classes.iter().enumerate().rev() {
            let (key, important, base) = split_class(class, self.ds.theme.prefix());
            let Some(id) = self.shape(base) else { continue };
            let shapes = &self.shapes;
            let earlier = &shapes[id as usize];
            let seen = later.entry((key, important)).or_default();
            if seen.iter().any(|&l| shapes[l as usize].erases(earlier)) {
                keep[i] = false;
            } else {
                seen.push(id);
            }
        }
        let mut out = String::new();
        for (class, keep) in classes.iter().zip(keep) {
            if keep {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(class);
            }
        }
        out
    }

    /// Shape table for the given classes, keyed by utility base, for the JS `cn` runtime.
    pub fn table<'c>(&mut self, classes: impl IntoIterator<Item = &'c str>) -> MergeTable {
        let mut entries: IndexMap<String, u32, FxBuildHasher> = IndexMap::default();
        for class in classes {
            let (_, _, base) = split_class(class, self.ds.theme.prefix());
            if entries.contains_key(base) {
                continue;
            }
            if let Some(id) = self.shape(base) {
                entries.insert(base.to_owned(), id);
            }
        }
        let shapes = &self.shapes;
        let erases = shapes
            .iter()
            .map(|later| shapes.iter().enumerate().filter(|(_, earlier)| later.erases(earlier)).map(|(i, _)| i as u32).collect())
            .collect();
        MergeTable { erases, entries }
    }
}

pub struct MergeTable {
    /// For each shape, the shapes it erases (itself included).
    pub erases: Vec<Vec<u32>>,
    pub entries: IndexMap<String, u32, FxBuildHasher>,
}
