use super::{arbitrary_type, bare_deg, bare_percent, push_alpha_replaced_drop_shadow};
use crate::utilities::{Cx, DefaultValue, Functional, Named, Registrar, STEPS_OF_5, Suggest};
use orisu_allocator::{ArenaString, Vec};
use orisu_ast::{Node, Nodes};
use orisu_candidate::{CandidateKind, Modifier, UtilityValue};
use orisu_parser::infer::{DataType, is_multiple_of_quarter};
use orisu_parser::segment::segment;
use orisu_parser::shadow::replace_shadow_colors;

const FILTER_VALUE: &str = "var(--tw-blur,) var(--tw-brightness,) var(--tw-contrast,) var(--tw-grayscale,) var(--tw-hue-rotate,) var(--tw-invert,) var(--tw-saturate,) var(--tw-sepia,) var(--tw-drop-shadow,)";

const BACKDROP_FILTER_VALUE: &str = "var(--tw-backdrop-blur,) var(--tw-backdrop-brightness,) var(--tw-backdrop-contrast,) var(--tw-backdrop-grayscale,) var(--tw-backdrop-hue-rotate,) var(--tw-backdrop-invert,) var(--tw-backdrop-opacity,) var(--tw-backdrop-saturate,) var(--tw-backdrop-sepia,)";

fn filter_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-blur", None, None),
        cx.property("--tw-brightness", None, None),
        cx.property("--tw-contrast", None, None),
        cx.property("--tw-grayscale", None, None),
        cx.property("--tw-hue-rotate", None, None),
        cx.property("--tw-invert", None, None),
        cx.property("--tw-opacity", None, None),
        cx.property("--tw-saturate", None, None),
        cx.property("--tw-sepia", None, None),
        cx.property("--tw-drop-shadow", None, None),
        cx.property("--tw-drop-shadow-color", None, None),
        cx.property("--tw-drop-shadow-alpha", Some("100%"), Some("<percentage>")),
        cx.property("--tw-drop-shadow-size", None, None),
    ])
}

fn backdrop_filter_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-backdrop-blur", None, None),
        cx.property("--tw-backdrop-brightness", None, None),
        cx.property("--tw-backdrop-contrast", None, None),
        cx.property("--tw-backdrop-grayscale", None, None),
        cx.property("--tw-backdrop-hue-rotate", None, None),
        cx.property("--tw-backdrop-invert", None, None),
        cx.property("--tw-backdrop-opacity", None, None),
        cx.property("--tw-backdrop-saturate", None, None),
        cx.property("--tw-backdrop-sepia", None, None),
    ])
}

fn filter<'a>(cx: &Cx<'a, '_>, var: &'a str, value: &'a str) -> Option<Nodes<'a>> {
    Some(cx.nodes([filter_properties(cx), cx.decl(var, value), cx.decl("filter", FILTER_VALUE)]))
}

fn backdrop<'a>(cx: &Cx<'a, '_>, var: &'a str, value: &'a str) -> Option<Nodes<'a>> {
    Some(cx.nodes([
        backdrop_filter_properties(cx),
        cx.decl(var, value),
        cx.decl("-webkit-backdrop-filter", BACKDROP_FILTER_VALUE),
        cx.decl("backdrop-filter", BACKDROP_FILTER_VALUE),
    ]))
}

type Bare = for<'a, 't> fn(&Cx<'a, 't>, Named<'a>) -> Option<&'a str>;

fn filter_pair<'a>(
    r: &mut Registrar<'a, '_>,
    name: &str,
    bare: Bare,
    default: Option<&'static str>,
    negative: bool,
    values: &'static [&'static str],
) {
    let open = r.cat(&[name, "("]);
    for is_backdrop in [false, true] {
        let full = if is_backdrop { format!("backdrop-{name}") } else { name.to_owned() };
        let var = r.cat(&["--tw-", &full]);
        let keys = if is_backdrop { r.keys(&[&format!("--{full}"), &format!("--{name}")]) } else { r.keys(&[&format!("--{name}")]) };
        let mut desc = Functional::new(move |cx, v, _| {
            let value = cx.cat(&[open, v, ")"]);
            if is_backdrop { backdrop(cx, var, value) } else { filter(cx, var, value) }
        })
        .keys(keys)
        .bare(bare);
        if let Some(default) = default {
            desc = desc.default_value(DefaultValue::Value(default));
        }
        desc.supports_negative = negative;
        r.functional(&full, desc);
    }
    for is_backdrop in [false, true] {
        let full = if is_backdrop { format!("backdrop-{name}") } else { name.to_owned() };
        let keys = if is_backdrop { r.keys(&[&format!("--{full}"), &format!("--{name}")]) } else { r.keys(&[&format!("--{name}")]) };
        let has_default = default.is_some();
        r.suggest(&full, vec![Suggest::values(values).keys(keys).default_value(has_default)]);
    }
}

pub(super) fn register<'a>(r: &mut Registrar<'a, '_>) {
    r.functional_fn("filter", |cx, c| {
        let CandidateKind::Functional { value, modifier: None, .. } = c.kind else {
            return None;
        };
        match value {
            None => Some(cx.nodes([filter_properties(cx), cx.decl("filter", FILTER_VALUE)])),
            Some(UtilityValue::Arbitrary { value, .. }) => cx.one("filter", value),
            Some(UtilityValue::Named { value: "none", .. }) => cx.one("filter", "none"),
            Some(UtilityValue::Named { .. }) => None,
        }
    });
    r.functional_fn("backdrop-filter", |cx, c| {
        let CandidateKind::Functional { value, modifier: None, .. } = c.kind else {
            return None;
        };
        let value = match value {
            None => {
                return Some(cx.nodes([
                    backdrop_filter_properties(cx),
                    cx.decl("-webkit-backdrop-filter", BACKDROP_FILTER_VALUE),
                    cx.decl("backdrop-filter", BACKDROP_FILTER_VALUE),
                ]));
            }
            Some(UtilityValue::Arbitrary { value, .. }) => value,
            Some(UtilityValue::Named { value: "none", .. }) => "none",
            Some(UtilityValue::Named { .. }) => return None,
        };
        Some(cx.nodes([cx.decl("-webkit-backdrop-filter", value), cx.decl("backdrop-filter", value)]))
    });

    r.functional(
        "blur",
        Functional::new(|cx, v, _| filter(cx, "--tw-blur", cx.cat(&["blur(", v, ")"])))
            .keys(&["--blur"])
            .static_nodes(&["none"], |cx, _| filter(cx, "--tw-blur", " ")),
    );
    r.functional(
        "backdrop-blur",
        Functional::new(|cx, v, _| backdrop(cx, "--tw-backdrop-blur", cx.cat(&["blur(", v, ")"])))
            .keys(&["--backdrop-blur", "--blur"])
            .static_nodes(&["none"], |cx, _| backdrop(cx, "--tw-backdrop-blur", " ")),
    );
    filter_pair(r, "brightness", bare_percent, None, false, &["0", "50", "75", "90", "95", "100", "105", "110", "125", "150", "200"]);
    filter_pair(r, "contrast", bare_percent, None, false, &["0", "50", "75", "100", "125", "150", "200"]);
    filter_pair(r, "grayscale", bare_percent, Some("100%"), false, &["0", "25", "50", "75", "100"]);
    filter_pair(r, "hue-rotate", bare_deg, None, true, &["0", "15", "30", "60", "90", "180"]);
    filter_pair(r, "invert", bare_percent, Some("100%"), false, &["0", "25", "50", "75", "100"]);
    filter_pair(r, "saturate", bare_percent, None, false, &["0", "50", "100", "150", "200"]);
    filter_pair(r, "sepia", bare_percent, Some("100%"), false, &["0", "50", "100"]);

    r.static_fn("drop-shadow-none", |cx, _| filter(cx, "--tw-drop-shadow", " "));
    let drop_shadow_size = |cx: &Cx<'a, '_>, value: &'a str| {
        let mut out = ArenaString::new_in(cx.alloc);
        for (i, part) in segment(value, b',').into_iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str("drop-shadow(");
            out.push_str(replace_shadow_colors(part, cx.alloc, &mut |color| cx.cat(&["var(", "--tw-drop-shadow-color", ", ", color, ")"])));
            out.push(')');
        }
        out.into_bump_str()
    };
    let drop_shadows = |cx: &Cx<'a, '_>, resolved: &str| {
        let parts: std::vec::Vec<String> = segment(resolved, b',').into_iter().map(|v| format!("drop-shadow({v})")).collect();
        cx.str(&parts.join(" "))
    };
    r.functional_decl(
        "drop-shadow",
        |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            let alpha = match modifier {
                Some(Modifier::Arbitrary(v)) => Some(v),
                Some(Modifier::Named(v)) if is_multiple_of_quarter(v) => Some(cx.cat(&[v, "%"])),
                _ => None,
            };
            let sized = |value: &'a str, shadow: &'a str| {
                let mut nodes = Vec::with_capacity_in(6, cx.alloc);
                nodes.push(filter_properties(cx));
                nodes.push(cx.decl_opt("--tw-drop-shadow-alpha", alpha));
                push_alpha_replaced_drop_shadow(cx, &mut nodes, "--tw-drop-shadow-size", value, alpha, "--tw-drop-shadow-color");
                nodes.push(cx.decl("--tw-drop-shadow", shadow));
                nodes.push(cx.decl("filter", FILTER_VALUE));
                Some(nodes)
            };
            let drop_shadows = |resolved: &str| {
                let parts: std::vec::Vec<String> = segment(resolved, b',').into_iter().map(|v| format!("drop-shadow({v})")).collect();
                cx.str(&parts.join(" "))
            };
            let colored = |color: &'a str| {
                Some(cx.nodes([
                    filter_properties(cx),
                    cx.decl("--tw-drop-shadow-color", color),
                    cx.decl("--tw-drop-shadow", "var(--tw-drop-shadow-size)"),
                ]))
            };
            let Some(value) = value else {
                let value = cx.theme.get(&["--drop-shadow"])?;
                let resolved = cx.resolve(None, &["--drop-shadow"])?;
                return sized(value, drop_shadows(resolved));
            };
            let value = match value {
                UtilityValue::Arbitrary { data_type, value } => {
                    if arbitrary_type(data_type, value, &[DataType::Color]) == Some("color") {
                        let color = cx.as_color(value, modifier)?;
                        return colored(cx.with_alpha(color, "var(--tw-drop-shadow-alpha)"));
                    }
                    if modifier.is_some() && alpha.is_none() {
                        return None;
                    }
                    return sized(value, "var(--tw-drop-shadow-size)");
                }
                UtilityValue::Named { value, .. } => value,
            };
            let theme_value = cx.theme.get(&[&format!("--drop-shadow-{value}")]).filter(|v| !v.is_empty());
            let resolved = cx.resolve(Some(value), &["--drop-shadow"]).filter(|v| !v.is_empty());
            if let (Some(theme_value), Some(resolved)) = (theme_value, resolved) {
                if modifier.is_some() && alpha.is_none() {
                    return None;
                }
                if alpha.is_some() {
                    return sized(theme_value, "var(--tw-drop-shadow-size)");
                }
                return sized(theme_value, drop_shadows(resolved));
            }
            let color = cx.resolve_theme_color(value, modifier, &["--drop-shadow-color", "--color"])?;
            if color == "inherit" {
                return colored("inherit");
            }
            colored(cx.with_alpha(color, "var(--tw-drop-shadow-alpha)"))
        },
        move |cx, c| {
            let CandidateKind::Functional { value, .. } = c.kind else {
                return None;
            };
            let sized = |cx: &Cx<'a, '_>, value: &'a str, shadow: &'a str| {
                Vec::from_iter_in(
                    [("--tw-drop-shadow-size", drop_shadow_size(cx, value)), ("--tw-drop-shadow", shadow), ("filter", FILTER_VALUE)],
                    cx.alloc,
                )
            };
            let colored = |cx: &Cx<'a, '_>, color: &'a str| {
                Vec::from_iter_in([("--tw-drop-shadow-color", color), ("--tw-drop-shadow", "var(--tw-drop-shadow-size)")], cx.alloc)
            };
            let Some(value) = value else {
                let theme_value = cx.theme.get(&["--drop-shadow"])?;
                let resolved = cx.resolve(None, &["--drop-shadow"])?;
                return Some(sized(cx, theme_value, drop_shadows(cx, resolved)));
            };
            let value = match value {
                UtilityValue::Arbitrary { data_type, value } => {
                    if arbitrary_type(data_type, value, &[DataType::Color]) == Some("color") {
                        let color = cx.as_color(value, None)?;
                        return Some(colored(cx, cx.with_alpha(color, "var(--tw-drop-shadow-alpha)")));
                    }
                    return Some(sized(cx, value, "var(--tw-drop-shadow-size)"));
                }
                UtilityValue::Named { value, .. } => value,
            };
            let theme_value = cx.theme.get(&[&format!("--drop-shadow-{value}")]).filter(|v| !v.is_empty());
            let resolved = cx.resolve(Some(value), &["--drop-shadow"]).filter(|v| !v.is_empty());
            if let (Some(theme_value), Some(resolved)) = (theme_value, resolved) {
                return Some(sized(cx, theme_value, drop_shadows(cx, resolved)));
            }
            let color = cx.resolve_theme_color(value, None, &["--drop-shadow-color", "--color"])?;
            if color == "inherit" {
                return Some(colored(cx, "inherit"));
            }
            Some(colored(cx, cx.with_alpha(color, "var(--tw-drop-shadow-alpha)")))
        },
    );
    r.suggest("drop-shadow", vec![Suggest::color(&["--drop-shadow-color", "--color"]), Suggest::default().keys(&["--drop-shadow"])]);

    r.functional(
        "backdrop-opacity",
        Functional::new(|cx, v, _| backdrop(cx, "--tw-backdrop-opacity", cx.cat(&["opacity(", v, ")"])))
            .keys(&["--backdrop-opacity", "--opacity"])
            .bare(|cx, n| is_multiple_of_quarter(n.value).then(|| cx.cat(&[n.value, "%"]))),
    );
    r.suggest("backdrop-opacity", vec![Suggest::values(&STEPS_OF_5).keys(&["--backdrop-opacity", "--opacity"])]);
}
