//! UnoCSS-style variant groups: `hover:(bg-red-500 focus:(underline ring))` expands to
//! `hover:bg-red-500 hover:focus:underline hover:focus:ring`.
//!
//! Only `:` opens a group, since `-(…)` already means a CSS variable in Tailwind v4 (`p-(--gap)`).

use memchr::{memchr, memmem};

/// Expands every group in `input`, wherever it appears. Meant for candidate extraction, where
/// expanding something that only looks like a group is harmless.
pub fn expand(input: &[u8]) -> Option<Vec<u8>> {
    if !has_group(input) {
        return None;
    }
    let mut out = Vec::with_capacity(input.len());
    expand_into(input, &mut out).then_some(out)
}

/// [`expand`] for text.
pub fn expand_str(input: &str) -> Option<String> {
    // SAFETY: the input is UTF-8 and is only split around ASCII bytes.
    expand(input.as_bytes()).map(|out| unsafe { String::from_utf8_unchecked(out) })
}

/// Expands groups only inside string literals (`'…'`, `"…"` and the text of `` `…` `` templates),
/// leaving code, comments and markup outside quotes untouched. Meant for rewriting source files.
pub fn expand_in_strings(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    if !has_group(bytes) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut changed = false;
    let mut copied = 0;
    let mut templates: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'\'' | b'"') => {
                let end = string_end(bytes, i + 1, quote);
                changed |= expand_region(bytes, i + 1..end, &mut copied, &mut out);
                i = end + 1;
            }
            b'`' => i = template_text(bytes, i + 1, &mut templates, &mut copied, &mut out, &mut changed),
            b'}' if templates.last() == Some(&0) => {
                templates.pop();
                i = template_text(bytes, i + 1, &mut templates, &mut copied, &mut out, &mut changed);
            }
            b'{' if !templates.is_empty() => {
                *templates.last_mut().unwrap() += 1;
                i += 1;
            }
            b'}' if !templates.is_empty() => {
                *templates.last_mut().unwrap() -= 1;
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = memchr(b'\n', &bytes[i..]).map_or(bytes.len(), |n| i + n),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = memmem::find(&bytes[i + 2..], b"*/").map_or(bytes.len(), |n| i + n + 4),
            _ => i += 1,
        }
    }
    if !changed {
        return None;
    }
    out.extend_from_slice(&bytes[copied..]);
    // SAFETY: the input is UTF-8 and is only split around ASCII bytes.
    Some(unsafe { String::from_utf8_unchecked(out) })
}

/// Scans template text from `start` up to the closing backtick or a `${`, expanding it.
fn template_text(
    bytes: &[u8],
    start: usize,
    templates: &mut Vec<usize>,
    copied: &mut usize,
    out: &mut Vec<u8>,
    changed: &mut bool,
) -> usize {
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => {
                *changed |= expand_region(bytes, start..i, copied, out);
                return i + 1;
            }
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                *changed |= expand_region(bytes, start..i, copied, out);
                templates.push(0);
                return i + 2;
            }
            _ => i += 1,
        }
    }
    *changed |= expand_region(bytes, start..bytes.len(), copied, out);
    bytes.len()
}

fn string_end(bytes: &[u8], mut i: usize, quote: u8) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote => return i,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn expand_region(bytes: &[u8], range: std::ops::Range<usize>, copied: &mut usize, out: &mut Vec<u8>) -> bool {
    let region = &bytes[range.clone()];
    if !has_group(region) {
        return false;
    }
    let mark = out.len();
    out.extend_from_slice(&bytes[*copied..range.start]);
    if expand_into(region, out) {
        *copied = range.end;
        true
    } else {
        out.truncate(mark);
        false
    }
}

/// Appends `input` to `out` with groups expanded; returns whether any group was found. On `false`
/// `out` is left unchanged.
fn expand_into(input: &[u8], out: &mut Vec<u8>) -> bool {
    let mark = out.len();
    let mut changed = false;
    let mut classes = Vec::new();
    let mut i = 0;
    while i < input.len() {
        if (i == 0 || is_boundary(input[i - 1])) && !is_boundary(input[i]) {
            classes.clear();
            if let Some(end) = token(input, i, &mut classes)
                && (classes.len() != 1 || classes[0] != input[i..end])
            {
                for (n, class) in classes.iter().enumerate() {
                    if n > 0 {
                        out.push(b' ');
                    }
                    out.extend_from_slice(class);
                }
                changed = true;
                i = end;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    if !changed {
        out.truncate(mark);
    }
    changed
}

/// Parses one class token at `start`, pushing the classes it stands for into `classes` and
/// returning where it ends.
fn token(input: &[u8], start: usize, classes: &mut Vec<Vec<u8>>) -> Option<usize> {
    let mut i = start;
    while i < input.len() {
        match input[i] {
            b if is_boundary(b) || b == b')' => break,
            b'[' => i = close(input, i)? + 1,
            b'(' if i > start && input[i - 1] == b':' => {
                let prefix = &input[start..i];
                let mut inner = Vec::new();
                let end = group(input, i, &mut inner)?;
                if input.get(end).is_some_and(|&b| !is_boundary(b) && b != b')') || arrow_follows(&input[end..]) {
                    return None;
                }
                classes.extend(inner.into_iter().map(|class| [prefix, &class].concat()));
                return Some(end);
            }
            b'(' => i = close(input, i)? + 1,
            _ => i += 1,
        }
    }
    if i == start {
        return None;
    }
    classes.push(input[start..i].to_vec());
    Some(i)
}

/// Parses the group opened at `open`, returning the index after its `)`.
fn group(input: &[u8], open: usize, classes: &mut Vec<Vec<u8>>) -> Option<usize> {
    let mut i = open + 1;
    loop {
        while input.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        match input.get(i)? {
            b')' if classes.is_empty() => return None,
            b')' => return Some(i + 1),
            &b if is_boundary(b) => return None,
            _ => i = token(input, i, classes)?,
        }
    }
}

/// Index of the bracket closing the one at `open`, skipping nested `[]` and `()`.
fn close(input: &[u8], open: usize) -> Option<usize> {
    let mut stack: Vec<u8> = Vec::new();
    for (i, &b) in input.iter().enumerate().skip(open) {
        match b {
            b'[' => stack.push(b']'),
            b'(' => stack.push(b')'),
            b']' | b')' => {
                if stack.pop()? != b {
                    return None;
                }
                if stack.is_empty() {
                    return Some(i);
                }
            }
            b if b.is_ascii_whitespace() || matches!(b, b'"' | b'\'' | b'`') => return None,
            _ => {}
        }
    }
    None
}

fn has_group(input: &[u8]) -> bool {
    memmem::find(input, b":(").is_some()
}

fn arrow_follows(rest: &[u8]) -> bool {
    rest.trim_ascii_start().starts_with(b"=>")
}

fn is_boundary(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b'"' | b'\'' | b'`' | b';' | b',' | b'{' | b'}' | b'<' | b'>' | b'=')
}
