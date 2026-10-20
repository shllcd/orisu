// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

mod backgrounds;
mod borders;
mod filters;
mod layout;
mod masks;
mod text;
mod transitions;
mod typography;

use super::{Cx, Named, Registrar, Spacing, Utilities};
use orisu_allocator::{Allocator, ArenaString};
use orisu_ast::{Node, Nodes};
use orisu_parser::infer::{DataType, infer_data_type, is_multiple_of_quarter, is_positive_integer, is_strict_positive_integer};
use orisu_parser::segment::segment;
use orisu_parser::shadow::replace_shadow_colors;
use orisu_theme::Theme;

pub fn register_builtin<'a>(utilities: &mut Utilities<'a>, theme: &Theme<'a>, alloc: &'a Allocator) {
    let mut r = Registrar { utilities, theme, alloc };
    layout::register(&mut r);
    borders::register(&mut r);
    backgrounds::register(&mut r);
    masks::register(&mut r);
    typography::register(&mut r);
    filters::register(&mut r);
    transitions::register(&mut r);
    text::register(&mut r);
}

fn bare_integer<'a>(_: &Cx<'a, '_>, n: Named<'a>) -> Option<&'a str> {
    is_positive_integer(n.value).then_some(n.value)
}

fn bare_percent<'a>(cx: &Cx<'a, '_>, n: Named<'a>) -> Option<&'a str> {
    is_positive_integer(n.value).then(|| cx.cat(&[n.value, "%"]))
}

fn bare_deg<'a>(cx: &Cx<'a, '_>, n: Named<'a>) -> Option<&'a str> {
    is_positive_integer(n.value).then(|| cx.cat(&[n.value, "deg"]))
}

fn bare_strict_integer_repeat<'a>(cx: &Cx<'a, '_>, n: Named<'a>) -> Option<&'a str> {
    is_strict_positive_integer(n.value).then(|| cx.cat(&["repeat(", n.value, ", minmax(0, 1fr))"]))
}

fn spacing_bare<'a>(cx: &Cx<'a, '_>, n: Named<'a>) -> Option<&'a str> {
    cx.resolve(None, &["--spacing"]).filter(|v| !v.is_empty())?;
    is_multiple_of_quarter(n.value).then(|| cx.cat(&["--spacing(", n.value, ")"]))
}

const NEG: Spacing = Spacing { supports_negative: true, supports_fractions: false };
const NEG_FRAC: Spacing = Spacing { supports_negative: true, supports_fractions: true };
const FRAC: Spacing = Spacing { supports_negative: false, supports_fractions: true };
const PLAIN: Spacing = Spacing { supports_negative: false, supports_fractions: false };

const POSITIONS: [(&str, &str); 9] = [
    ("top", "top"),
    ("top-left", "left top"),
    ("top-right", "right top"),
    ("bottom", "bottom"),
    ("bottom-left", "left bottom"),
    ("bottom-right", "right bottom"),
    ("left", "left"),
    ("right", "right"),
    ("center", "center"),
];

const REPEATS: [(&str, &str); 6] = [
    ("repeat", "repeat"),
    ("no-repeat", "no-repeat"),
    ("repeat-x", "repeat-x"),
    ("repeat-y", "repeat-y"),
    ("repeat-round", "round"),
    ("repeat-space", "space"),
];

#[derive(Clone, Copy)]
enum Stop {
    Color,
    Position,
}

fn arbitrary_type<'a>(data_type: Option<&'a str>, value: &str, types: &[DataType]) -> Option<&'a str> {
    data_type.or_else(|| infer_data_type(value, types).map(DataType::name))
}

fn bare_px<'a>(cx: &Cx<'a, '_>, n: Named<'a>) -> Option<&'a str> {
    is_positive_integer(n.value).then(|| cx.cat(&[n.value, "px"]))
}

fn alpha_color<'a>(cx: &Cx<'a, '_>, color: &'a str, alpha: Option<&'a str>, fallback: &mut bool) -> &'a str {
    match alpha {
        None => color,
        Some(alpha) if color.starts_with("current") => cx.with_alpha(color, alpha),
        Some(alpha) => {
            if color.starts_with("var(") || alpha.starts_with("var(") {
                *fallback = true;
            }
            cx.replace_alpha(color, alpha)
        }
    }
}

fn supports_lab<'a>(cx: &Cx<'a, '_>, decl: Node<'a>) -> Node<'a> {
    Node::AtRule { name: "@supports", params: "(color: lab(from red l a b))", nodes: cx.nodes([decl]) }
}

fn push_alpha_replaced_shadow<'a>(
    cx: &Cx<'a, '_>,
    out: &mut Nodes<'a>,
    property: &'a str,
    value: &'a str,
    alpha: Option<&'a str>,
    color_var: &str,
    prefix: &str,
) {
    let inject = |color: &str| cx.cat(&["var(", color_var, ", ", color, ")"]);
    let mut fallback = false;
    let replaced = replace_shadow_colors(value, cx.alloc, &mut |color| inject(alpha_color(cx, color, alpha, &mut fallback)));
    let apply_prefix = |x: &'a str| -> &'a str {
        if prefix.is_empty() {
            return x;
        }
        let prefix = orisu_ast::bytes::trim(prefix);
        let mut out = ArenaString::new_in(cx.alloc);
        for (i, v) in segment(x, b',').into_iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(prefix);
            out.push(' ');
            out.push_str(orisu_ast::bytes::trim(v));
        }
        out.into_bump_str()
    };
    if fallback {
        let plain = replace_shadow_colors(value, cx.alloc, &mut |color| inject(color));
        out.push(cx.decl(property, apply_prefix(plain)));
        out.push(supports_lab(cx, cx.decl(property, apply_prefix(replaced))));
    } else {
        out.push(cx.decl(property, apply_prefix(replaced)));
    }
}

fn push_alpha_replaced_drop_shadow<'a>(
    cx: &Cx<'a, '_>,
    out: &mut Nodes<'a>,
    property: &'a str,
    value: &'a str,
    alpha: Option<&'a str>,
    color_var: &str,
) {
    let inject = |color: &str| cx.cat(&["var(", color_var, ", ", color, ")"]);
    let drop_shadows = |replace: &mut dyn FnMut(&'a str) -> &'a str| -> &'a str {
        let mut out = ArenaString::new_in(cx.alloc);
        for (i, v) in segment(value, b',').into_iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str("drop-shadow(");
            out.push_str(replace_shadow_colors(v, cx.alloc, replace));
            out.push(')');
        }
        out.into_bump_str()
    };
    let mut fallback = false;
    let replaced = drop_shadows(&mut |color| inject(alpha_color(cx, color, alpha, &mut fallback)));
    if fallback {
        out.push(cx.decl(property, drop_shadows(&mut |color| inject(color))));
        out.push(supports_lab(cx, cx.decl(property, replaced)));
    } else {
        out.push(cx.decl(property, replaced));
    }
}
