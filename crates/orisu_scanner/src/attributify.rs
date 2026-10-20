//! Extracts UnoCSS-style attributify candidates from markup: `<div bg="red-500 hover:blue-500" p-4>`
//! yields `[bg~="red-500"]`, `[bg~="hover:blue-500"]` and `[p-4=""]`.

use bstr::ByteSlice;
use rustc_hash::FxHashSet;

const IGNORED: &[&[u8]] = &[
    b"class",
    b"classname",
    b"style",
    b"id",
    b"key",
    b"ref",
    b"href",
    b"src",
    b"srcset",
    b"alt",
    b"title",
    b"content",
    b"d",
    b"xmlns",
    b"viewbox",
    b"for",
    b"type",
    b"name",
    b"value",
    b"placeholder",
    b"lang",
    b"role",
];

const MAX_VALUE_LEN: usize = 128;

#[derive(Debug, Clone, Default)]
pub struct Attributify {
    pub prefix: String,
    pub prefixed_only: bool,
}

impl Attributify {
    pub fn extract(&self, input: &[u8], out: &mut FxHashSet<String>) {
        let mut i = 0;
        while let Some(at) = input[i..].find_byte(b'<') {
            i += at + 1;
            if !input.get(i).is_some_and(u8::is_ascii_alphabetic) {
                continue;
            }
            while input.get(i).is_some_and(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b':' | b'_')) {
                i += 1;
            }
            i = self.attributes(input, i, out);
        }
    }

    fn attributes(&self, input: &[u8], mut i: usize, out: &mut FxHashSet<String>) -> usize {
        loop {
            while input.get(i).is_some_and(u8::is_ascii_whitespace) {
                i += 1;
            }
            match input.get(i) {
                None | Some(b'>' | b'<') => return i,
                Some(b'/') if input.get(i + 1) == Some(&b'>') => return i,
                Some(b'{') => {
                    i = skip_braces(input, i);
                    continue;
                }
                Some(b'"' | b'\'' | b'=' | b'}') => {
                    i += 1;
                    continue;
                }
                _ => {}
            }
            let start = i;
            while input.get(i).is_some_and(|&b| !is_name_end(b) && !(b == b'/' && input.get(i + 1) == Some(&b'>'))) {
                i += 1;
            }
            let name = &input[start..i];
            while input.get(i).is_some_and(u8::is_ascii_whitespace) {
                i += 1;
            }
            if input.get(i) != Some(&b'=') {
                self.emit(name, None, out);
                continue;
            }
            i += 1;
            while input.get(i).is_some_and(u8::is_ascii_whitespace) {
                i += 1;
            }
            let value = match input.get(i) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let end = input[i + 1..].find_byte(quote).map_or(input.len(), |n| i + 1 + n);
                    let value = &input[i + 1..end];
                    i = end + 1;
                    Some(value)
                }
                Some(b'{') => {
                    let end = skip_braces(input, i);
                    let value = string_expression(&input[i + 1..end.saturating_sub(1).max(i + 1)]);
                    i = end;
                    value
                }
                _ => {
                    let start = i;
                    while input.get(i).is_some_and(|&b| !b.is_ascii_whitespace() && b != b'>') {
                        i += 1;
                    }
                    Some(&input[start..i])
                }
            };
            if let Some(value) = value {
                self.emit(name, Some(value), out);
            }
        }
    }

    /// Whether an attribute named `name` can hold utilities.
    pub fn accepts(&self, name: &[u8]) -> bool {
        name.first().is_some_and(|&b| b.is_ascii_alphabetic() || matches!(b, b'[' | b'!' | b'-' | b'*'))
            && !name.starts_with(b"v-")
            && !name.starts_with(b"data-")
            && !name.starts_with(b"aria-")
            && (!self.prefixed_only || name.starts_with(self.prefix.as_bytes()))
            && !IGNORED.iter().any(|ignored| name.eq_ignore_ascii_case(ignored))
    }

    fn emit(&self, name: &[u8], value: Option<&[u8]>, out: &mut FxHashSet<String>) {
        if !self.accepts(name) {
            return;
        }
        let Ok(name) = std::str::from_utf8(name) else {
            return;
        };
        match value {
            None => {
                out.insert(format!("[{name}=\"\"]"));
            }
            Some(value) => {
                for item in value.split(|b| b.is_ascii_whitespace()) {
                    if item.is_empty() || item.len() > MAX_VALUE_LEN || item.contains(&b'"') {
                        continue;
                    }
                    if let Ok(item) = std::str::from_utf8(item) {
                        out.insert(format!("[{name}~=\"{item}\"]"));
                    }
                }
            }
        }
    }
}

fn is_name_end(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b'=' | b'>' | b'<' | b'"' | b'\'' | b'{' | b'}')
}

/// The index after the `}` balancing the `{` at `open`.
fn skip_braces(input: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, &b) in input.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    input.len()
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
