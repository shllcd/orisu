// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use orisu_allocator::{AllocExt, Allocator};
use orisu_ast::bytes::{ByteTable, trim};
use orisu_parser::decode::decode_arbitrary_value;
use orisu_parser::segment::{is_valid_arbitrary, segment};
use smallvec::SmallVec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier<'a> {
    Arbitrary(&'a str),
    Named(&'a str),
}

impl<'a> Modifier<'a> {
    #[inline]
    pub fn value(&self) -> &'a str {
        match *self {
            Modifier::Arbitrary(v) | Modifier::Named(v) => v,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UtilityValue<'a> {
    Arbitrary { data_type: Option<&'a str>, value: &'a str },
    Named { value: &'a str, fraction: Option<&'a str> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VariantValue<'a> {
    Arbitrary(&'a str),
    Named(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variant<'a> {
    Arbitrary { selector: &'a str, relative: bool },
    Static { root: &'a str },
    Functional { root: &'a str, value: Option<VariantValue<'a>>, modifier: Option<Modifier<'a>> },
    Compound { root: &'a str, modifier: Option<Modifier<'a>>, variant: &'a Variant<'a> },
}

impl<'a> Variant<'a> {
    #[inline]
    pub fn root(&self) -> Option<&'a str> {
        match *self {
            Variant::Arbitrary { .. } => None,
            Variant::Static { root } | Variant::Functional { root, .. } | Variant::Compound { root, .. } => Some(root),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateKind<'a> {
    Arbitrary { property: &'a str, value: &'a str, modifier: Option<Modifier<'a>> },
    Static { root: &'a str },
    Functional { root: &'a str, value: Option<UtilityValue<'a>>, modifier: Option<Modifier<'a>> },
}

/// `variants` are in reverse source order (closest to the utility first), as in Tailwind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Candidate<'a> {
    pub kind: CandidateKind<'a>,
    pub variants: &'a [&'a Variant<'a>],
    pub important: bool,
    pub raw: &'a str,
}

impl<'a> Candidate<'a> {
    #[inline]
    pub fn root(&self) -> Option<&'a str> {
        match self.kind {
            CandidateKind::Static { root } | CandidateKind::Functional { root, .. } => Some(root),
            CandidateKind::Arbitrary { .. } => None,
        }
    }

    #[inline]
    pub fn modifier(&self) -> Option<Modifier<'a>> {
        match self.kind {
            CandidateKind::Static { .. } => None,
            CandidateKind::Arbitrary { modifier, .. } | CandidateKind::Functional { modifier, .. } => modifier,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantKind {
    Static,
    Functional,
    Compound,
}

pub trait Registry<'a> {
    fn alloc(&self) -> &'a Allocator;
    fn prefix(&self) -> Option<&str>;
    fn has_static_utility(&self, root: &str) -> bool;
    fn has_functional_utility(&self, root: &str) -> bool;
    fn variant_kind(&self, root: &str) -> Option<VariantKind>;
    fn compounds_with(&self, root: &str, variant: &Variant<'a>) -> bool;
    /// Whether the compound variant `root` folds its `/modifier` into its inner variant.
    fn modifier_in_value(&self, root: &str) -> bool;
    /// Cached entry point that recursively calls [`parse_variant`].
    fn parse_variant(&mut self, raw: &'a str) -> Option<&'a Variant<'a>>;
}

pub type Candidates<'a> = SmallVec<[Candidate<'a>; 2]>;

static NAMED_VALUE: ByteTable = ByteTable::new(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.%-");

#[inline]
fn is_valid_named_value(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| NAMED_VALUE.has(b))
}

#[inline]
fn is_blank(value: &str) -> bool {
    trim(value).is_empty()
}

#[inline]
fn last(s: &str) -> Option<u8> {
    s.as_bytes().last().copied()
}

#[inline]
fn first(s: &str) -> Option<u8> {
    s.as_bytes().first().copied()
}

#[inline]
fn starts_with_dashes(s: &str) -> bool {
    s.as_bytes().starts_with(b"--")
}

pub fn parse_candidate<'a, R: Registry<'a> + ?Sized>(input: &'a str, registry: &mut R) -> Candidates<'a> {
    let mut out = Candidates::new();
    let alloc = registry.alloc();
    let mut raw_variants = segment(input, b':');
    if let Some(prefix) = registry.prefix() {
        if raw_variants.len() == 1 || raw_variants[0] != prefix {
            return out;
        }
        raw_variants.remove(0);
    }
    let Some(mut base) = raw_variants.pop() else {
        return out;
    };

    let mut variants = orisu_allocator::Vec::with_capacity_in(raw_variants.len(), alloc);
    for raw in raw_variants.iter().rev() {
        let Some(variant) = registry.parse_variant(raw) else {
            return out;
        };
        variants.push(variant);
    }
    let variants: &'a [&'a Variant<'a>] = variants.into_bump_slice();

    let mut important = false;
    if last(base) == Some(b'!') {
        important = true;
        base = &base[..base.len() - 1];
    } else if first(base) == Some(b'!') {
        important = true;
        base = &base[1..];
    }

    let candidate = |kind| Candidate { kind, variants, important, raw: input };

    if registry.has_static_utility(base) && !base.contains('[') {
        out.push(candidate(CandidateKind::Static { root: base }));
    }

    let parts = segment(base, b'/');
    if parts.len() > 2 {
        return out;
    }
    let mut base_without_modifier = parts[0];
    let modifier_segment = parts.get(1).copied();
    let Some(modifier) = parse_optional_modifier(modifier_segment, alloc) else {
        return out;
    };

    if first(base_without_modifier) == Some(b'[') {
        if last(base_without_modifier) != Some(b']') {
            return out;
        }
        let code = base_without_modifier.as_bytes().get(1).copied().unwrap_or(0);
        if code != b'-' && !code.is_ascii_lowercase() {
            return out;
        }
        base_without_modifier = &base_without_modifier[1..base_without_modifier.len() - 1];
        let idx = match base_without_modifier.find(':') {
            Some(idx) if idx != 0 && idx != base_without_modifier.len() - 1 => idx,
            _ => return out,
        };
        let property = &base_without_modifier[..idx];
        let Some(value) = decode_valid_arbitrary_value(&base_without_modifier[idx + 1..], alloc) else {
            return out;
        };
        out.push(candidate(CandidateKind::Arbitrary { property, value, modifier }));
        return out;
    }

    let push_root = |root: &'a str, value: Option<&'a str>, out: &mut Candidates<'a>| -> bool {
        let Some(value) = value else {
            out.push(candidate(CandidateKind::Functional { root, value: None, modifier }));
            return true;
        };
        if let Some(start) = value.find('[') {
            if last(value) != Some(b']') {
                return false;
            }
            let Some(mut arbitrary) = decode_valid_arbitrary_value(&value[start + 1..value.len() - 1], alloc) else {
                return true;
            };
            let mut typehint = None;
            for (i, b) in arbitrary.bytes().enumerate() {
                if b == b':' {
                    typehint = Some(&arbitrary[..i]);
                    arbitrary = &arbitrary[i + 1..];
                    break;
                }
                if b == b'-' || b.is_ascii_lowercase() {
                    continue;
                }
                break;
            }
            if is_blank(arbitrary) || typehint == Some("") {
                return true;
            }
            out.push(candidate(CandidateKind::Functional {
                root,
                value: Some(UtilityValue::Arbitrary { data_type: typehint, value: arbitrary }),
                modifier,
            }));
        } else {
            let fraction = match (modifier_segment, modifier) {
                (Some(segment), Some(Modifier::Named(_))) => Some(alloc.concat(&[value, "/", segment])),
                _ => None,
            };
            if !is_valid_named_value(value) {
                return true;
            }
            out.push(candidate(CandidateKind::Functional { root, value: Some(UtilityValue::Named { value, fraction }), modifier }));
        }
        true
    };

    match last(base_without_modifier) {
        Some(b']') => {
            let Some(idx) = base_without_modifier.find("-[") else {
                return out;
            };
            let root = &base_without_modifier[..idx];
            if registry.has_functional_utility(root) {
                push_root(root, Some(&base_without_modifier[idx + 1..]), &mut out);
            }
        }
        Some(b')') => {
            let Some(idx) = base_without_modifier.find("-(") else {
                return out;
            };
            let root = &base_without_modifier[..idx];
            if !registry.has_functional_utility(root) {
                return out;
            }
            let mut value = &base_without_modifier[idx + 2..base_without_modifier.len() - 1];
            let parts = segment(value, b':');
            let mut data_type = None;
            if parts.len() == 2 {
                data_type = Some(parts[0]);
                value = parts[1];
            }
            if !starts_with_dashes(value) || !is_valid_arbitrary(value) {
                return out;
            }
            let wrapped = match data_type {
                None => alloc.concat(&["[", wrap_var(value, alloc), "]"]),
                Some(data_type) => alloc.concat(&["[", data_type, ":", wrap_var(value, alloc), "]"]),
            };
            push_root(root, Some(wrapped), &mut out);
        }
        _ => {
            for (root, value, ()) in Roots::new(base_without_modifier, |root| registry.has_functional_utility(root).then_some(())) {
                if !push_root(root, value, &mut out) {
                    break;
                }
            }
        }
    }
    out
}

/// Validates before decoding, which would turn the unbalanced `calc(1px` into the balanced `calc()1px`.
fn decode_valid_arbitrary_value<'a>(raw: &'a str, alloc: &'a Allocator) -> Option<&'a str> {
    is_valid_arbitrary(raw).then(|| decode_arbitrary_value(raw, alloc))
}

/// Decodes the inside of `[…]`, rejecting unbalanced and blank values.
fn bracketed_value<'a>(inner: &'a str, alloc: &'a Allocator) -> Option<&'a str> {
    decode_valid_arbitrary_value(inner, alloc).filter(|value| !is_blank(value))
}

/// The value `(--name)` stands for.
pub fn wrap_var<'a>(name: &str, alloc: &'a Allocator) -> &'a str {
    alloc.concat(&["var(", name, ")"])
}

/// The `--name` a printed `(--name)` shorthand holds when `value` is a lone `var()`; inverse of [`wrap_var`].
pub(crate) fn unwrap_var(value: &str) -> &str {
    &value[4..value.len() - 1]
}

/// `None` when the modifier is present but invalid.
fn parse_optional_modifier<'a>(modifier: Option<&'a str>, alloc: &'a Allocator) -> Option<Option<Modifier<'a>>> {
    match modifier {
        None => Some(None),
        Some(m) => parse_modifier(m, alloc).map(Some),
    }
}

pub fn parse_modifier<'a>(modifier: &'a str, alloc: &'a Allocator) -> Option<Modifier<'a>> {
    if first(modifier) == Some(b'[') && last(modifier) == Some(b']') {
        return bracketed_value(&modifier[1..modifier.len() - 1], alloc).map(Modifier::Arbitrary);
    }
    if first(modifier) == Some(b'(') && last(modifier) == Some(b')') {
        let inner = &modifier[1..modifier.len() - 1];
        if !starts_with_dashes(inner) || !is_valid_arbitrary(inner) {
            return None;
        }
        return Some(Modifier::Arbitrary(decode_arbitrary_value(wrap_var(inner, alloc), alloc)));
    }
    is_valid_named_value(modifier).then_some(Modifier::Named(modifier))
}

pub fn parse_variant<'a, R: Registry<'a> + ?Sized>(variant: &'a str, registry: &mut R) -> Option<Variant<'a>> {
    let alloc = registry.alloc();
    if first(variant) == Some(b'[') && last(variant) == Some(b']') {
        if variant.as_bytes().get(1) == Some(&b'@') && variant.contains('&') {
            return None;
        }
        let mut selector = bracketed_value(&variant[1..variant.len() - 1], alloc)?;
        let relative = matches!(first(selector), Some(b'>' | b'+' | b'~'));
        if !relative && first(selector) != Some(b'@') && !selector.contains('&') {
            selector = alloc.concat(&["&:is(", selector, ")"]);
        }
        return Some(Variant::Arbitrary { selector, relative });
    }

    let parts = segment(variant, b'/');
    if parts.len() > 2 {
        return None;
    }
    let without_modifier = parts[0];
    let mut modifier = parts.get(1).copied();

    let (root, mut value) = 'compound: {
        for (root, value, kind) in Roots::new(without_modifier, |root| registry.variant_kind(root)) {
            match kind {
                VariantKind::Static => {
                    if value.is_some() || modifier.is_some() {
                        return None;
                    }
                    return Some(Variant::Static { root });
                }
                VariantKind::Functional => {
                    let parsed_modifier = parse_optional_modifier(modifier, alloc)?;
                    let Some(value) = value else {
                        return Some(Variant::Functional { root, modifier: parsed_modifier, value: None });
                    };
                    if last(value) == Some(b']') {
                        if first(value) != Some(b'[') {
                            continue;
                        }
                        let arbitrary = bracketed_value(&value[1..value.len() - 1], alloc)?;
                        return Some(Variant::Functional {
                            root,
                            modifier: parsed_modifier,
                            value: Some(VariantValue::Arbitrary(arbitrary)),
                        });
                    }
                    if last(value) == Some(b')') {
                        if first(value) != Some(b'(') {
                            continue;
                        }
                        let arbitrary = decode_valid_arbitrary_value(&value[1..value.len() - 1], alloc)?;
                        if is_blank(arbitrary) || !starts_with_dashes(arbitrary) {
                            return None;
                        }
                        return Some(Variant::Functional {
                            root,
                            modifier: parsed_modifier,
                            value: Some(VariantValue::Arbitrary(wrap_var(arbitrary, alloc))),
                        });
                    }
                    if !is_valid_named_value(value) {
                        continue;
                    }
                    return Some(Variant::Functional { root, modifier: parsed_modifier, value: Some(VariantValue::Named(value)) });
                }
                VariantKind::Compound => break 'compound (root, value?),
            }
        }
        return None;
    };
    if let Some(m) = modifier
        && !m.is_empty()
        && registry.modifier_in_value(root)
    {
        value = alloc.concat(&[value, "/", m]);
        modifier = None;
    }
    let sub = registry.parse_variant(value)?;
    if !registry.compounds_with(root, sub) {
        return None;
    }
    let parsed_modifier = parse_optional_modifier(modifier, alloc)?;
    Some(Variant::Compound { root, modifier: parsed_modifier, variant: sub })
}

/// The one root whose value follows it without a dash: `@md`, `@[30rem]`.
pub const ROOT_WITHOUT_DASH: &str = "@";

/// Lazy port of Tailwind's `findRoots`: yields `(root, value, found)` splits at each `-`, longest root first,
/// for roots where `exists` finds something.
struct Roots<'a, F> {
    input: &'a str,
    exists: F,
    state: RootsState,
}

#[derive(Clone, Copy)]
enum RootsState {
    Whole,
    Dash(usize),
    At,
    Done,
}

impl<'a, T, F: FnMut(&str) -> Option<T>> Roots<'a, F> {
    #[inline]
    fn new(input: &'a str, exists: F) -> Self {
        Self { input, exists, state: RootsState::Whole }
    }

    #[inline]
    fn last_dash_before(&self, end: usize) -> Option<usize> {
        memchr::memrchr(b'-', &self.input.as_bytes()[..end])
    }
}

impl<'a, T, F: FnMut(&str) -> Option<T>> Iterator for Roots<'a, F> {
    type Item = (&'a str, Option<&'a str>, T);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.state {
                RootsState::Whole => {
                    self.state = match self.last_dash_before(self.input.len()) {
                        Some(idx) => RootsState::Dash(idx),
                        None => RootsState::At,
                    };
                    if let Some(found) = (self.exists)(self.input) {
                        return Some((self.input, None, found));
                    }
                }
                RootsState::Dash(idx) => {
                    if idx == 0 {
                        self.state = RootsState::At;
                        continue;
                    }
                    self.state = match self.last_dash_before(idx) {
                        Some(next) => RootsState::Dash(next),
                        None => RootsState::At,
                    };
                    let root = &self.input[..idx];
                    if let Some(found) = (self.exists)(root) {
                        let value = &self.input[idx + 1..];
                        if value.is_empty() || root == ROOT_WITHOUT_DASH {
                            self.state = RootsState::At;
                            continue;
                        }
                        return Some((root, Some(value), found));
                    }
                }
                RootsState::At => {
                    self.state = RootsState::Done;
                    if self.input.starts_with(ROOT_WITHOUT_DASH)
                        && let Some(found) = (self.exists)(ROOT_WITHOUT_DASH)
                    {
                        return Some((ROOT_WITHOUT_DASH, Some(&self.input[ROOT_WITHOUT_DASH.len()..]), found));
                    }
                }
                RootsState::Done => return None,
            }
        }
    }
}
