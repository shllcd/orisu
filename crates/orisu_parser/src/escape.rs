// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use orisu_ast::bytes::ByteTable;
use std::borrow::Cow;
use std::fmt::Write;

static PLAIN: ByteTable = ByteTable::new(b"-_0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ");

fn needs_escape(value: &str) -> bool {
    let b = value.as_bytes();
    match b {
        [] => false,
        [b'-'] => true,
        [first, ..] if first.is_ascii_digit() => true,
        [b'-', second, ..] if second.is_ascii_digit() => true,
        _ => b.iter().any(|&c| c < 0x80 && !PLAIN.has(c)),
    }
}

pub fn escape(value: &str) -> Cow<'_, str> {
    if !needs_escape(value) {
        return Cow::Borrowed(value);
    }
    let mut out = String::with_capacity(value.len() + 8);
    escape_into(value, &mut out);
    Cow::Owned(out)
}

pub fn escape_into(value: &str, out: &mut impl Write) {
    if value == "-" {
        let _ = out.write_str("\\-");
        return;
    }
    let first = value.as_bytes().first().copied();
    for (index, ch) in value.chars().enumerate() {
        let code = ch as u32;
        if code == 0 {
            let _ = out.write_char('\u{FFFD}');
        } else if (0x01..=0x1f).contains(&code)
            || code == 0x7f
            || (index == 0 && ch.is_ascii_digit())
            || (index == 1 && ch.is_ascii_digit() && first == Some(b'-'))
        {
            let _ = write!(out, "\\{code:x} ");
        } else if code >= 0x80 || PLAIN.has(code as u8) {
            let _ = out.write_char(ch);
        } else {
            let _ = out.write_char('\\');
            let _ = out.write_char(ch);
        }
    }
}

pub fn unescape(escaped: &str) -> Cow<'_, str> {
    let Some(first) = memchr::memchr(b'\\', escaped.as_bytes()) else {
        return Cow::Borrowed(escaped);
    };
    let bytes = escaped.as_bytes();
    let mut result = String::with_capacity(escaped.len());
    result.push_str(&escaped[..first]);
    let mut i = first;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            let ch = escaped[i..].chars().next().unwrap_or('\\');
            result.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let hex_len = bytes[i + 1..].iter().take(6).take_while(|b| b.is_ascii_hexdigit()).count();
        if hex_len == 0 {
            let ch = escaped[i + 1..].chars().next().unwrap_or('\\');
            result.push(ch);
            i += 1 + ch.len_utf8();
            continue;
        }
        let code = u32::from_str_radix(&escaped[i + 1..i + 1 + hex_len], 16).unwrap_or(0);
        let mut end = i + 1 + hex_len;
        if matches!(bytes.get(end), Some(b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')) {
            end += 1;
        }
        result.push(match code {
            0 | 0xd800..=0xdfff => '\u{FFFD}',
            code => char::from_u32(code).unwrap_or('\u{FFFD}'),
        });
        i = end;
    }
    Cow::Owned(result)
}
