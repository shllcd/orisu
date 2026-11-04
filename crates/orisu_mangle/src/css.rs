use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashSet};
use std::ops::Range;

/// Calls `found` with the source range and unescaped name of every class in the selectors of `css`: in style rule
/// preludes and in `@scope` preludes, never in declarations or other at-rule preludes.
fn selector_classes(css: &str, mut found: impl FnMut(Range<usize>, String)) {
    let bytes = css.as_bytes();
    let mut pending: Vec<(Range<usize>, String)> = Vec::new();
    let mut statement_start = true;
    let mut at_rule = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i = css[i + 2..].find("*/").map_or(bytes.len(), |end| i + 2 + end + 2);
            continue;
        }
        if statement_start && !b.is_ascii_whitespace() {
            statement_start = false;
            at_rule = b == b'@' && !css[i..].starts_with("@scope");
        }
        match b {
            b'"' | b'\'' => {
                i = skip_string(bytes, i);
                continue;
            }
            b'\\' => {
                i = (i + 2).min(bytes.len());
                continue;
            }
            b'(' if css[..i].ends_with("url") && !is_ident_byte(bytes.get(i.wrapping_sub(4)).copied().unwrap_or(b' ')) => {
                i = skip_url(bytes, i + 1);
                continue;
            }
            b'{' => {
                if !at_rule {
                    for (range, name) in pending.drain(..) {
                        found(range, name);
                    }
                }
                pending.clear();
                statement_start = true;
            }
            b';' | b'}' => {
                pending.clear();
                statement_start = true;
            }
            b'.' if starts_ident(&bytes[i + 1..]) => {
                let (end, name) = read_ident(css, i + 1);
                if !at_rule {
                    pending.push((i + 1..end, name));
                }
                i = end;
                continue;
            }
            _ if is_ident_byte(b) => {
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
}

/// The classes the selectors of `css` match.
pub fn classes(css: &str) -> FxHashSet<String> {
    let mut out = FxHashSet::default();
    selector_classes(css, |_, name| {
        out.insert(name);
    });
    out
}

/// `css` with every selector class in `names` replaced by its new name.
pub fn rename(css: &str, names: &IndexMap<String, String, FxBuildHasher>) -> String {
    let mut out = String::with_capacity(css.len());
    let mut last = 0;
    selector_classes(css, |range, name| {
        if let Some(short) = names.get(&name) {
            out.push_str(&css[last..range.start]);
            out.push_str(short);
            last = range.end;
        }
    });
    out.push_str(&css[last..]);
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b >= 0x80
}

fn starts_ident(rest: &[u8]) -> bool {
    let name_start = |b: Option<&u8>| matches!(b, Some(&b) if b.is_ascii_alphabetic() || b == b'_' || b >= 0x80);
    let escape = |at: usize| rest.get(at) == Some(&b'\\') && !matches!(rest.get(at + 1), None | Some(b'\n' | b'\r' | b'\x0c'));
    match rest.first() {
        Some(b'-') => name_start(rest.get(1)) || rest.get(1) == Some(&b'-') || escape(1),
        Some(b'\\') => escape(0),
        first => name_start(first),
    }
}

fn read_ident(css: &str, start: usize) -> (usize, String) {
    let bytes = css.as_bytes();
    let mut name = String::new();
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            if matches!(bytes.get(i + 1), None | Some(b'\n' | b'\r' | b'\x0c')) {
                break;
            }
            let hex = bytes[i + 1..].iter().take(6).take_while(|b| b.is_ascii_hexdigit()).count();
            if hex > 0 {
                let code = u32::from_str_radix(&css[i + 1..i + 1 + hex], 16).expect("hex digits");
                name.push(char::from_u32(code).filter(|&c| c != '\0').unwrap_or('\u{fffd}'));
                i += 1 + hex;
                if bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                }
            } else {
                let c = css[i + 1..].chars().next().expect("an escaped character");
                name.push(c);
                i += 1 + c.len_utf8();
            }
        } else if is_ident_byte(b) {
            let c = css[i..].chars().next().expect("a character");
            name.push(c);
            i += c.len_utf8();
        } else {
            break;
        }
    }
    (i, name)
}

fn skip_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote || b == b'\n' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_url(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if matches!(bytes.get(i), Some(b'"' | b'\'')) {
        return i;
    }
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b')' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}
