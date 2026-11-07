//! UnoCSS-style attributify: `<div bg="red-500 hover:blue-500" p-4>` styled through attribute selectors. The scanner
//! hands attributes to the compiler as candidates in the form of the selector matching them: `[bg~="red-500"]` for a
//! value, `[p-4=""]` for a bare attribute.

use std::ops::Range;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributify {
    pub prefix: String,
    pub prefixed_only: bool,
}

impl Attributify {
    /// Parses the params of `@attributify`: `prefix(<name>)` and `prefixed-only`, in any order.
    pub fn parse(params: &str) -> Result<Self, String> {
        let mut out = Self { prefix: "un-".to_owned(), prefixed_only: false };
        for param in params.split_ascii_whitespace() {
            if let Some(prefix) = param.strip_prefix("prefix(").and_then(|p| p.strip_suffix(')')) {
                out.prefix = prefix.trim_matches(['"', '\'']).to_owned();
            } else if param == "prefixed-only" {
                out.prefixed_only = true;
            } else {
                return Err(format!("Unknown `@attributify` option `{param}`."));
            }
        }
        Ok(out)
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
}

/// The candidate for the attribute `name`, holding `value` among its whitespace-separated items or bare.
pub fn encode(name: &str, value: Option<&str>) -> String {
    match value {
        None => format!("[{name}=\"\"]"),
        Some(value) => format!("[{name}~=\"{value}\"]"),
    }
}

/// The attribute name and value of a candidate made by [`encode`], unless the name or value could not have come from it.
pub fn decode(raw: &str) -> Option<(&str, Option<&str>)> {
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
    Some((name, value))
}

/// The value of a markup attribute, as byte ranges into the scanned text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// The text between the quotes of `name="…"`.
    Quoted(Range<usize>),
    /// An unquoted `name=…`.
    Bare(Range<usize>),
    /// The text inside the braces of `name={…}`.
    Expression(Range<usize>),
}

/// An attribute of a markup tag: its name and, after an `=`, its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: Range<usize>,
    pub value: Option<Value>,
}

/// The attributes of every markup tag in `input`, in order. Brace blocks between attributes, like JSX spreads, are skipped.
pub fn attributes(input: &[u8]) -> Attributes<'_> {
    Attributes { input, i: 0, in_tag: false }
}

pub struct Attributes<'a> {
    input: &'a [u8],
    i: usize,
    in_tag: bool,
}

impl Iterator for Attributes<'_> {
    type Item = Attribute;

    fn next(&mut self) -> Option<Attribute> {
        let input = self.input;
        loop {
            if !self.in_tag {
                self.i += memchr::memchr(b'<', input.get(self.i..)?)? + 1;
                if !input.get(self.i).is_some_and(u8::is_ascii_alphabetic) {
                    continue;
                }
                while input.get(self.i).is_some_and(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b':' | b'_')) {
                    self.i += 1;
                }
                self.in_tag = true;
            }
            if let Some(attribute) = self.attribute() {
                return Some(attribute);
            }
            self.in_tag = false;
        }
    }
}

impl Attributes<'_> {
    /// The next attribute of the current tag, `None` at its end.
    fn attribute(&mut self) -> Option<Attribute> {
        let input = self.input;
        let mut i = self.i;
        let attribute = loop {
            i = skip_whitespace(input, i);
            match input.get(i) {
                None | Some(b'>' | b'<') => break None,
                Some(b'/') if input.get(i + 1) == Some(&b'>') => break None,
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
            let name = start..i;
            i = skip_whitespace(input, i);
            if input.get(i) != Some(&b'=') {
                break Some(Attribute { name, value: None });
            }
            i = skip_whitespace(input, i + 1);
            let value = match input.get(i) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let end = memchr::memchr(quote, &input[i + 1..]).map_or(input.len(), |n| i + 1 + n);
                    let value = Value::Quoted(i + 1..end);
                    i = (end + 1).min(input.len());
                    value
                }
                Some(b'{') => {
                    let end = skip_braces(input, i);
                    let value = Value::Expression(i + 1..end.saturating_sub(1).max(i + 1));
                    i = end;
                    value
                }
                _ => {
                    let start = i;
                    while input.get(i).is_some_and(|&b| !b.is_ascii_whitespace() && b != b'>') {
                        i += 1;
                    }
                    Value::Bare(start..i)
                }
            };
            break Some(Attribute { name, value: Some(value) });
        };
        self.i = i;
        attribute
    }
}

fn skip_whitespace(input: &[u8], mut i: usize) -> usize {
    while input.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
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
