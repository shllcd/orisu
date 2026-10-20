// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::infer::is_named_color;
use crate::segment::segment;
use crate::value::{ValueNode, parse, to_css, write_css};
use orisu_allocator::{Allocator, ArenaString};
use orisu_ast::bytes::trim;

const KEYWORDS: [&str; 5] = ["inset", "inherit", "initial", "revert", "unset"];
const LENGTH_FUNCTIONS: [&str; 5] = ["calc", "clamp", "max", "min", "--spacing"];
const COLOR_FUNCTIONS: [&str; 15] = [
    "color",
    "color-mix",
    "contrast-color",
    "device-cmyk",
    "hsl",
    "hsla",
    "hwb",
    "lab",
    "lch",
    "light-dark",
    "oklab",
    "oklch",
    "rgb",
    "rgba",
    "--alpha",
];

fn is_one_of(list: &[&str], value: &str) -> bool {
    list.iter().any(|k| k.eq_ignore_ascii_case(value))
}

fn is_length(word: &str) -> bool {
    let b = word.strip_prefix('-').unwrap_or(word).as_bytes();
    matches!(b, [b'0'..=b'9', ..] | [b'.', b'0'..=b'9', ..]) && !word.contains(['\r', '\u{2028}', '\u{2029}'])
}

/// Rewrites the color of every comma-separated shadow in `input` through `replacement`,
/// appending one when a shadow has offsets but no color.
pub fn replace_shadow_colors<'a>(input: &str, alloc: &'a Allocator, replacement: &mut dyn FnMut(&'a str) -> &'a str) -> &'a str {
    let mut out = ArenaString::new_in(alloc);
    for (i, shadow) in segment(input, b',').into_iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let shadow = trim(shadow);
        let ast = parse(shadow, alloc);
        let mut target = None;
        let mut unknown = None;
        let mut unknowns = 0;
        let mut lengths = 0;
        for (idx, node) in ast.iter().enumerate() {
            match node {
                ValueNode::Word(v) => {
                    if is_one_of(&KEYWORDS, v) {
                        continue;
                    }
                    if is_length(v) {
                        lengths += 1;
                        continue;
                    }
                    if v.starts_with('#') || is_named_color(v) {
                        target = Some(idx);
                        break;
                    }
                    unknown = Some(idx);
                    unknowns += 1;
                }
                ValueNode::Function(name, _) => {
                    if is_one_of(&COLOR_FUNCTIONS, name) {
                        target = Some(idx);
                        break;
                    }
                    if is_one_of(&LENGTH_FUNCTIONS, name) {
                        lengths += 1;
                        continue;
                    }
                    unknown = Some(idx);
                    unknowns += 1;
                }
                ValueNode::Separator(_) => {}
            }
        }
        if target.is_none() {
            if lengths < 2 {
                out.push_str(shadow);
                continue;
            }
            if unknowns == 0 {
                out.push_str(shadow);
                out.push(' ');
                out.push_str(replacement("currentcolor"));
                continue;
            }
            if unknowns == 1 {
                target = unknown;
            }
        }
        let Some(target) = target else {
            out.push_str(shadow);
            continue;
        };
        for (idx, node) in ast.iter().enumerate() {
            if idx == target {
                let color = to_css(std::slice::from_ref(node), alloc);
                write_css(&parse(replacement(color), alloc), &mut out);
            } else {
                write_css(std::slice::from_ref(node), &mut out);
            }
        }
    }
    out.into_bump_str()
}
