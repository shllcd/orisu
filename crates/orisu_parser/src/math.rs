// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use orisu_allocator::{Allocator, Vec};

const MATH_FUNCTIONS: [&str; 19] = [
    "calc", "min", "max", "clamp", "mod", "rem", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "pow", "sqrt", "hypot", "log",
    "exp", "round",
];

pub fn has_math_fn(input: &str) -> bool {
    if !input.contains('(') {
        return false;
    }
    let bytes = input.as_bytes();
    memchr::memchr_iter(b'(', bytes).any(|paren| MATH_FUNCTIONS.iter().any(|f| bytes[..paren].ends_with(f.as_bytes())))
}

pub fn is_math_fn(name: &str) -> bool {
    MATH_FUNCTIONS.contains(&name)
}

fn is_op(b: u8) -> bool {
    matches!(b, b'+' | b'*' | b'/' | b'-')
}

pub fn add_whitespace_around_math_operators<'a>(input: &'a str, alloc: &'a Allocator) -> &'a str {
    if !MATH_FUNCTIONS.iter().any(|f| input.contains(f)) {
        return input;
    }
    let bytes = input.as_bytes();
    let mut result = Vec::with_capacity_in(bytes.len() + 16, alloc);
    let mut formattable: smallvec::SmallVec<[bool; 16]> = smallvec::SmallVec::new();
    let mut value_pos: Option<usize> = None;
    let mut last_value_pos: Option<usize> = None;

    for i in 0..bytes.len() {
        let ch = bytes[i];
        if ch.is_ascii_digit() || (value_pos.is_some() && (ch == b'%' || ch.is_ascii_alphabetic())) {
            value_pos = Some(i);
        } else {
            last_value_pos = value_pos;
            value_pos = None;
        }

        let top = formattable.last().copied().unwrap_or(false);
        match ch {
            b'(' => {
                result.push(ch);
                let mut start = i;
                while start > 0 && matches!(bytes[start - 1], b'0'..=b'9' | b'a'..=b'z') {
                    start -= 1;
                }
                let name = &input[start..i];
                formattable.push(is_math_fn(name) || (top && name.is_empty()));
            }
            b')' => {
                result.push(ch);
                formattable.pop();
            }
            b',' if top => result.extend_from_slice(b", "),
            b' ' if top && result.last() == Some(&b' ') => {}
            _ if is_op(ch) && top => {
                let mut end = result.len();
                while end > 0 && matches!(result[end - 1], b' ' | b'\t' | b'\n' | b'\r' | 0x0c) {
                    end -= 1;
                }
                let prev = end.checked_sub(1).map(|p| result[p]);
                let prev_prev = end.checked_sub(2).map(|p| result[p]);
                let next = bytes.get(i + 1).copied();
                let next_digit = next.is_some_and(|n| n.is_ascii_digit());
                if (matches!(prev, Some(b'e' | b'E')) && prev_prev.is_some_and(|p| p.is_ascii_digit()))
                    || prev.is_some_and(is_op)
                    || matches!(prev, Some(b'(' | b','))
                {
                    result.push(ch);
                } else if i > 0 && bytes[i - 1] == b' ' {
                    result.push(ch);
                    result.push(b' ');
                } else if prev.is_some_and(|p| p.is_ascii_digit())
                    || next_digit
                    || prev == Some(b')')
                    || next == Some(b'(')
                    || next.is_some_and(is_op)
                    || (i > 0 && last_value_pos == Some(i - 1))
                {
                    result.push(b' ');
                    result.push(ch);
                    result.push(b' ');
                } else {
                    result.push(ch);
                }
            }
            _ => result.push(ch),
        }
    }
    std::str::from_utf8(result.into_bump_slice()).unwrap_or(input)
}
