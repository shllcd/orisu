//! Compiles the attributify candidates of `orisu_candidate::attributify` to rules on attribute selectors.

use orisu_allocator::{AllocExt, Allocator};
use orisu_ast::Node;
use orisu_candidate::attributify::{Attributify, decode, encode};
use orisu_parser::escape::escape;
use orisu_parser::segment::segment;

/// The utility an attribute candidate stands for, and the selector matching the attribute:
/// `[hover:text~="red-500"]` is `hover:text-red-500` on `[hover\:text~="red-500"]`.
pub(crate) fn resolve(attributify: &Attributify, raw: &str) -> Option<(String, String)> {
    let (name, value) = decode(raw)?;
    let bare = match name.strip_prefix(attributify.prefix.as_str()) {
        Some(bare) if !attributify.prefix.is_empty() => bare,
        _ if attributify.prefixed_only => return None,
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
    let selector = encode(&escape(name), value);
    Some((utility, selector))
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
