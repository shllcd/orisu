use oxc_sourcemap::SourceMapBuilder;
use std::ops::Range;

/// Starts every placeholder; a module that already contains it cannot be rewritten.
pub const MARKER: &str = "__orisu-";

/// The placeholder of `class` in a module: the marker, the class's UTF-8 in hex, `__`. The `-` keeps a minifier from
/// turning a quoted key that holds it into an identifier.
pub fn placeholder(class: &str) -> String {
    let mut out = String::with_capacity(MARKER.len() + class.len() * 2 + 2);
    out.push_str(MARKER);
    for b in class.bytes() {
        out.push(char::from_digit((b >> 4) as u32, 16).expect("a nibble"));
        out.push(char::from_digit((b & 15) as u32, 16).expect("a nibble"));
    }
    out.push_str("__");
    out
}

/// The placeholders in `code` with the class each one holds.
pub fn placeholders(code: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = code[from..].find(MARKER) {
        let start = from + at;
        let hex_start = start + MARKER.len();
        let hex_len = code[hex_start..].bytes().take_while(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')).count();
        let end = hex_start + hex_len + 2;
        from = hex_start;
        if hex_len == 0 || hex_len % 2 == 1 || code.get(hex_start + hex_len..end) != Some("__") {
            continue;
        }
        let bytes = (hex_start..hex_start + hex_len)
            .step_by(2)
            .map(|i| u8::from_str_radix(&code[i..i + 2], 16).expect("hex digits"))
            .collect::<Vec<u8>>();
        if let Ok(class) = String::from_utf8(bytes) {
            out.push((start..end, class));
            from = end;
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Space,
    Word,
    Punct,
    Edit,
}

fn kind(c: char) -> Kind {
    if c.is_whitespace() {
        Kind::Space
    } else if c.is_alphanumeric() || c == '_' || c == '$' {
        Kind::Word
    } else {
        Kind::Punct
    }
}

/// `source` with each `(range, text)` of `edits` (sorted, disjoint, on one line each) replaced, and a JSON source map
/// from the result back to `source`.
pub fn apply(source: &str, name: &str, edits: &[(Range<usize>, String)]) -> (String, String) {
    let mut builder = SourceMapBuilder::default();
    let id = builder.set_source_and_content(name, source);
    let mut out = String::with_capacity(source.len() + edits.iter().map(|(_, text)| text.len()).sum::<usize>());
    let (mut line, mut col, mut out_col) = (0u32, 0u32, 0u32);
    let mut previous = Kind::Space;
    let mut edits = edits.iter().peekable();
    let mut i = 0;
    while i < source.len() {
        if let Some((range, text)) = edits.next_if(|(range, _)| range.start == i) {
            builder.add_token(line, out_col, line, col, Some(id), None);
            out.push_str(text);
            out_col += text.encode_utf16().count() as u32;
            col += source[range.clone()].encode_utf16().count() as u32;
            i = range.end;
            previous = Kind::Edit;
            continue;
        }
        let c = source[i..].chars().next().expect("a character");
        let current = kind(c);
        if current != Kind::Space && current != previous {
            builder.add_token(line, out_col, line, col, Some(id), None);
        }
        previous = current;
        out.push(c);
        i += c.len_utf8();
        if c == '\n' {
            line += 1;
            col = 0;
            out_col = 0;
        } else {
            col += c.len_utf16() as u32;
            out_col += c.len_utf16() as u32;
        }
    }
    (out, builder.into_sourcemap().to_json_string())
}
