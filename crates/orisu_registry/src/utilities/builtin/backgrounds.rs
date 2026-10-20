use super::{POSITIONS, REPEATS, Stop, arbitrary_type};
use crate::utilities::{Cx, Functional, PERCENT_STEPS_OF_5, Registrar, Suggest};
use orisu_allocator::Vec;
use orisu_ast::{Node, Nodes};
use orisu_candidate::{Candidate, CandidateKind, Modifier, UtilityValue};
use orisu_parser::infer::{DataType, is_positive_integer};

const INTERPOLATION_MODIFIERS: [&str; 8] = ["oklab", "oklch", "srgb", "hsl", "longer", "shorter", "increasing", "decreasing"];

const LINEAR_DIRECTIONS: [(&str, &str); 8] = [
    ("to-t", "to top"),
    ("to-tr", "to top right"),
    ("to-r", "to right"),
    ("to-br", "to bottom right"),
    ("to-b", "to bottom"),
    ("to-bl", "to bottom left"),
    ("to-l", "to left"),
    ("to-tl", "to top left"),
];

const GRADIENT_ANGLES: [&str; 12] = ["0", "30", "60", "90", "120", "150", "180", "210", "240", "270", "300", "330"];

const GRADIENT_STOPS: &str = "var(--tw-gradient-via-stops, var(--tw-gradient-position), var(--tw-gradient-from) var(--tw-gradient-from-position), var(--tw-gradient-to) var(--tw-gradient-to-position))";

const GRADIENT_VIA_STOPS: &str = "var(--tw-gradient-position), var(--tw-gradient-from) var(--tw-gradient-from-position), var(--tw-gradient-via) var(--tw-gradient-via-position), var(--tw-gradient-to) var(--tw-gradient-to-position)";

fn interpolation<'a>(cx: &Cx<'a, '_>, modifier: Option<Modifier<'a>>) -> &'a str {
    match modifier {
        Some(Modifier::Named(v @ ("longer" | "shorter" | "increasing" | "decreasing"))) => cx.cat(&["in oklch ", v, " hue"]),
        Some(Modifier::Named(v)) => cx.cat(&["in ", v]),
        Some(Modifier::Arbitrary(v)) => v,
        None => "in oklab",
    }
}

fn gradient<'a>(cx: &Cx<'a, '_>, position: &'a str, image: &'a str) -> Option<Nodes<'a>> {
    Some(cx.nodes([cx.decl("--tw-gradient-position", position), cx.decl("background-image", image)]))
}

fn gradient_stop_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-gradient-position", None, None),
        cx.property("--tw-gradient-from", Some("#0000"), Some("<color>")),
        cx.property("--tw-gradient-via", Some("#0000"), Some("<color>")),
        cx.property("--tw-gradient-to", Some("#0000"), Some("<color>")),
        cx.property("--tw-gradient-stops", None, None),
        cx.property("--tw-gradient-via-stops", None, None),
        cx.property("--tw-gradient-from-position", Some("0%"), Some("<length-percentage>")),
        cx.property("--tw-gradient-via-position", Some("50%"), Some("<length-percentage>")),
        cx.property("--tw-gradient-to-position", Some("100%"), Some("<length-percentage>")),
    ])
}

fn resolve_gradient_stop<'a>(cx: &Cx<'a, '_>, candidate: &Candidate<'a>) -> Option<(&'a str, Stop)> {
    let CandidateKind::Functional { value, modifier, .. } = candidate.kind else {
        return None;
    };
    match value? {
        UtilityValue::Arbitrary { data_type, value } => {
            match arbitrary_type(data_type, value, &[DataType::Color, DataType::Length, DataType::Percentage]) {
                Some("length" | "percentage") => {
                    if modifier.is_some() {
                        return None;
                    }
                    Some((value, Stop::Position))
                }
                _ => Some((cx.as_color(value, modifier)?, Stop::Color)),
            }
        }
        UtilityValue::Named { value, .. } => {
            if let Some(v) = cx.resolve_theme_color(value, modifier, &["--background-color", "--color"]) {
                return Some((v, Stop::Color));
            }
            if modifier.is_some() {
                return None;
            }
            if let Some(v) = cx.resolve(Some(value), &["--gradient-color-stop-positions"]).filter(|v| !v.is_empty()) {
                return Some((v, Stop::Position));
            }
            if !value.strip_suffix('%').is_some_and(is_positive_integer) {
                return None;
            }
            Some((value, Stop::Position))
        }
    }
}

fn gradient_stop<'a>(
    r: &mut Registrar<'a, '_>,
    name: &str,
    handle: impl for<'t> Fn(&Cx<'a, 't>, &'a str, Stop) -> Option<Nodes<'a>> + Send + Sync + 'a,
    declarations: impl for<'t> Fn(&Cx<'a, 't>, &'a str, Stop) -> Vec<'a, (&'a str, &'a str)> + Send + Sync + 'a,
) {
    r.functional_decl(
        name,
        move |cx, candidate| {
            let (value, stop) = resolve_gradient_stop(cx, candidate)?;
            handle(cx, value, stop)
        },
        move |cx, candidate| {
            let (value, stop) = resolve_gradient_stop(cx, candidate)?;
            Some(declarations(cx, value, stop))
        },
    );
    r.suggest(
        name,
        vec![
            Suggest::color(&["--background-color", "--color"]),
            Suggest::values(&PERCENT_STEPS_OF_5).keys(&["--gradient-color-stop-positions"]),
        ],
    );
}

pub(super) fn register<'a>(r: &mut Registrar<'a, '_>) {
    for value in ["auto", "cover", "contain"] {
        r.static_decls(&format!("bg-{value}"), &[("background-size", value)]);
    }
    r.functional("bg-size", Functional::decl("background-size"));
    for value in ["fixed", "local", "scroll"] {
        r.static_decls(&format!("bg-{value}"), &[("background-attachment", value)]);
    }
    for (name, value) in POSITIONS {
        r.static_decls(&format!("bg-{name}"), &[("background-position", value)]);
    }
    r.functional("bg-position", Functional::decl("background-position"));
    for (name, value) in REPEATS {
        r.static_decls(&format!("bg-{name}"), &[("background-repeat", value)]);
    }
    r.static_decls("bg-none", &[("background-image", "none")]);

    for negative in [true, false] {
        r.functional_fn(if negative { "-bg-linear" } else { "bg-linear" }, move |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            let value = match value? {
                UtilityValue::Arbitrary { data_type, value } => {
                    if modifier.is_some() {
                        return None;
                    }
                    let value = match arbitrary_type(data_type, value, &[DataType::Angle]) {
                        Some("angle") if negative => cx.negate(value),
                        Some("angle") => value,
                        _ if negative => return None,
                        _ => value,
                    };
                    return gradient(cx, value, cx.cat(&["linear-gradient(var(--tw-gradient-stops,", value, "))"]));
                }
                UtilityValue::Named { value, .. } => match LINEAR_DIRECTIONS.iter().find(|(k, _)| *k == value) {
                    Some(&(_, direction)) if !negative => direction,
                    _ if is_positive_integer(value) => {
                        if negative {
                            cx.cat(&["calc(", value, "deg * -1)"])
                        } else {
                            cx.cat(&[value, "deg"])
                        }
                    }
                    _ => return None,
                },
            };
            let method = interpolation(cx, modifier);
            Some(cx.nodes([
                cx.decl("--tw-gradient-position", value),
                Node::AtRule {
                    name: "@supports",
                    params: "(background-image: linear-gradient(in lab, red, red))",
                    nodes: cx.nodes([cx.decl("--tw-gradient-position", cx.cat(&[value, " ", method]))]),
                },
                cx.decl("background-image", "linear-gradient(var(--tw-gradient-stops))"),
            ]))
        });
    }
    r.suggest(
        "bg-linear",
        vec![
            Suggest::values(r.keys(&LINEAR_DIRECTIONS.map(|(k, _)| k))).modifiers(&INTERPOLATION_MODIFIERS),
            Suggest::values(&GRADIENT_ANGLES).negative(true).modifiers(&INTERPOLATION_MODIFIERS),
        ],
    );

    for negative in [true, false] {
        r.functional_fn(if negative { "-bg-conic" } else { "bg-conic" }, move |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            match value {
                Some(UtilityValue::Arbitrary { value, .. }) => {
                    if modifier.is_some() {
                        return None;
                    }
                    gradient(cx, value, cx.cat(&["conic-gradient(var(--tw-gradient-stops,", value, "))"]))
                }
                None => gradient(cx, interpolation(cx, modifier), "conic-gradient(var(--tw-gradient-stops))"),
                Some(UtilityValue::Named { value, .. }) => {
                    if !is_positive_integer(value) {
                        return None;
                    }
                    let method = interpolation(cx, modifier);
                    let position = if negative {
                        cx.cat(&["from calc(", value, "deg * -1) ", method])
                    } else {
                        cx.cat(&["from ", value, "deg ", method])
                    };
                    gradient(cx, position, "conic-gradient(var(--tw-gradient-stops))")
                }
            }
        });
    }
    r.suggest(
        "bg-conic",
        vec![
            Suggest::default().default_value(true).modifiers(&INTERPOLATION_MODIFIERS),
            Suggest::values(&GRADIENT_ANGLES).negative(true).modifiers(&INTERPOLATION_MODIFIERS),
        ],
    );

    r.functional_fn("bg-radial", |cx, c| {
        let CandidateKind::Functional { value, modifier, .. } = c.kind else {
            return None;
        };
        match value {
            None => gradient(cx, interpolation(cx, modifier), "radial-gradient(var(--tw-gradient-stops))"),
            Some(UtilityValue::Arbitrary { value, .. }) => {
                if modifier.is_some() {
                    return None;
                }
                gradient(cx, value, cx.cat(&["radial-gradient(var(--tw-gradient-stops,", value, "))"]))
            }
            Some(UtilityValue::Named { .. }) => None,
        }
    });
    r.suggest("bg-radial", vec![Suggest::default().default_value(true).modifiers(&INTERPOLATION_MODIFIERS)]);

    r.functional_decl(
        "bg",
        |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            match value? {
                UtilityValue::Arbitrary { data_type, value } => {
                    let property = match arbitrary_type(
                        data_type,
                        value,
                        &[
                            DataType::Image,
                            DataType::Color,
                            DataType::Percentage,
                            DataType::Position,
                            DataType::BgSize,
                            DataType::Length,
                            DataType::Url,
                        ],
                    ) {
                        Some("percentage" | "position") => "background-position",
                        Some("bg-size" | "length" | "size") => "background-size",
                        Some("image" | "url") => "background-image",
                        _ => return cx.one("background-color", cx.as_color(value, modifier)?),
                    };
                    if modifier.is_some() {
                        return None;
                    }
                    cx.one(property, value)
                }
                UtilityValue::Named { value, .. } => {
                    if let Some(v) = cx.resolve_theme_color(value, modifier, &["--background-color", "--color"]) {
                        return cx.one("background-color", v);
                    }
                    if modifier.is_some() {
                        return None;
                    }
                    cx.one("background-image", cx.resolve(Some(value), &["--background-image"]).filter(|v| !v.is_empty())?)
                }
            }
        },
        |cx, c| {
            let CandidateKind::Functional { value, .. } = c.kind else {
                return None;
            };
            match value? {
                UtilityValue::Arbitrary { data_type, value } => {
                    let property = match arbitrary_type(
                        data_type,
                        value,
                        &[
                            DataType::Image,
                            DataType::Color,
                            DataType::Percentage,
                            DataType::Position,
                            DataType::BgSize,
                            DataType::Length,
                            DataType::Url,
                        ],
                    ) {
                        Some("percentage" | "position") => "background-position",
                        Some("bg-size" | "length" | "size") => "background-size",
                        Some("image" | "url") => "background-image",
                        _ => {
                            let color = cx.as_color(value, None)?;
                            return Some(Vec::from_iter_in([("background-color", color)], cx.alloc));
                        }
                    };
                    Some(Vec::from_iter_in([(property, value)], cx.alloc))
                }
                UtilityValue::Named { value, .. } => {
                    if let Some(color) = cx.resolve_theme_color(value, None, &["--background-color", "--color"]) {
                        return Some(Vec::from_iter_in([("background-color", color)], cx.alloc));
                    }
                    let image = cx.resolve(Some(value), &["--background-image"]).filter(|v| !v.is_empty())?;
                    Some(Vec::from_iter_in([("background-image", image)], cx.alloc))
                }
            }
        },
    );
    r.suggest("bg", vec![Suggest::color(&["--background-color", "--color"]), Suggest::default().keys(&["--background-image"])]);

    gradient_stop(
        r,
        "from",
        |cx, v, stop| match stop {
            Stop::Color => Some(cx.nodes([
                gradient_stop_properties(cx),
                cx.decl("--tw-sort", "--tw-gradient-from"),
                cx.decl("--tw-gradient-from", v),
                cx.decl("--tw-gradient-stops", GRADIENT_STOPS),
            ])),
            Stop::Position => Some(cx.nodes([gradient_stop_properties(cx), cx.decl("--tw-gradient-from-position", v)])),
        },
        |cx, v, stop| match stop {
            Stop::Color => Vec::from_iter_in([("--tw-gradient-from", v), ("--tw-gradient-stops", GRADIENT_STOPS)], cx.alloc),
            Stop::Position => Vec::from_iter_in([("--tw-gradient-from-position", v)], cx.alloc),
        },
    );
    r.static_decls("via-none", &[("--tw-gradient-via-stops", "initial")]);
    gradient_stop(
        r,
        "via",
        |cx, v, stop| match stop {
            Stop::Color => Some(cx.nodes([
                gradient_stop_properties(cx),
                cx.decl("--tw-sort", "--tw-gradient-via"),
                cx.decl("--tw-gradient-via", v),
                cx.decl("--tw-gradient-via-stops", GRADIENT_VIA_STOPS),
                cx.decl("--tw-gradient-stops", "var(--tw-gradient-via-stops)"),
            ])),
            Stop::Position => Some(cx.nodes([gradient_stop_properties(cx), cx.decl("--tw-gradient-via-position", v)])),
        },
        |cx, v, stop| match stop {
            Stop::Color => Vec::from_iter_in(
                [
                    ("--tw-gradient-via", v),
                    ("--tw-gradient-via-stops", GRADIENT_VIA_STOPS),
                    ("--tw-gradient-stops", "var(--tw-gradient-via-stops)"),
                ],
                cx.alloc,
            ),
            Stop::Position => Vec::from_iter_in([("--tw-gradient-via-position", v)], cx.alloc),
        },
    );
    gradient_stop(
        r,
        "to",
        |cx, v, stop| match stop {
            Stop::Color => Some(cx.nodes([
                gradient_stop_properties(cx),
                cx.decl("--tw-sort", "--tw-gradient-to"),
                cx.decl("--tw-gradient-to", v),
                cx.decl("--tw-gradient-stops", GRADIENT_STOPS),
            ])),
            Stop::Position => Some(cx.nodes([gradient_stop_properties(cx), cx.decl("--tw-gradient-to-position", v)])),
        },
        |cx, v, stop| match stop {
            Stop::Color => Vec::from_iter_in([("--tw-gradient-to", v), ("--tw-gradient-stops", GRADIENT_STOPS)], cx.alloc),
            Stop::Position => Vec::from_iter_in([("--tw-gradient-to-position", v)], cx.alloc),
        },
    );
}
