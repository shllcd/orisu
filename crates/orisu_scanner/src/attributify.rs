//! Finds the attributes of markup tags and encodes the ones attributify accepts as candidates.

use bstr::ByteSlice;
use orisu_candidate::attributify::{Attribute, Attributify, Value, attributes, encode};
use rustc_hash::FxHashSet;

const MAX_VALUE_LEN: usize = 128;

pub(crate) fn extract(attributify: &Attributify, input: &[u8], out: &mut FxHashSet<String>) {
    for Attribute { name, value } in attributes(input) {
        let value = match value {
            None => None,
            Some(Value::Quoted(range) | Value::Bare(range)) => Some(&input[range]),
            Some(Value::Expression(range)) => match string_expression(&input[range]) {
                Some(value) => Some(value),
                None => continue,
            },
        };
        emit(attributify, &input[name], value, out);
    }
}

fn emit(attributify: &Attributify, name: &[u8], value: Option<&[u8]>, out: &mut FxHashSet<String>) {
    if !attributify.accepts(name) {
        return;
    }
    let Ok(name) = std::str::from_utf8(name) else {
        return;
    };
    match value {
        None => {
            out.insert(encode(name, None));
        }
        Some(value) => {
            for item in value.split(|b| b.is_ascii_whitespace()) {
                if item.is_empty() || item.len() > MAX_VALUE_LEN || item.contains(&b'"') {
                    continue;
                }
                if let Ok(item) = std::str::from_utf8(item) {
                    out.insert(encode(name, Some(item)));
                }
            }
        }
    }
}

/// The text of a JSX expression that is a single string literal, like `{"red-500"}`.
fn string_expression(expression: &[u8]) -> Option<&[u8]> {
    let expression = expression.trim();
    let quote = *expression.first()?;
    if !matches!(quote, b'"' | b'\'' | b'`') || expression.len() < 2 || expression.last() != Some(&quote) {
        return None;
    }
    let inner = &expression[1..expression.len() - 1];
    (!inner.contains(&quote) && !inner.contains_str("${")).then_some(inner)
}
