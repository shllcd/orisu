// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeSelector<'a> {
    pub attribute: &'a str,
    pub operator: Option<&'a str>,
    pub quote: Option<char>,
    pub value: Option<&'a str>,
    pub sensitivity: Option<char>,
}

#[inline]
fn is_ws(b: Option<u8>) -> bool {
    matches!(b, Some(b' ' | b'\t' | b'\n' | b'\r'))
}

/// Whether `input` is a single well-formed `[attr op value flag]` selector.
pub fn is_valid_attribute_selector(input: &str) -> bool {
    parse(input).is_some()
}

pub fn parse(input: &str) -> Option<AttributeSelector<'_>> {
    let b = input.as_bytes();
    if b.first() != Some(&b'[') || b.last() != Some(&b']') {
        return None;
    }
    let at = |i: usize| b.get(i).copied();
    let end = b.len() - 1;
    let mut i = 1;
    while is_ws(at(i)) {
        i += 1;
    }
    let start = i;
    while i < end {
        let c = b[i];
        if c == b'\\' {
            i += 2;
            continue;
        }
        if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c >= 0x80 {
            i += 1;
            continue;
        }
        break;
    }
    if start == i {
        return None;
    }
    let attribute = input.get(start..i.min(end))?;
    while is_ws(at(i)) {
        i += 1;
    }
    if i == end {
        return Some(AttributeSelector { attribute, operator: None, quote: None, value: None, sensitivity: None });
    }
    let op_start = i;
    match at(i) {
        Some(b'=') => i += 1,
        Some(b'~' | b'|' | b'^' | b'$' | b'*') if at(i + 1) == Some(b'=') => i += 2,
        _ => return None,
    }
    let operator = &input[op_start..i];
    while is_ws(at(i)) {
        i += 1;
    }
    if i == end {
        return None;
    }
    let (quote, value) = match at(i) {
        Some(quote @ (b'\'' | b'"')) => {
            i += 1;
            let start = i;
            let mut j = i;
            while j < end {
                if b[j] == quote {
                    i = j + 1;
                } else if b[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            (Some(quote as char), input.get(start..(i - 1).max(start)).unwrap_or(""))
        }
        _ => {
            let start = i;
            while i < end && !is_ws(at(i)) {
                i += 1;
            }
            (None, &input[start..i])
        }
    };
    while is_ws(at(i)) {
        i += 1;
    }
    let mut selector = AttributeSelector { attribute, operator: Some(operator), quote, value: Some(value), sensitivity: None };
    if i == end {
        return Some(selector);
    }
    selector.sensitivity = match at(i) {
        Some(b'i' | b'I') => Some('i'),
        Some(b's' | b'S') => Some('s'),
        _ => return None,
    };
    i += 1;
    while is_ws(at(i)) {
        i += 1;
    }
    (i == end).then_some(selector)
}
