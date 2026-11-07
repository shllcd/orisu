// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use indexmap::IndexMap;
use orisu_allocator::{AllocExt, Allocator, ArenaString};
use orisu_ast::Node;
use orisu_parser::escape::{escape_into, unescape};
use rustc_hash::FxBuildHasher;

/// Tailwind's `ThemeOptions` bits, stored per value and exposed numerically to JS configs.
pub mod opts {
    pub const NONE: u8 = 0;
    pub const INLINE: u8 = 1 << 0;
    pub const REFERENCE: u8 = 1 << 1;
    pub const DEFAULT: u8 = 1 << 2;
    pub const STATIC: u8 = 1 << 3;
    /// Never stored; [`super::Theme::get_options`] adds it for variables the output references.
    pub const USED: u8 = 1 << 4;
}

/// Whether a lookup returns a value itself or a `var()` reference to it; a query mode over the stored [`opts::INLINE`] bit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Inline {
    /// Inline only `@theme inline` values.
    #[default]
    AsDeclared,
    /// Inline every value, as `theme()` does.
    Always,
    /// Treat `@theme inline` values as plain `@theme` values.
    Never,
}

#[derive(Debug, Clone, Copy)]
pub struct ThemeValue<'a> {
    pub value: &'a str,
    pub options: u8,
    var_ref: &'a str,
}

impl<'a> ThemeValue<'a> {
    #[inline]
    fn resolved(&self, inline: Inline) -> &'a str {
        let inlined = match inline {
            Inline::AsDeclared => self.options & opts::INLINE != 0,
            Inline::Always => true,
            Inline::Never => false,
        };
        if inlined { self.value } else { self.var_ref }
    }
}

/// Theme variables referenced by the output so far, by theme index. Only grows within one compiler.
#[derive(Debug, Clone, Default)]
pub struct UsedVars {
    bits: std::vec::Vec<u64>,
}

impl UsedVars {
    #[inline]
    pub fn contains(&self, index: usize) -> bool {
        self.bits.get(index / 64).is_some_and(|w| w & (1 << (index % 64)) != 0)
    }

    /// Returns whether `index` was not marked before.
    pub fn insert(&mut self, index: usize) -> bool {
        let word = index / 64;
        if word >= self.bits.len() {
            self.bits.resize(word + 1, 0);
        }
        let bit = 1 << (index % 64);
        let fresh = self.bits[word] & bit == 0;
        self.bits[word] |= bit;
        fresh
    }

    pub fn union(&mut self, other: &UsedVars) {
        if other.bits.len() > self.bits.len() {
            self.bits.resize(other.bits.len(), 0);
        }
        for (a, b) in self.bits.iter_mut().zip(&other.bits) {
            *a |= b;
        }
    }

    pub fn clear(&mut self) {
        self.bits.clear();
    }
}

const IGNORED: &[(&str, &[&str])] = &[
    ("--font", &["--font-weight", "--font-size"]),
    ("--inset", &["--inset-shadow", "--inset-ring"]),
    (
        "--text",
        &[
            "--text-color",
            "--text-decoration-color",
            "--text-decoration-thickness",
            "--text-indent",
            "--text-shadow",
            "--text-underline-offset",
        ],
    ),
    ("--grid-column", &["--grid-column-start", "--grid-column-end"]),
    ("--grid-row", &["--grid-row-start", "--grid-row-end"]),
];

fn ignored_for(namespace: &str) -> &'static [&'static str] {
    IGNORED.iter().find(|(n, _)| *n == namespace).map_or(&[], |(_, v)| v)
}

pub fn is_ignored_theme_key(key: &str, namespace: &str) -> bool {
    ignored_for(namespace).iter().any(|ignored| key.strip_prefix(ignored).is_some_and(|rest| rest.is_empty() || rest.starts_with('-')))
}

struct KeyBuf {
    stack: [u8; 128],
    len: usize,
    heap: Option<String>,
}

impl KeyBuf {
    fn new(parts: &[&str]) -> Self {
        let total: usize = parts.iter().map(|p| p.len()).sum();
        let mut buf = Self { stack: [0; 128], len: 0, heap: None };
        if total <= buf.stack.len() {
            for p in parts {
                buf.stack[buf.len..buf.len + p.len()].copy_from_slice(p.as_bytes());
                buf.len += p.len();
            }
        } else {
            buf.heap = Some(parts.concat());
        }
        buf
    }

    fn as_str(&self) -> &str {
        match &self.heap {
            Some(s) => s,
            // SAFETY: `new` fills the stack with whole `&str` parts only.
            None => unsafe { std::str::from_utf8_unchecked(&self.stack[..self.len]) },
        }
    }
}

pub struct Theme<'a> {
    prefix: Option<&'a str>,
    values: IndexMap<&'a str, ThemeValue<'a>, FxBuildHasher>,
    keyframes: std::vec::Vec<Node<'a>>,
}

pub type NestedValues<'a, 'n> = smallvec::SmallVec<[(&'n str, &'a str); 4]>;

impl<'a> Theme<'a> {
    pub fn new() -> Self {
        Self { prefix: None, values: IndexMap::default(), keyframes: std::vec::Vec::new() }
    }

    #[inline]
    pub fn prefix(&self) -> Option<&'a str> {
        self.prefix
    }

    pub fn set_prefix(&mut self, alloc: &'a Allocator, prefix: Option<&'a str>) {
        self.prefix = prefix;
        for (key, value) in &mut self.values {
            value.var_ref = var_ref(prefix, key, value.value, value.options, alloc);
        }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn add(&mut self, alloc: &'a Allocator, key: &'a str, value: &'a str, options: u8) -> Result<(), String> {
        if let Some(namespace) = key.strip_suffix("-*") {
            if value != "initial" {
                return Err(format!("Invalid theme value `{value}` for namespace `{key}`"));
            }
            if key == "--*" {
                self.values.clear();
            } else {
                self.clear_namespace(namespace, opts::NONE);
            }
        }
        if options & opts::DEFAULT != 0
            && let Some(existing) = self.values.get(key)
            && existing.options & opts::DEFAULT == 0
        {
            return Ok(());
        }
        if value == "initial" {
            self.values.shift_remove(key);
            return Ok(());
        }
        let entry = ThemeValue { value, options, var_ref: var_ref(self.prefix, key, value, options, alloc) };
        match self.values.get_mut(key) {
            Some(slot) => *slot = entry,
            None => {
                self.values.insert(key, entry);
            }
        }
        Ok(())
    }

    pub fn keys_in_namespaces<'n>(&self, namespaces: impl IntoIterator<Item = &'n str>) -> std::vec::Vec<&'a str> {
        let mut keys = std::vec::Vec::new();
        for namespace in namespaces {
            for &key in self.values.keys() {
                let Some(rest) = key.strip_prefix(namespace).and_then(|r| r.strip_prefix('-')) else {
                    continue;
                };
                if key[2..].contains("--") || is_ignored_theme_key(key, namespace) {
                    continue;
                }
                keys.push(rest);
            }
        }
        keys
    }

    pub fn get(&self, keys: &[&str]) -> Option<&'a str> {
        keys.iter().find_map(|k| self.values.get(*k).map(|v| v.value))
    }

    pub fn get_value(&self, key: &str) -> Option<&ThemeValue<'a>> {
        self.values.get(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    pub fn has_default(&self, key: &str) -> bool {
        self.index_of(key).is_some_and(|i| self.values[i].options & opts::DEFAULT != 0)
    }

    /// Index of the variable `key` names as it appears in CSS, prefix and escapes included.
    pub fn index_of(&self, key: &str) -> Option<usize> {
        match self.prefix {
            Some(p) if !p.is_empty() => {
                let rest = key.strip_prefix("--")?.strip_prefix(p)?.strip_prefix('-')?;
                let k = KeyBuf::new(&["--", rest]);
                self.values.get_index_of(unescape(k.as_str()).as_ref())
            }
            _ => self.values.get_index_of(unescape(key).as_ref()),
        }
    }

    pub fn get_options(&self, key: &str, used: &UsedVars) -> u8 {
        self.index_of(key).map_or(opts::NONE, |i| self.values[i].options | if used.contains(i) { opts::USED } else { 0 })
    }

    /// Returns whether the variable exists and was not marked before.
    pub fn mark_used_variable(&self, key: &str, used: &mut UsedVars) -> bool {
        self.index_of(key).is_some_and(|i| used.insert(i))
    }

    pub fn entries<'s>(&'s self, alloc: &'a Allocator) -> impl Iterator<Item = (&'a str, &'s ThemeValue<'a>)> {
        self.values.iter().map(move |(k, v)| (self.prefix_key(k, alloc), v))
    }

    pub fn prefix_key<'b>(&self, key: &'b str, alloc: &'b Allocator) -> &'b str {
        prefix_key(self.prefix, key, alloc)
    }

    pub fn clear_namespace(&mut self, namespace: &str, clear_options: u8) {
        let ignored = ignored_for(namespace);
        self.values.retain(|key, value| {
            !(key.starts_with(namespace)
                && (clear_options == opts::NONE || value.options & clear_options == clear_options)
                && !ignored.iter().any(|i| key.starts_with(i)))
        });
    }

    fn resolve_key(&self, candidate_value: Option<&str>, keys: &[&str]) -> Option<(&'a str, &ThemeValue<'a>)> {
        for namespace in keys {
            let found = match candidate_value {
                Some(v) => {
                    let k = KeyBuf::new(&[namespace, "-", v]);
                    match self.values.get_key_value(k.as_str()) {
                        Some(hit) => Some(hit),
                        None if v.contains('.') => {
                            let replaced = v.replace('.', "_");
                            let k = KeyBuf::new(&[namespace, "-", &replaced]);
                            self.values.get_key_value(k.as_str())
                        }
                        None => None,
                    }
                }
                None => self.values.get_key_value(*namespace),
            };
            let Some((key, value)) = found else { continue };
            if is_ignored_theme_key(key, namespace) {
                continue;
            }
            return Some((key, value));
        }
        None
    }

    pub fn resolve(&self, candidate_value: Option<&str>, keys: &[&str], inline: Inline) -> Option<&'a str> {
        self.resolve_key(candidate_value, keys).map(|(_, v)| v.resolved(inline))
    }

    pub fn resolve_value(&self, candidate_value: Option<&str>, keys: &[&str]) -> Option<&'a str> {
        self.resolve_key(candidate_value, keys).map(|(_, v)| v.value)
    }

    pub fn resolve_with<'n>(
        &self,
        candidate_value: &str,
        keys: &[&str],
        nested: &[&'n str],
        inline: Inline,
    ) -> Option<(&'a str, NestedValues<'a, 'n>)> {
        let (key, value) = self.resolve_key(Some(candidate_value), keys)?;
        let mut extra = smallvec::SmallVec::new();
        for name in nested {
            let k = KeyBuf::new(&[key, name]);
            let Some(v) = self.values.get(k.as_str()) else {
                continue;
            };
            extra.push((*name, v.resolved(inline)));
        }
        Some((value.resolved(inline), extra))
    }

    pub fn namespace(&self, namespace: &str) -> std::vec::Vec<(Option<&'a str>, &'a str)> {
        let mut values: IndexMap<Option<&'a str>, &'a str, FxBuildHasher> = IndexMap::default();
        for (&key, value) in &self.values {
            if key == namespace {
                values.insert(None, value.value);
            } else if let Some(rest) = key.strip_prefix(namespace).and_then(|r| r.strip_prefix('-')) {
                if rest.starts_with('-') {
                    values.insert(Some(&key[namespace.len()..]), value.value);
                } else {
                    values.insert(Some(rest), value.value);
                }
            }
        }
        values.into_iter().collect()
    }

    pub fn add_keyframes(&mut self, node: Node<'a>) {
        if !self.keyframes.contains(&node) {
            self.keyframes.push(node);
        }
    }

    pub fn keyframes(&self) -> &[Node<'a>] {
        &self.keyframes
    }
}

impl Default for Theme<'_> {
    fn default() -> Self {
        Self::new()
    }
}

fn prefix_key<'a>(prefix: Option<&str>, key: &'a str, alloc: &'a Allocator) -> &'a str {
    match prefix {
        Some(p) if !p.is_empty() => alloc.concat(&["--", p, "-", key.get(2..).unwrap_or("")]),
        _ => key,
    }
}

fn var_ref<'a>(prefix: Option<&str>, key: &'a str, value: &'a str, options: u8, alloc: &'a Allocator) -> &'a str {
    let mut out = ArenaString::new_in(alloc);
    out.push_str("var(");
    escape_into(prefix_key(prefix, key, alloc), &mut out);
    if options & opts::REFERENCE != 0 && !value.is_empty() {
        out.push_str(", ");
        out.push_str(value);
    }
    out.push(')');
    out.into_bump_str()
}
