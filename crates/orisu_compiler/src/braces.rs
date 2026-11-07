// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use orisu_parser::segment::segment;

fn numeric_range(s: &str) -> Option<(i64, i64, Option<i64>)> {
    let parts: Vec<&str> = s.split("..").collect();
    if parts.len() != 2 && parts.len() != 3 {
        return None;
    }
    let num = |p: &str| -> Option<i64> {
        let digits = p.strip_prefix('-').unwrap_or(p);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse().ok()
    };
    Some((
        num(parts[0])?,
        num(parts[1])?,
        match parts.get(2) {
            Some(p) => Some(num(p)?),
            None => None,
        },
    ))
}

pub(crate) fn expand_braces(pattern: &str) -> Result<Vec<String>, String> {
    let Some(index) = pattern.find('{') else {
        return Ok(vec![pattern.to_owned()]);
    };
    let pre = &pattern[..index];
    let rest = &pattern[index..];
    let mut depth = 0i32;
    let mut end = rest.rfind('}');
    for (i, b) in rest.bytes().enumerate() {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                end = Some(i);
                break;
            }
        }
    }
    let Some(end) = end else {
        return Err(format!("The pattern `{pattern}` is not balanced."));
    };
    let inside = rest.get(1..end).unwrap_or("");
    let post = &rest[end + 1..];
    let parts: Vec<String> = match numeric_range(inside) {
        Some((start, stop, step)) => {
            let increasing = start < stop;
            let mut step = step.unwrap_or(if start <= stop { 1 } else { -1 });
            if step == 0 {
                return Err("Step cannot be zero in sequence expansion.".to_owned());
            }
            if increasing && step < 0 || !increasing && step > 0 {
                step = -step;
            }
            let mut out = Vec::new();
            let mut i = start;
            while if increasing { i <= stop } else { i >= stop } {
                out.push(i.to_string());
                i += step;
            }
            out
        }
        None => segment(inside, b',').into_iter().map(str::to_owned).collect(),
    };
    let mut expanded = Vec::new();
    for part in parts {
        expanded.extend(expand_braces(&part)?);
    }
    let mut result = Vec::new();
    for tail in expand_braces(post)? {
        for part in &expanded {
            result.push(format!("{pre}{part}{tail}"));
        }
    }
    Ok(result)
}
