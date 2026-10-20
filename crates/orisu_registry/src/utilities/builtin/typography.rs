use super::{NEG, PLAIN, POSITIONS, arbitrary_type};
use crate::utilities::{Functional, Registrar, Suggest};
use orisu_allocator::Vec;
use orisu_ast::Node;
use orisu_candidate::{CandidateKind, UtilityValue};
use orisu_parser::infer::{DataType, is_positive_integer, js_number};

const BLEND_MODES: [&str; 16] = [
    "normal",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color-dodge",
    "color-burn",
    "hard-light",
    "soft-light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
];

pub(super) fn register<'a>(r: &mut Registrar<'a, '_>) {
    for value in ["slice", "clone"] {
        r.static_decls(&format!("box-decoration-{value}"), &[("-webkit-box-decoration-break", value), ("box-decoration-break", value)]);
    }
    r.static_decls("bg-clip-text", &[("background-clip", "text")]);
    for value in ["border", "padding", "content"] {
        r.static_decls(&format!("bg-clip-{value}"), &[("background-clip", format!("{value}-box").as_str())]);
    }
    for value in ["border", "padding", "content"] {
        r.static_decls(&format!("bg-origin-{value}"), &[("background-origin", format!("{value}-box").as_str())]);
    }
    for value in BLEND_MODES {
        r.static_decls(&format!("bg-blend-{value}"), &[("background-blend-mode", value)]);
        r.static_decls(&format!("mix-blend-{value}"), &[("mix-blend-mode", value)]);
    }
    r.static_decls("mix-blend-plus-darker", &[("mix-blend-mode", "plus-darker")]);
    r.static_decls("mix-blend-plus-lighter", &[("mix-blend-mode", "plus-lighter")]);

    r.static_decls("fill-none", &[("fill", "none")]);
    r.functional_decl(
        "fill",
        |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            let value = match value? {
                UtilityValue::Arbitrary { value, .. } => cx.as_color(value, modifier),
                UtilityValue::Named { value, .. } => cx.resolve_theme_color(value, modifier, &["--fill", "--color"]),
            };
            cx.one("fill", value?)
        },
        |cx, c| {
            let CandidateKind::Functional { value, .. } = c.kind else {
                return None;
            };
            let value = match value? {
                UtilityValue::Arbitrary { value, .. } => cx.as_color(value, None),
                UtilityValue::Named { value, .. } => cx.resolve_theme_color(value, None, &["--fill", "--color"]),
            };
            Some(Vec::from_iter_in([("fill", value?)], cx.alloc))
        },
    );
    r.suggest("fill", vec![Suggest::color(&["--fill", "--color"])]);

    r.static_decls("stroke-none", &[("stroke", "none")]);
    r.functional_decl(
        "stroke",
        |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            match value? {
                UtilityValue::Arbitrary { data_type, value } => {
                    match arbitrary_type(data_type, value, &[DataType::Color, DataType::Number, DataType::Length, DataType::Percentage]) {
                        Some("number" | "length" | "percentage") => {
                            if modifier.is_some() {
                                return None;
                            }
                            cx.one("stroke-width", value)
                        }
                        _ => cx.one("stroke", cx.as_color(value, modifier)?),
                    }
                }
                UtilityValue::Named { value, .. } => {
                    if let Some(v) = cx.resolve_theme_color(value, modifier, &["--stroke", "--color"]) {
                        return cx.one("stroke", v);
                    }
                    if let Some(v) = cx.resolve(Some(value), &["--stroke-width"]).filter(|v| !v.is_empty()) {
                        return cx.one("stroke-width", v);
                    }
                    is_positive_integer(value).then(|| cx.one("stroke-width", value))?
                }
            }
        },
        |cx, c| {
            let CandidateKind::Functional { value, .. } = c.kind else {
                return None;
            };
            match value? {
                UtilityValue::Arbitrary { data_type, value } => {
                    match arbitrary_type(data_type, value, &[DataType::Color, DataType::Number, DataType::Length, DataType::Percentage]) {
                        Some("number" | "length" | "percentage") => Some(Vec::from_iter_in([("stroke-width", value)], cx.alloc)),
                        _ => {
                            let color = cx.as_color(value, None)?;
                            Some(Vec::from_iter_in([("stroke", color)], cx.alloc))
                        }
                    }
                }
                UtilityValue::Named { value, .. } => {
                    if let Some(color) = cx.resolve_theme_color(value, None, &["--stroke", "--color"]) {
                        return Some(Vec::from_iter_in([("stroke", color)], cx.alloc));
                    }
                    if let Some(v) = cx.resolve(Some(value), &["--stroke-width"]).filter(|v| !v.is_empty()) {
                        return Some(Vec::from_iter_in([("stroke-width", v)], cx.alloc));
                    }
                    is_positive_integer(value).then(|| Vec::from_iter_in([("stroke-width", value)], cx.alloc))
                }
            }
        },
    );
    r.suggest("stroke", vec![Suggest::color(&["--stroke", "--color"]), Suggest::values(&["0", "1", "2", "3"]).keys(&["--stroke-width"])]);

    for value in ["contain", "cover", "fill", "none", "scale-down"] {
        r.static_decls(&format!("object-{value}"), &[("object-fit", value)]);
    }
    let decls: std::vec::Vec<[(&str, &str); 1]> = POSITIONS.iter().map(|&(_, v)| [("object-position", v)]).collect();
    let entries: std::vec::Vec<(&str, &[(&str, &str)])> = POSITIONS.iter().zip(&decls).map(|(&(k, _), d)| (k, &d[..])).collect();
    let statics = r.statics(&entries);
    r.functional("object", Functional::decl("object-position").keys(&["--object-position"]).statics(statics));

    for (name, property) in [
        ("p", "padding"),
        ("px", "padding-inline"),
        ("py", "padding-block"),
        ("ps", "padding-inline-start"),
        ("pe", "padding-inline-end"),
        ("pbs", "padding-block-start"),
        ("pbe", "padding-block-end"),
        ("pt", "padding-top"),
        ("pr", "padding-right"),
        ("pb", "padding-bottom"),
        ("pl", "padding-left"),
    ] {
        r.spacing_decl(name, &["--padding", "--spacing"], PLAIN, property);
    }

    for value in ["left", "center", "right", "justify", "start", "end"] {
        r.static_decls(&format!("text-{value}"), &[("text-align", value)]);
    }
    r.spacing_decl("indent", &["--text-indent", "--spacing"], NEG, "text-indent");
    for value in ["baseline", "top", "middle", "bottom", "text-top", "text-bottom", "sub", "super"] {
        r.static_decls(&format!("align-{value}"), &[("vertical-align", value)]);
    }
    r.functional("align", Functional::decl("vertical-align"));

    r.functional_fn("font", |cx, c| {
        let CandidateKind::Functional { value, modifier: None, .. } = c.kind else {
            return None;
        };
        let weight = |v| {
            Some(cx.nodes([
                cx.at_root([cx.property("--tw-font-weight", None, None)]),
                cx.decl("--tw-font-weight", v),
                cx.decl("font-weight", v),
            ]))
        };
        match value? {
            UtilityValue::Arbitrary { data_type, value } => {
                match arbitrary_type(data_type, value, &[DataType::Number, DataType::GenericName, DataType::FamilyName]) {
                    Some("generic-name" | "family-name") => cx.one("font-family", value),
                    _ => weight(value),
                }
            }
            UtilityValue::Named { value, .. } => {
                if let Some((families, options)) =
                    cx.theme.resolve_with(value, &["--font"], &["--font-feature-settings", "--font-variation-settings"], cx.options)
                {
                    let option = |key| options.iter().find(|(name, _)| *name == key).map(|&(_, v)| v);
                    return Some(cx.nodes([
                        cx.decl("font-family", families),
                        cx.decl_opt("font-feature-settings", option("--font-feature-settings")),
                        cx.decl_opt("font-variation-settings", option("--font-variation-settings")),
                    ]));
                }
                weight(cx.resolve(Some(value), &["--font-weight"]).filter(|v| !v.is_empty())?)
            }
        }
    });
    r.suggest("font", vec![Suggest::default().keys(&["--font"]), Suggest::default().keys(&["--font-weight"])]);
    r.functional("font-features", Functional::decl("font-feature-settings"));

    r.static_decls("uppercase", &[("text-transform", "uppercase")]);
    r.static_decls("lowercase", &[("text-transform", "lowercase")]);
    r.static_decls("capitalize", &[("text-transform", "capitalize")]);
    r.static_decls("normal-case", &[("text-transform", "none")]);
    r.static_decls("italic", &[("font-style", "italic")]);
    r.static_decls("not-italic", &[("font-style", "normal")]);
    r.static_decls("underline", &[("text-decoration-line", "underline")]);
    r.static_decls("overline", &[("text-decoration-line", "overline")]);
    r.static_decls("line-through", &[("text-decoration-line", "line-through")]);
    r.static_decls("no-underline", &[("text-decoration-line", "none")]);
    for value in [
        "normal",
        "ultra-condensed",
        "extra-condensed",
        "condensed",
        "semi-condensed",
        "semi-expanded",
        "expanded",
        "extra-expanded",
        "ultra-expanded",
    ] {
        r.static_decls(&format!("font-stretch-{value}"), &[("font-stretch", value)]);
    }
    r.functional(
        "font-stretch",
        Functional::decl("font-stretch").bare(|_, n| {
            let n_value = js_number(n.value.strip_suffix('%')?);
            (n_value.fract() == 0.0 && (50.0..=200.0).contains(&n_value)).then_some(n.value)
        }),
    );
    r.suggest("font-stretch", vec![Suggest::values(&["50%", "75%", "90%", "95%", "100%", "105%", "110%", "125%", "150%", "200%"])]);

    r.color("placeholder", &["--placeholder-color", "--color"], |cx, v| {
        Some(cx.nodes([Node::Rule {
            selector: "&::placeholder",
            nodes: cx.nodes([cx.decl("--tw-sort", "placeholder-color"), cx.decl("color", v)]),
        }]))
    });

    for value in ["solid", "double", "dotted", "dashed", "wavy"] {
        r.static_decls(&format!("decoration-{value}"), &[("text-decoration-style", value)]);
    }
    r.static_decls("decoration-auto", &[("text-decoration-thickness", "auto")]);
    r.static_decls("decoration-from-font", &[("text-decoration-thickness", "from-font")]);
    r.functional_decl(
        "decoration",
        |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            match value? {
                UtilityValue::Arbitrary { data_type, value } => {
                    match arbitrary_type(data_type, value, &[DataType::Color, DataType::Length, DataType::Percentage]) {
                        Some("length" | "percentage") => {
                            if modifier.is_some() {
                                return None;
                            }
                            cx.one("text-decoration-thickness", value)
                        }
                        _ => cx.one("text-decoration-color", cx.as_color(value, modifier)?),
                    }
                }
                UtilityValue::Named { value, .. } => {
                    let thickness = cx
                        .resolve(Some(value), &["--text-decoration-thickness"])
                        .filter(|v| !v.is_empty())
                        .or_else(|| is_positive_integer(value).then(|| cx.cat(&[value, "px"])));
                    if let Some(thickness) = thickness {
                        if modifier.is_some() {
                            return None;
                        }
                        return cx.one("text-decoration-thickness", thickness);
                    }
                    cx.one("text-decoration-color", cx.resolve_theme_color(value, modifier, &["--text-decoration-color", "--color"])?)
                }
            }
        },
        |cx, c| {
            let CandidateKind::Functional { value, .. } = c.kind else {
                return None;
            };
            match value? {
                UtilityValue::Arbitrary { data_type, value } => {
                    match arbitrary_type(data_type, value, &[DataType::Color, DataType::Length, DataType::Percentage]) {
                        Some("length" | "percentage") => Some(Vec::from_iter_in([("text-decoration-thickness", value)], cx.alloc)),
                        _ => {
                            let color = cx.as_color(value, None)?;
                            Some(Vec::from_iter_in([("text-decoration-color", color)], cx.alloc))
                        }
                    }
                }
                UtilityValue::Named { value, .. } => {
                    let thickness = cx
                        .resolve(Some(value), &["--text-decoration-thickness"])
                        .filter(|v| !v.is_empty())
                        .or_else(|| is_positive_integer(value).then(|| cx.cat(&[value, "px"])));
                    if let Some(thickness) = thickness {
                        return Some(Vec::from_iter_in([("text-decoration-thickness", thickness)], cx.alloc));
                    }
                    let color = cx.resolve_theme_color(value, None, &["--text-decoration-color", "--color"])?;
                    Some(Vec::from_iter_in([("text-decoration-color", color)], cx.alloc))
                }
            }
        },
    );
    r.suggest(
        "decoration",
        vec![
            Suggest::color(&["--text-decoration-color", "--color"]),
            Suggest::values(&["0", "1", "2"]).keys(&["--text-decoration-thickness"]),
        ],
    );

    let none = r.statics(&[("none", &[("animation", "none")])]);
    r.functional("animate", Functional::decl("animation").keys(&["--animate"]).statics(none));
}
