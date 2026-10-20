//! UnoCSS-style attributify: `<div bg="red-500 hover:blue-500" p-4>` styled through attribute
//! selectors. The scanner hands attributes over as `[name~="value"]` and `[name=""]` candidates.

use orisu_allocator::{AllocExt, Allocator};
use orisu_ast::Node;
use orisu_parser::escape::escape;
use orisu_parser::segment::segment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attributify<'a> {
    pub prefix: &'a str,
    pub prefixed_only: bool,
}

impl<'a> Attributify<'a> {
    /// Parses the params of `@attributify`: `prefix(<name>)` and `prefixed-only`, in any order.
    pub fn parse(params: &'a str) -> Result<Self, String> {
        let mut out = Self { prefix: "un-", prefixed_only: false };
        for param in params.split_ascii_whitespace() {
            if let Some(prefix) = param.strip_prefix("prefix(").and_then(|p| p.strip_suffix(')')) {
                out.prefix = prefix.trim_matches(['"', '\'']);
            } else if param == "prefixed-only" {
                out.prefixed_only = true;
            } else {
                return Err(format!("Unknown `@attributify` option `{param}`."));
            }
        }
        Ok(out)
    }

    /// The utility an attribute candidate stands for, and the selector matching the attribute:
    /// `[hover:text~="red-500"]` is `hover:text-red-500` on `[hover\:text~="red-500"]`.
    pub fn resolve(&self, raw: &str) -> Option<(String, String)> {
        let inner = raw.strip_prefix('[')?.strip_suffix(']')?;
        let (name, value) = match inner.strip_suffix("=\"\"") {
            Some(name) => (name, None),
            None => {
                let (name, value) = inner.split_once("~=\"")?;
                (name, Some(value.strip_suffix('"')?))
            }
        };
        if name.is_empty() || name.contains(['"', '=', '~']) || value.is_some_and(|v| v.is_empty() || v.contains('"')) {
            return None;
        }
        let bare = match name.strip_prefix(self.prefix) {
            Some(bare) if !self.prefix.is_empty() => bare,
            _ if self.prefixed_only => return None,
            _ => name,
        };
        let utility = match value {
            None => bare.to_owned(),
            Some(value) => {
                let (name_variants, root) = split_variants(bare);
                let (value_variants, value) = split_variants(value);
                let (important, value) = match value.strip_prefix('!') {
                    Some(value) => ("!", value),
                    None => ("", value),
                };
                if value == "~" {
                    format!("{name_variants}{value_variants}{important}{root}")
                } else {
                    format!("{name_variants}{value_variants}{important}{root}-{value}")
                }
            }
        };
        let name = escape(name);
        let selector = match value {
            None => format!("[{name}=\"\"]"),
            Some(value) => format!("[{name}~=\"{value}\"]"),
        };
        Some((utility, selector))
    }
}

/// Splits `hover:focus:bg` into `hover:focus:` and `bg`.
fn split_variants(input: &str) -> (&str, &str) {
    let base = segment(input, b':').last().copied().unwrap_or(input);
    input.split_at(input.len() - base.len())
}

/// Swaps the class selector `class` for `selector` in every rule under `node`.
pub(crate) fn retarget<'a>(node: &mut Node<'a>, class: &str, selector: &str, alloc: &'a Allocator) {
    if let Node::Rule { selector: current, .. } = node
        && let Some(replaced) = replace_class(current, class, selector)
    {
        *current = alloc.str(&replaced);
    }
    if let Some(children) = node.children_mut() {
        for child in children.iter_mut() {
            retarget(child, class, selector, alloc);
        }
    }
}

fn replace_class(input: &str, class: &str, selector: &str) -> Option<String> {
    let mut out = String::new();
    let mut copied = 0;
    for (at, _) in input.match_indices(class) {
        let end = at + class.len();
        if input.as_bytes().get(end).is_some_and(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'\\') || b >= 0x80) {
            continue;
        }
        out.push_str(&input[copied..at]);
        out.push_str(selector);
        copied = end;
    }
    if copied == 0 {
        return None;
    }
    out.push_str(&input[copied..]);
    Some(out)
}
