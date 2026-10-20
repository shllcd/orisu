// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use smallvec::SmallVec;

pub type Segments<'a> = SmallVec<[&'a str; 8]>;

pub fn segment(input: &str, separator: u8) -> Segments<'_> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut stack: SmallVec<[u8; 16]> = SmallVec::new();
    let mut parts = Segments::new();
    let mut last = 0;
    let mut idx = 0;
    while idx < len {
        let ch = bytes[idx];
        if stack.is_empty() && ch == separator {
            parts.push(&input[last..idx]);
            last = idx + 1;
            idx += 1;
            continue;
        }
        match ch {
            b'\\' => idx += 1,
            b'\'' | b'"' => {
                idx += 1;
                while idx < len {
                    let next = bytes[idx];
                    if next == b'\\' {
                        idx += 2;
                        continue;
                    }
                    if next == ch {
                        break;
                    }
                    idx += 1;
                }
            }
            b'(' => stack.push(b')'),
            b'[' => stack.push(b']'),
            b'{' => stack.push(b'}'),
            b')' | b']' | b'}' if stack.last() == Some(&ch) => {
                stack.pop();
            }
            _ => {}
        }
        idx += 1;
    }
    parts.push(&input[last.min(len)..]);
    parts
}

pub fn is_valid_arbitrary(input: &str) -> bool {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut stack: SmallVec<[u8; 16]> = SmallVec::new();
    let mut idx = 0;
    while idx < len {
        let ch = bytes[idx];
        match ch {
            b'\\' => idx += 1,
            b'\'' | b'"' => {
                idx += 1;
                while idx < len {
                    let next = bytes[idx];
                    if next == b'\\' {
                        idx += 2;
                        continue;
                    }
                    if next == ch {
                        break;
                    }
                    idx += 1;
                }
            }
            b'(' => stack.push(b')'),
            b'[' => stack.push(b']'),
            b')' | b']' | b'}' => {
                if stack.is_empty() {
                    return false;
                }
                if stack.last() == Some(&ch) {
                    stack.pop();
                }
            }
            b';' if stack.is_empty() => return false,
            _ => {}
        }
        idx += 1;
    }
    true
}
