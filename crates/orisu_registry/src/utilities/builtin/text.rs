use super::{arbitrary_type, bare_px, push_alpha_replaced_shadow};
use crate::utilities::{Cx, Functional, Registrar, STEPS_OF_5, Suggest};
use orisu_allocator::ArenaString;
use orisu_allocator::Vec;
use orisu_ast::{Node, Nodes, bytes::trim};
use orisu_candidate::{CandidateKind, Modifier, UtilityValue};
use orisu_parser::infer::{DataType, is_multiple_of_quarter, is_positive_integer};
use orisu_parser::segment::segment;
use orisu_parser::shadow::replace_shadow_colors;

const BOX_SHADOW_VALUE: &str =
    "var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow)";
const NULL_SHADOW: &str = "0 0 #0000";
const RING_OFFSET_SHADOW: &str = "var(--tw-ring-inset,) 0 0 0 var(--tw-ring-offset-width) var(--tw-ring-offset-color)";

fn outline_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([cx.property("--tw-outline-style", Some("solid"), None)])
}

fn text_shadow_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-text-shadow-color", None, None),
        cx.property("--tw-text-shadow-alpha", Some("100%"), Some("<percentage>")),
    ])
}

fn box_shadow_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-shadow", Some(NULL_SHADOW), None),
        cx.property("--tw-shadow-color", None, None),
        cx.property("--tw-shadow-alpha", Some("100%"), Some("<percentage>")),
        cx.property("--tw-inset-shadow", Some(NULL_SHADOW), None),
        cx.property("--tw-inset-shadow-color", None, None),
        cx.property("--tw-inset-shadow-alpha", Some("100%"), Some("<percentage>")),
        cx.property("--tw-ring-color", None, None),
        cx.property("--tw-ring-shadow", Some(NULL_SHADOW), None),
        cx.property("--tw-inset-ring-color", None, None),
        cx.property("--tw-inset-ring-shadow", Some(NULL_SHADOW), None),
        cx.property("--tw-ring-inset", None, None),
        cx.property("--tw-ring-offset-width", Some("0px"), Some("<length>")),
        cx.property("--tw-ring-offset-color", Some("#fff"), None),
        cx.property("--tw-ring-offset-shadow", Some(NULL_SHADOW), None),
    ])
}

fn shadow_alpha<'a>(cx: &Cx<'a, '_>, modifier: Option<Modifier<'a>>) -> Option<&'a str> {
    match modifier {
        Some(Modifier::Arbitrary(v)) => Some(v),
        Some(Modifier::Named(v)) if is_multiple_of_quarter(v) => Some(cx.cat(&[v, "%"])),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct Shadow {
    name: &'static str,
    properties: for<'a, 't> fn(&Cx<'a, 't>) -> Node<'a>,
    target: &'static str,
    color_var: &'static str,
    alpha_var: &'static str,
    alpha_ref: &'static str,
    theme_key: &'static str,
    color_keys: &'static [&'static str],
    none: &'static str,
    box_shadow: bool,
    arbitrary_prefix: &'static str,
}

fn shadow_value<'a>(cx: &Cx<'a, '_>, value: &'a str, prefix: &str, color_var: &str) -> &'a str {
    let replaced = replace_shadow_colors(value, cx.alloc, &mut |color| cx.cat(&["var(", color_var, ", ", color, ")"]));
    if prefix.is_empty() {
        return replaced;
    }
    let prefix = trim(prefix);
    let mut out = ArenaString::new_in(cx.alloc);
    for (i, part) in segment(replaced, b',').into_iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(prefix);
        out.push(' ');
        out.push_str(trim(part));
    }
    out.into_bump_str()
}

fn shadow<'a>(r: &mut Registrar<'a, '_>, s: Shadow) {
    r.static_fn(&format!("{}-initial", s.name), move |cx, _| Some(cx.nodes([(s.properties)(cx), cx.decl(s.color_var, "initial")])));
    r.functional_decl(
        s.name,
        move |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            let alpha = shadow_alpha(cx, modifier);
            let shadows = |value: &'a str, prefix: &str| {
                let mut nodes = Vec::with_capacity_in(5, cx.alloc);
                nodes.push((s.properties)(cx));
                nodes.push(cx.decl_opt(s.alpha_var, alpha));
                push_alpha_replaced_shadow(cx, &mut nodes, s.target, value, alpha, s.color_var, prefix);
                if s.box_shadow {
                    nodes.push(cx.decl("box-shadow", BOX_SHADOW_VALUE));
                }
                Some(nodes)
            };
            let colored = |color: &'a str| Some(cx.nodes([(s.properties)(cx), cx.decl(s.color_var, color)]));
            let value = match value {
                None => return shadows(cx.theme.get(&[s.theme_key])?, ""),
                Some(UtilityValue::Arbitrary { data_type, value }) => {
                    if arbitrary_type(data_type, value, &[DataType::Color]) == Some("color") {
                        return colored(cx.with_alpha(cx.as_color(value, modifier)?, s.alpha_ref));
                    }
                    return shadows(value, s.arbitrary_prefix);
                }
                Some(UtilityValue::Named { value, .. }) => value,
            };
            match value {
                "none" if modifier.is_none() => {
                    let mut nodes = Vec::with_capacity_in(3, cx.alloc);
                    nodes.push((s.properties)(cx));
                    nodes.push(cx.decl(s.target, s.none));
                    if s.box_shadow {
                        nodes.push(cx.decl("box-shadow", BOX_SHADOW_VALUE));
                    }
                    return Some(nodes);
                }
                "inherit" if modifier.is_none() => return colored("inherit"),
                "none" | "inherit" => return None,
                _ => {}
            }
            if let Some(v) = cx.theme.get(&[&format!("{}-{value}", s.theme_key)]).filter(|v| !v.is_empty()) {
                return shadows(v, "");
            }
            colored(cx.with_alpha(cx.resolve_theme_color(value, modifier, s.color_keys)?, s.alpha_ref))
        },
        move |cx, c| {
            let CandidateKind::Functional { value, .. } = c.kind else {
                return None;
            };
            let colored = |cx: &Cx<'a, '_>, color: &'a str| Some(Vec::from_iter_in([(s.color_var, color)], cx.alloc));
            let shadows = |cx: &Cx<'a, '_>, value: &'a str, prefix: &str| {
                let mut out = Vec::with_capacity_in(2, cx.alloc);
                out.push((s.target, shadow_value(cx, value, prefix, s.color_var)));
                if s.box_shadow {
                    out.push(("box-shadow", BOX_SHADOW_VALUE));
                }
                Some(out)
            };
            match value {
                None => shadows(cx, cx.theme.get(&[s.theme_key])?, ""),
                Some(UtilityValue::Arbitrary { data_type, value }) => {
                    if arbitrary_type(data_type, value, &[DataType::Color]) == Some("color") {
                        return colored(cx, cx.with_alpha(cx.as_color(value, None)?, s.alpha_ref));
                    }
                    shadows(cx, value, s.arbitrary_prefix)
                }
                Some(UtilityValue::Named { value, .. }) => match value {
                    "none" => {
                        let mut out = Vec::with_capacity_in(2, cx.alloc);
                        out.push((s.target, s.none));
                        if s.box_shadow {
                            out.push(("box-shadow", BOX_SHADOW_VALUE));
                        }
                        Some(out)
                    }
                    "inherit" => colored(cx, "inherit"),
                    _ => {
                        if let Some(v) = cx.theme.get(&[&format!("{}-{value}", s.theme_key)]).filter(|v| !v.is_empty()) {
                            return shadows(cx, v, "");
                        }
                        colored(cx, cx.with_alpha(cx.resolve_theme_color(value, None, s.color_keys)?, s.alpha_ref))
                    }
                },
            }
        },
    );
    let theme_keys = r.keys(&[s.theme_key]);
    r.suggest(
        s.name,
        vec![
            Suggest::color(s.color_keys),
            Suggest::values(&["none"]),
            Suggest::default().keys(theme_keys).modifiers(&STEPS_OF_5).default_key(s.theme_key),
        ],
    );
}

fn ring<'a>(
    r: &mut Registrar<'a, '_>,
    name: &str,
    color_var: &'static str,
    shadow_var: &'static str,
    default_width: &'a str,
    head: &'static str,
    tail: &'a str,
) {
    r.functional_decl(
        name,
        move |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            let width = |v: &'a str| {
                Some(cx.nodes([
                    box_shadow_properties(cx),
                    cx.decl(shadow_var, cx.cat(&[head, v, tail])),
                    cx.decl("box-shadow", BOX_SHADOW_VALUE),
                ]))
            };
            let Some(value) = value else {
                if modifier.is_some() {
                    return None;
                }
                return width(default_width);
            };
            match value {
                UtilityValue::Arbitrary { data_type, value } => {
                    if arbitrary_type(data_type, value, &[DataType::Color, DataType::Length]) == Some("length") {
                        if modifier.is_some() {
                            return None;
                        }
                        return width(value);
                    }
                    cx.one(color_var, cx.as_color(value, modifier)?)
                }
                UtilityValue::Named { value, .. } => {
                    if let Some(v) = cx.resolve_theme_color(value, modifier, &["--ring-color", "--color"]) {
                        return cx.one(color_var, v);
                    }
                    if modifier.is_some() {
                        return None;
                    }
                    let v = cx
                        .resolve(Some(value), &["--ring-width"])
                        .or_else(|| is_positive_integer(value).then(|| cx.cat(&[value, "px"])))
                        .filter(|v| !v.is_empty())?;
                    width(v)
                }
            }
        },
        move |cx, c| {
            let CandidateKind::Functional { value, .. } = c.kind else {
                return None;
            };
            let width = |cx: &Cx<'a, '_>, v: &'a str| {
                Some(Vec::from_iter_in([(shadow_var, cx.cat(&[head, v, tail])), ("box-shadow", BOX_SHADOW_VALUE)], cx.alloc))
            };
            let Some(value) = value else {
                return width(cx, default_width);
            };
            match value {
                UtilityValue::Arbitrary { data_type, value } => {
                    if arbitrary_type(data_type, value, &[DataType::Color, DataType::Length]) == Some("length") {
                        return width(cx, value);
                    }
                    let color = cx.as_color(value, None)?;
                    Some(Vec::from_iter_in([(color_var, color)], cx.alloc))
                }
                UtilityValue::Named { value, .. } => {
                    if let Some(color) = cx.resolve_theme_color(value, None, &["--ring-color", "--color"]) {
                        return Some(Vec::from_iter_in([(color_var, color)], cx.alloc));
                    }
                    let value = cx
                        .resolve(Some(value), &["--ring-width"])
                        .or_else(|| is_positive_integer(value).then(|| cx.cat(&[value, "px"])))
                        .filter(|v| !v.is_empty())?;
                    width(cx, value)
                }
            }
        },
    );
    r.suggest(
        name,
        vec![
            Suggest::color(&["--ring-color", "--color"]),
            Suggest::values(&["0", "1", "2", "4", "8"]).keys(&["--ring-width"]).default_value(true),
        ],
    );
}

fn line_height<'a>(cx: &Cx<'a, '_>, modifier: Modifier<'a>) -> Option<&'a str> {
    let name = modifier.value();
    let resolved = match modifier {
        Modifier::Arbitrary(v) => Some(v),
        Modifier::Named(v) => cx.resolve(Some(v), &["--leading"]),
    };
    resolved.filter(|v| !v.is_empty()).or_else(|| {
        if is_multiple_of_quarter(name) {
            cx.resolve(None, &["--spacing"]).filter(|v| !v.is_empty())?;
            return Some(cx.cat(&["--spacing(", name, ")"]));
        }
        (name == "none").then_some("1")
    })
}

fn font_size<'a>(cx: &Cx<'a, '_>, size: &'a str, modifier: Option<Modifier<'a>>) -> Option<Nodes<'a>> {
    match modifier {
        Some(m) => Some(cx.nodes([cx.decl("font-size", size), cx.decl("line-height", line_height(cx, m)?)])),
        None => cx.one("font-size", size),
    }
}

pub(super) fn register<'a>(r: &mut Registrar<'a, '_>) {
    r.static_fn("outline-hidden", |cx, _| {
        Some(cx.nodes([
            cx.decl("--tw-outline-style", "none"),
            cx.decl("outline-style", "none"),
            Node::AtRule {
                name: "@media",
                params: "(forced-colors: active)",
                nodes: cx.nodes([cx.decl("outline", "2px solid transparent"), cx.decl("outline-offset", "2px")]),
            },
        ]))
    });
    for style in ["none", "solid", "dashed", "dotted", "double"] {
        r.static_decls(&format!("outline-{style}"), &[("--tw-outline-style", style), ("outline-style", style)]);
    }
    r.functional_decl(
        "outline",
        |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            let width = |v| {
                Some(cx.nodes([outline_properties(cx), cx.decl("outline-style", "var(--tw-outline-style)"), cx.decl("outline-width", v)]))
            };
            let Some(value) = value else {
                if modifier.is_some() {
                    return None;
                }
                return width(cx.theme.get(&["--default-outline-width"]).unwrap_or("1px"));
            };
            match value {
                UtilityValue::Arbitrary { data_type, value } => {
                    match arbitrary_type(data_type, value, &[DataType::Color, DataType::Length, DataType::Number, DataType::Percentage]) {
                        Some("length" | "number" | "percentage") => {
                            if modifier.is_some() {
                                return None;
                            }
                            width(value)
                        }
                        _ => cx.one("outline-color", cx.as_color(value, modifier)?),
                    }
                }
                UtilityValue::Named { value, .. } => {
                    if let Some(v) = cx.resolve_theme_color(value, modifier, &["--outline-color", "--color"]) {
                        return cx.one("outline-color", v);
                    }
                    if modifier.is_some() {
                        return None;
                    }
                    if let Some(v) = cx.resolve(Some(value), &["--outline-width"]).filter(|v| !v.is_empty()) {
                        return width(v);
                    }
                    is_positive_integer(value).then(|| width(cx.cat(&[value, "px"])))?
                }
            }
        },
        |cx, c| {
            let CandidateKind::Functional { value, .. } = c.kind else {
                return None;
            };
            let width = |cx: &Cx<'a, '_>, v: &'a str| {
                Some(Vec::from_iter_in([("outline-style", "var(--tw-outline-style)"), ("outline-width", v)], cx.alloc))
            };
            let Some(value) = value else {
                return width(cx, cx.theme.get(&["--default-outline-width"]).unwrap_or("1px"));
            };
            match value {
                UtilityValue::Arbitrary { data_type, value } => {
                    match arbitrary_type(data_type, value, &[DataType::Color, DataType::Length, DataType::Number, DataType::Percentage]) {
                        Some("length" | "number" | "percentage") => width(cx, value),
                        _ => {
                            let color = cx.as_color(value, None)?;
                            Some(Vec::from_iter_in([("outline-color", color)], cx.alloc))
                        }
                    }
                }
                UtilityValue::Named { value, .. } => {
                    if let Some(color) = cx.resolve_theme_color(value, None, &["--outline-color", "--color"]) {
                        return Some(Vec::from_iter_in([("outline-color", color)], cx.alloc));
                    }
                    if let Some(v) = cx.resolve(Some(value), &["--outline-width"]).filter(|v| !v.is_empty()) {
                        return width(cx, v);
                    }
                    is_positive_integer(value).then(|| width(cx, cx.cat(&[value, "px"])))?
                }
            }
        },
    );
    r.suggest(
        "outline",
        vec![
            Suggest::color(&["--outline-color", "--color"]).default_value(true),
            Suggest::values(&["0", "1", "2", "4", "8"]).keys(&["--outline-width"]),
        ],
    );
    r.functional("outline-offset", Functional::decl("outline-offset").negative().keys(&["--outline-offset"]).bare(bare_px));
    r.suggest("outline-offset", vec![Suggest::values(&["0", "1", "2", "4", "8"]).negative(true).keys(&["--outline-offset"])]);

    r.functional(
        "opacity",
        Functional::decl("opacity").keys(&["--opacity"]).bare(|cx, n| is_multiple_of_quarter(n.value).then(|| cx.cat(&[n.value, "%"]))),
    );
    r.suggest("opacity", vec![Suggest::values(&STEPS_OF_5).keys(&["--opacity"])]);

    let auto = r.statics(&[("auto", &[("text-underline-offset", "auto")])]);
    r.functional(
        "underline-offset",
        Functional::decl("text-underline-offset").negative().keys(&["--text-underline-offset"]).bare(bare_px).statics(auto),
    );
    r.suggest("underline-offset", vec![Suggest::values(&["0", "1", "2", "4", "8"]).negative(true).keys(&["--text-underline-offset"])]);

    r.functional_decl(
        "text",
        |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            match value? {
                UtilityValue::Arbitrary { data_type, value } => match arbitrary_type(
                    data_type,
                    value,
                    &[DataType::Color, DataType::Length, DataType::Percentage, DataType::AbsoluteSize, DataType::RelativeSize],
                ) {
                    Some("size" | "length" | "percentage" | "absolute-size" | "relative-size") => font_size(cx, value, modifier),
                    _ => cx.one("color", cx.as_color(value, modifier)?),
                },
                UtilityValue::Named { value, .. } => {
                    if let Some(v) = cx.resolve_theme_color(value, modifier, &["--text-color", "--color"]) {
                        return cx.one("color", v);
                    }
                    let (size, options) =
                        cx.theme.resolve_with(value, &["--text"], &["--line-height", "--letter-spacing", "--font-weight"], cx.options)?;
                    if modifier.is_some() {
                        return font_size(cx, size, modifier);
                    }
                    let option = |key: &str, var: &str| {
                        options
                            .iter()
                            .find(|(name, _)| *name == key)
                            .map(|&(_, v)| v)
                            .filter(|v| !v.is_empty())
                            .map(|v| cx.cat(&["var(", var, ", ", v, ")"]))
                    };
                    Some(cx.nodes([
                        cx.decl("font-size", size),
                        cx.decl_opt("line-height", option("--line-height", "--tw-leading")),
                        cx.decl_opt("letter-spacing", option("--letter-spacing", "--tw-tracking")),
                        cx.decl_opt("font-weight", option("--font-weight", "--tw-font-weight")),
                    ]))
                }
            }
        },
        |cx, c| {
            let CandidateKind::Functional { value, .. } = c.kind else {
                return None;
            };
            match value? {
                UtilityValue::Arbitrary { data_type, value } => match arbitrary_type(
                    data_type,
                    value,
                    &[DataType::Color, DataType::Length, DataType::Percentage, DataType::AbsoluteSize, DataType::RelativeSize],
                ) {
                    Some("size" | "length" | "percentage" | "absolute-size" | "relative-size") => {
                        Some(Vec::from_iter_in([("font-size", value)], cx.alloc))
                    }
                    _ => {
                        let color = cx.as_color(value, None)?;
                        Some(Vec::from_iter_in([("color", color)], cx.alloc))
                    }
                },
                UtilityValue::Named { value, .. } => {
                    if let Some(color) = cx.resolve_theme_color(value, None, &["--text-color", "--color"]) {
                        return Some(Vec::from_iter_in([("color", color)], cx.alloc));
                    }
                    let (size, options) =
                        cx.theme.resolve_with(value, &["--text"], &["--line-height", "--letter-spacing", "--font-weight"], cx.options)?;
                    let mut out = Vec::with_capacity_in(4, cx.alloc);
                    out.push(("font-size", size));
                    for (key, var, property) in [
                        ("--line-height", "--tw-leading", "line-height"),
                        ("--letter-spacing", "--tw-tracking", "letter-spacing"),
                        ("--font-weight", "--tw-font-weight", "font-weight"),
                    ] {
                        if let Some(&(_, value)) = options.iter().find(|(name, _)| *name == key)
                            && !value.is_empty()
                        {
                            out.push((property, cx.cat(&["var(", var, ", ", value, ")"])));
                        }
                    }
                    Some(out)
                }
            }
        },
    );
    r.suggest(
        "text",
        vec![Suggest::color(&["--text-color", "--color"]), Suggest::default().keys(&["--text"]).modifier_keys(&["--leading"])],
    );

    shadow(
        r,
        Shadow {
            name: "text-shadow",
            properties: text_shadow_properties,
            target: "text-shadow",
            color_var: "--tw-text-shadow-color",
            alpha_var: "--tw-text-shadow-alpha",
            alpha_ref: "var(--tw-text-shadow-alpha)",
            theme_key: "--text-shadow",
            color_keys: &["--text-shadow-color", "--color"],
            none: "none",
            box_shadow: false,
            arbitrary_prefix: "",
        },
    );
    shadow(
        r,
        Shadow {
            name: "shadow",
            properties: box_shadow_properties,
            target: "--tw-shadow",
            color_var: "--tw-shadow-color",
            alpha_var: "--tw-shadow-alpha",
            alpha_ref: "var(--tw-shadow-alpha)",
            theme_key: "--shadow",
            color_keys: &["--box-shadow-color", "--color"],
            none: NULL_SHADOW,
            box_shadow: true,
            arbitrary_prefix: "",
        },
    );
    shadow(
        r,
        Shadow {
            name: "inset-shadow",
            properties: box_shadow_properties,
            target: "--tw-inset-shadow",
            color_var: "--tw-inset-shadow-color",
            alpha_var: "--tw-inset-shadow-alpha",
            alpha_ref: "var(--tw-inset-shadow-alpha)",
            theme_key: "--inset-shadow",
            color_keys: &["--box-shadow-color", "--color"],
            none: "inset 0 0 #0000",
            box_shadow: true,
            arbitrary_prefix: "inset",
        },
    );

    r.static_fn("ring-inset", |cx, _| Some(cx.nodes([box_shadow_properties(cx), cx.decl("--tw-ring-inset", "inset")])));
    let ring_color = r.theme.get(&["--default-ring-color"]).unwrap_or("currentcolor");
    let ring_width = r.theme.get(&["--default-ring-width"]).unwrap_or("1px");
    let ring_tail = r.cat(&[" + var(--tw-ring-offset-width)) var(--tw-ring-color, ", ring_color, ")"]);
    ring(r, "ring", "--tw-ring-color", "--tw-ring-shadow", ring_width, "var(--tw-ring-inset,) 0 0 0 calc(", ring_tail);
    ring(
        r,
        "inset-ring",
        "--tw-inset-ring-color",
        "--tw-inset-ring-shadow",
        "1px",
        "inset 0 0 0 ",
        " var(--tw-inset-ring-color, currentcolor)",
    );

    r.functional_decl(
        "ring-offset",
        |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            let width = |v| Some(cx.nodes([cx.decl("--tw-ring-offset-width", v), cx.decl("--tw-ring-offset-shadow", RING_OFFSET_SHADOW)]));
            match value? {
                UtilityValue::Arbitrary { data_type, value } => {
                    if arbitrary_type(data_type, value, &[DataType::Color, DataType::Length]) == Some("length") {
                        if modifier.is_some() {
                            return None;
                        }
                        return width(value);
                    }
                    cx.one("--tw-ring-offset-color", cx.as_color(value, modifier)?)
                }
                UtilityValue::Named { value, .. } => {
                    let w = cx
                        .resolve(Some(value), &["--ring-offset-width"])
                        .filter(|v| !v.is_empty())
                        .or_else(|| is_positive_integer(value).then(|| cx.cat(&[value, "px"])));
                    if let Some(w) = w {
                        if modifier.is_some() {
                            return None;
                        }
                        return width(w);
                    }
                    cx.one("--tw-ring-offset-color", cx.resolve_theme_color(value, modifier, &["--ring-offset-color", "--color"])?)
                }
            }
        },
        |cx, c| {
            let CandidateKind::Functional { value, .. } = c.kind else {
                return None;
            };
            let width = |cx: &Cx<'a, '_>, v: &'a str| {
                Some(Vec::from_iter_in([("--tw-ring-offset-width", v), ("--tw-ring-offset-shadow", RING_OFFSET_SHADOW)], cx.alloc))
            };
            match value? {
                UtilityValue::Arbitrary { data_type, value } => {
                    if arbitrary_type(data_type, value, &[DataType::Color, DataType::Length]) == Some("length") {
                        return width(cx, value);
                    }
                    let color = cx.as_color(value, None)?;
                    Some(Vec::from_iter_in([("--tw-ring-offset-color", color)], cx.alloc))
                }
                UtilityValue::Named { value, .. } => {
                    let w = cx
                        .resolve(Some(value), &["--ring-offset-width"])
                        .filter(|v| !v.is_empty())
                        .or_else(|| is_positive_integer(value).then(|| cx.cat(&[value, "px"])));
                    if let Some(w) = w {
                        return width(cx, w);
                    }
                    let color = cx.resolve_theme_color(value, None, &["--ring-offset-color", "--color"])?;
                    Some(Vec::from_iter_in([("--tw-ring-offset-color", color)], cx.alloc))
                }
            }
        },
    );
    r.suggest(
        "ring-offset",
        vec![
            Suggest::color(&["--ring-offset-color", "--color"]),
            Suggest::values(&["0", "1", "2", "4", "8"]).keys(&["--ring-offset-width"]),
        ],
    );

    r.functional_fn("@container", |cx, c| {
        let CandidateKind::Functional { value, modifier, .. } = c.kind else {
            return None;
        };
        let value = match value {
            None => "inline-size",
            Some(UtilityValue::Arbitrary { value, .. }) => value,
            Some(UtilityValue::Named { value: v @ ("normal" | "size"), .. }) => v,
            Some(UtilityValue::Named { .. }) => return None,
        };
        match modifier {
            Some(m) => Some(cx.nodes([cx.decl("container-type", value), cx.decl("container-name", m.value())])),
            None => cx.one("container-type", value),
        }
    });
    r.suggest("@container", vec![Suggest::values(&["normal"]).default_value(true)]);
}
