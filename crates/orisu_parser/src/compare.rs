// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use std::cmp::Ordering;

pub fn compare(a: &str, z: &str) -> Ordering {
    let a = a.as_bytes();
    let z = z.as_bytes();
    let min = a.len().min(z.len());
    let mut i = 0;
    while i < min {
        let ac = a[i];
        let zc = z[i];
        if ac.is_ascii_digit() && zc.is_ascii_digit() {
            let mut a_end = i + 1;
            while a_end < a.len() && a[a_end].is_ascii_digit() {
                a_end += 1;
            }
            let mut z_end = i + 1;
            while z_end < z.len() && z[z_end].is_ascii_digit() {
                z_end += 1;
            }
            let an = &a[i..a_end];
            let zn = &z[i..z_end];
            let parse = |s: &[u8]| std::str::from_utf8(s).ok().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            let diff = parse(an) - parse(zn);
            if diff != 0.0 {
                return if diff < 0.0 { Ordering::Less } else { Ordering::Greater };
            }
            match an.cmp(zn) {
                Ordering::Equal => {}
                other => return other,
            }
            i += 1;
            continue;
        }
        if ac != zc {
            return ac.cmp(&zc);
        }
        i += 1;
    }
    a.len().cmp(&z.len())
}

/// `parseFloat`: the longest numeric prefix after leading whitespace.
fn parse_float(s: &str) -> Option<f64> {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    let digits = |mut i: usize| {
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        i
    };
    let sign = usize::from(matches!(bytes.first(), Some(b'-' | b'+')));
    if s[sign..].starts_with("Infinity") {
        return Some(if bytes[0] == b'-' { f64::NEG_INFINITY } else { f64::INFINITY });
    }
    let int_end = digits(sign);
    let mut end = int_end;
    if bytes.get(end) == Some(&b'.') {
        end = digits(end + 1);
    }
    if end == sign || (int_end == sign && end == sign + 1) {
        return None;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let exponent = end + 1 + usize::from(matches!(bytes.get(end + 1), Some(b'-' | b'+')));
        let exponent_end = digits(exponent);
        if exponent_end > exponent {
            end = exponent_end;
        }
    }
    s[..end].parse().ok()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}

pub fn compare_breakpoints(a: &str, z: &str, direction: Direction) -> Ordering {
    if a == z {
        return Ordering::Equal;
    }
    let bucket = |s: &str| -> String {
        match s.find('(') {
            Some(i) => s[..i].to_owned(),
            None => s.chars().filter(|c| !c.is_ascii_digit() && *c != '.').collect(),
        }
    };
    let ab = bucket(a);
    let zb = bucket(z);
    if ab != zb {
        return ab.cmp(&zb);
    }
    let order = match direction {
        Direction::Asc => parse_float(a).zip(parse_float(z)).and_then(|(x, y)| (x - y).partial_cmp(&0.0)),
        Direction::Desc => parse_float(a).zip(parse_float(z)).and_then(|(x, y)| (y - x).partial_cmp(&0.0)),
    };
    match order {
        Some(order) => order,
        None => {
            if a < z {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
    }
}

pub fn dimension(input: &str) -> Option<(f64, Option<&str>)> {
    let bytes = input.as_bytes();
    let mut i = 0;
    if matches!(bytes.first(), Some(b'-' | b'+')) {
        i = 1;
    }
    let num_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    let num = &input[..i];
    let digits = &input[num_start..i];
    let dots = digits.bytes().filter(|b| *b == b'.').count();
    if digits.is_empty() || dots > 1 || digits.ends_with('.') {
        return None;
    }
    let unit = &input[i..];
    if !unit.is_empty() && unit != "%" && !unit.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let value: f64 = num.parse().ok()?;
    Some((value, (!unit.is_empty()).then_some(unit)))
}
