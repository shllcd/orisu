use super::{POSITIONS, REPEATS, Stop, arbitrary_type};
use crate::utilities::{Cx, DefaultValue, Functional, Named, PERCENT_STEPS_OF_5, Registrar, Suggest, declaration_fn};
use orisu_allocator::Vec;
use orisu_ast::{Node, Nodes};
use orisu_candidate::{Candidate, CandidateKind, UtilityValue};
use orisu_parser::infer::{DataType, infer_data_type, is_multiple_of_quarter, is_positive_integer, js_number};

const MASK_IMAGE: &str = "var(--tw-mask-linear), var(--tw-mask-radial), var(--tw-mask-conic)";
const ANGLES: [&str; 9] = ["0", "1", "2", "3", "6", "12", "45", "90", "180"];

fn drop_last(value: &str) -> &str {
    value.char_indices().next_back().map_or("", |(i, _)| &value[..i])
}

fn mask_properties_gradient<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-mask-linear", Some("linear-gradient(#fff, #fff)"), None),
        cx.property("--tw-mask-radial", Some("linear-gradient(#fff, #fff)"), None),
        cx.property("--tw-mask-conic", Some("linear-gradient(#fff, #fff)"), None),
    ])
}

fn mask_properties_edge<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-mask-left", Some("linear-gradient(#fff, #fff)"), None),
        cx.property("--tw-mask-right", Some("linear-gradient(#fff, #fff)"), None),
        cx.property("--tw-mask-bottom", Some("linear-gradient(#fff, #fff)"), None),
        cx.property("--tw-mask-top", Some("linear-gradient(#fff, #fff)"), None),
    ])
}

fn mask_properties_linear<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-mask-linear-position", Some("0deg"), None),
        cx.property("--tw-mask-linear-from-position", Some("0%"), None),
        cx.property("--tw-mask-linear-to-position", Some("100%"), None),
        cx.property("--tw-mask-linear-from-color", Some("black"), None),
        cx.property("--tw-mask-linear-to-color", Some("transparent"), None),
    ])
}

fn mask_properties_radial<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-mask-radial-from-position", Some("0%"), None),
        cx.property("--tw-mask-radial-to-position", Some("100%"), None),
        cx.property("--tw-mask-radial-from-color", Some("black"), None),
        cx.property("--tw-mask-radial-to-color", Some("transparent"), None),
        cx.property("--tw-mask-radial-shape", Some("ellipse"), None),
        cx.property("--tw-mask-radial-size", Some("farthest-corner"), None),
        cx.property("--tw-mask-radial-position", Some("center"), None),
    ])
}

fn mask_properties_conic<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-mask-conic-position", Some("0deg"), None),
        cx.property("--tw-mask-conic-from-position", Some("0%"), None),
        cx.property("--tw-mask-conic-to-position", Some("100%"), None),
        cx.property("--tw-mask-conic-from-color", Some("black"), None),
        cx.property("--tw-mask-conic-to-color", Some("transparent"), None),
    ])
}

fn bare_angle<'a>(cx: &Cx<'a, '_>, n: Named<'a>) -> Option<&'a str> {
    if !is_positive_integer(n.value) {
        return None;
    }
    let n_value = js_number(n.value);
    Some(if n_value == 0.0 {
        "0deg"
    } else if n_value == 1.0 {
        "1deg"
    } else {
        cx.cat(&["calc(1deg * ", n.value, ")"])
    })
}

fn negative_bare_angle<'a>(cx: &Cx<'a, '_>, n: Named<'a>) -> Option<&'a str> {
    if !is_positive_integer(n.value) {
        return None;
    }
    let n_value = js_number(n.value);
    Some(if n_value == 0.0 {
        "0deg"
    } else if n_value == 1.0 {
        "-1deg"
    } else {
        cx.cat(&["calc(1deg * -", n.value, ")"])
    })
}

fn resolve_stop<'a>(cx: &Cx<'a, '_>, candidate: &Candidate<'a>) -> Option<(&'a str, Stop)> {
    let CandidateKind::Functional { value, modifier, .. } = candidate.kind else {
        return None;
    };
    match value? {
        UtilityValue::Arbitrary { data_type, value } => {
            match arbitrary_type(data_type, value, &[DataType::Length, DataType::Percentage, DataType::Color]) {
                Some("color") => Some((cx.as_color(value, modifier)?, Stop::Color)),
                ty => {
                    if modifier.is_some() || (ty == Some("percentage") && !is_positive_integer(drop_last(value))) {
                        return None;
                    }
                    Some((value, Stop::Position))
                }
            }
        }
        UtilityValue::Named { value, .. } => {
            if let Some(v) = cx.resolve_theme_color(value, modifier, &["--background-color", "--color"]) {
                return Some((v, Stop::Color));
            }
            if modifier.is_some() {
                return None;
            }
            match infer_data_type(value, &[DataType::Number, DataType::Percentage])? {
                DataType::Number => {
                    cx.resolve(None, &["--spacing"]).filter(|v| !v.is_empty())?;
                    if !is_multiple_of_quarter(value) {
                        return None;
                    }
                    Some((cx.cat(&["--spacing(", value, ")"]), Stop::Position))
                }
                _ => {
                    if !is_positive_integer(drop_last(value)) {
                        return None;
                    }
                    Some((value, Stop::Position))
                }
            }
        }
    }
}

fn mask_stop<'a>(
    r: &mut Registrar<'a, '_>,
    name: &str,
    handle: impl for<'t> Fn(&Cx<'a, 't>, &'a str, Stop) -> Option<Nodes<'a>> + Send + Sync + 'a,
    declarations: impl for<'t> Fn(&Cx<'a, 't>, &'a str, Stop) -> Vec<'a, (&'a str, &'a str)> + Send + Sync + 'a,
) {
    r.functional_decl(
        name,
        move |cx, candidate| {
            let (value, stop) = resolve_stop(cx, candidate)?;
            handle(cx, value, stop)
        },
        move |cx, candidate| {
            let (value, stop) = resolve_stop(cx, candidate)?;
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
    r.suggest(
        name,
        vec![
            Suggest::values(&PERCENT_STEPS_OF_5),
            Suggest { spacing: true, ..Suggest::default() },
            Suggest::color(&["--background-color", "--color"]),
        ],
    );
}

#[derive(Clone, Copy)]
struct Edge<'a> {
    var: &'a str,
    gradient: &'a str,
    from_position: &'a str,
    to_position: &'a str,
    from_color: &'a str,
    to_color: &'a str,
    color: &'a str,
    position: &'a str,
}

fn mask_edge<'a>(r: &mut Registrar<'a, '_>, name: &str, stop: &str, edges: &[&str]) {
    let edges: std::vec::Vec<Edge<'a>> = edges
        .iter()
        .map(|edge| {
            let var = r.cat(&["--tw-mask-", edge]);
            Edge {
                var,
                gradient: r.cat(&[
                    "linear-gradient(to ",
                    edge,
                    ", var(",
                    var,
                    "-from-color) var(",
                    var,
                    "-from-position), var(",
                    var,
                    "-to-color) var(",
                    var,
                    "-to-position))",
                ]),
                from_position: r.cat(&[var, "-from-position"]),
                to_position: r.cat(&[var, "-to-position"]),
                from_color: r.cat(&[var, "-from-color"]),
                to_color: r.cat(&[var, "-to-color"]),
                color: r.cat(&[var, "-", stop, "-color"]),
                position: r.cat(&[var, "-", stop, "-position"]),
            }
        })
        .collect();
    let declared_edges = edges.clone();
    mask_stop(
        r,
        name,
        move |cx, v, stop| {
            let mut nodes = Vec::with_capacity_in(5 + edges.len() * 3, cx.alloc);
            nodes.push(mask_properties_gradient(cx));
            nodes.push(mask_properties_edge(cx));
            nodes.push(cx.decl("mask-image", MASK_IMAGE));
            nodes.push(cx.decl("mask-composite", "intersect"));
            nodes.push(cx.decl("--tw-mask-linear", "var(--tw-mask-left), var(--tw-mask-right), var(--tw-mask-bottom), var(--tw-mask-top)"));
            for edge in &edges {
                nodes.push(cx.decl(edge.var, edge.gradient));
                nodes.push(cx.at_root([
                    cx.property(edge.from_position, Some("0%"), None),
                    cx.property(edge.to_position, Some("100%"), None),
                    cx.property(edge.from_color, Some("black"), None),
                    cx.property(edge.to_color, Some("transparent"), None),
                ]));
                nodes.push(cx.decl(
                    match stop {
                        Stop::Color => edge.color,
                        Stop::Position => edge.position,
                    },
                    v,
                ));
            }
            Some(nodes)
        },
        move |cx, v, stop| {
            let mut declarations = Vec::with_capacity_in(3 + declared_edges.len() * 2, cx.alloc);
            declarations.push(("mask-image", MASK_IMAGE));
            declarations.push(("mask-composite", "intersect"));
            declarations.push(("--tw-mask-linear", "var(--tw-mask-left), var(--tw-mask-right), var(--tw-mask-bottom), var(--tw-mask-top)"));
            for edge in &declared_edges {
                declarations.push((edge.var, edge.gradient));
                declarations.push((
                    match stop {
                        Stop::Color => edge.color,
                        Stop::Position => edge.position,
                    },
                    v,
                ));
            }
            declarations
        },
    );
}

fn mask_shape_stop<'a>(
    r: &mut Registrar<'a, '_>,
    shape: &str,
    stop: &str,
    properties: impl for<'t> Fn(&Cx<'a, 't>) -> Node<'a> + Send + Sync + 'a,
    stops: &'static str,
    function: &str,
) {
    let stops_var = r.cat(&["--tw-mask-", shape, "-stops"]);
    let var = r.cat(&["--tw-mask-", shape]);
    let image = r.cat(&[function, "(var(", stops_var, "))"]);
    let color = r.cat(&[var, "-", stop, "-color"]);
    let position = r.cat(&[var, "-", stop, "-position"]);
    mask_stop(
        r,
        &format!("mask-{shape}-{stop}"),
        move |cx, v, stop| {
            Some(cx.nodes([
                mask_properties_gradient(cx),
                properties(cx),
                cx.decl("mask-image", MASK_IMAGE),
                cx.decl("mask-composite", "intersect"),
                cx.decl(stops_var, stops),
                cx.decl(var, image),
                cx.decl(
                    match stop {
                        Stop::Color => color,
                        Stop::Position => position,
                    },
                    v,
                ),
            ]))
        },
        move |cx, v, stop| {
            Vec::from_iter_in(
                [
                    ("mask-image", MASK_IMAGE),
                    ("mask-composite", "intersect"),
                    (stops_var, stops),
                    (var, image),
                    (
                        match stop {
                            Stop::Color => color,
                            Stop::Position => position,
                        },
                        v,
                    ),
                ],
                cx.alloc,
            )
        },
    );
}

pub(super) fn register<'a>(r: &mut Registrar<'a, '_>) {
    r.static_decls("mask-none", &[("mask-image", "none")]);
    let mask_property = |data_type: Option<&'a str>, value: &'a str| match arbitrary_type(
        data_type,
        value,
        &[DataType::Image, DataType::Percentage, DataType::Position, DataType::BgSize, DataType::Length, DataType::Url],
    ) {
        Some("percentage" | "position") => "mask-position",
        Some("bg-size" | "length" | "size") => "mask-size",
        _ => "mask-image",
    };
    r.functional_decl(
        "mask",
        move |cx, c| {
            let CandidateKind::Functional { value: Some(UtilityValue::Arbitrary { data_type, value }), modifier: None, .. } = c.kind else {
                return None;
            };
            cx.one(mask_property(data_type, value), value)
        },
        move |cx, c| {
            let CandidateKind::Functional { value: Some(UtilityValue::Arbitrary { data_type, value }), modifier: None, .. } = c.kind else {
                return None;
            };
            Some(Vec::from_iter_in([(mask_property(data_type, value), value)], cx.alloc))
        },
    );
    for value in ["add", "subtract", "intersect", "exclude"] {
        r.static_decls(&format!("mask-{value}"), &[("mask-composite", value)]);
    }
    r.static_decls("mask-alpha", &[("mask-mode", "alpha")]);
    r.static_decls("mask-luminance", &[("mask-mode", "luminance")]);
    r.static_decls("mask-match", &[("mask-mode", "match-source")]);
    r.static_decls("mask-type-alpha", &[("mask-type", "alpha")]);
    r.static_decls("mask-type-luminance", &[("mask-type", "luminance")]);
    for value in ["auto", "cover", "contain"] {
        r.static_decls(&format!("mask-{value}"), &[("mask-size", value)]);
    }
    r.functional("mask-size", Functional::decl("mask-size"));
    for (name, value) in POSITIONS {
        r.static_decls(&format!("mask-{name}"), &[("mask-position", value)]);
    }
    r.functional("mask-position", Functional::decl("mask-position"));
    for (name, value) in REPEATS {
        r.static_decls(&format!("mask-{name}"), &[("mask-repeat", value)]);
    }
    for value in ["border", "padding", "content", "fill", "stroke", "view"] {
        r.static_decls(&format!("mask-clip-{value}"), &[("mask-clip", format!("{value}-box").as_str())]);
    }
    r.static_decls("mask-no-clip", &[("mask-clip", "no-clip")]);
    for value in ["border", "padding", "content", "fill", "stroke", "view"] {
        r.static_decls(&format!("mask-origin-{value}"), &[("mask-origin", format!("{value}-box").as_str())]);
    }

    for (axis, edges) in
        [("x", &["right", "left"][..]), ("y", &["top", "bottom"]), ("t", &["top"]), ("r", &["right"]), ("b", &["bottom"]), ("l", &["left"])]
    {
        for stop in ["from", "to"] {
            mask_edge(r, &format!("mask-{axis}-{stop}"), stop, edges);
        }
    }

    r.functional(
        "mask-linear",
        Functional::new(|cx, v, _| {
            Some(cx.nodes([
                mask_properties_gradient(cx),
                mask_properties_linear(cx),
                cx.decl("mask-image", MASK_IMAGE),
                cx.decl("mask-composite", "intersect"),
                cx.decl("--tw-mask-linear", "linear-gradient(var(--tw-mask-linear-stops, var(--tw-mask-linear-position)))"),
                cx.decl("--tw-mask-linear-position", v),
            ]))
        })
        .negative()
        .default_value(DefaultValue::Null)
        .bare(bare_angle)
        .negative_bare(negative_bare_angle)
        .declaration(declaration_fn(|cx, v, _| {
            Some(Vec::from_iter_in(
                [
                    ("mask-image", MASK_IMAGE),
                    ("mask-composite", "intersect"),
                    ("--tw-mask-linear", "linear-gradient(var(--tw-mask-linear-stops, var(--tw-mask-linear-position)))"),
                    ("--tw-mask-linear-position", v),
                ],
                cx.alloc,
            ))
        })),
    );
    r.suggest("mask-linear", vec![Suggest::values(&ANGLES).negative(true)]);
    const LINEAR_STOPS: &str = "var(--tw-mask-linear-position), var(--tw-mask-linear-from-color) var(--tw-mask-linear-from-position), var(--tw-mask-linear-to-color) var(--tw-mask-linear-to-position)";
    for stop in ["from", "to"] {
        mask_shape_stop(r, "linear", stop, mask_properties_linear, LINEAR_STOPS, "linear-gradient");
    }

    r.static_decls("mask-circle", &[("--tw-mask-radial-shape", "circle")]);
    r.static_decls("mask-ellipse", &[("--tw-mask-radial-shape", "ellipse")]);
    for value in ["closest-side", "farthest-side", "closest-corner", "farthest-corner"] {
        r.static_decls(&format!("mask-radial-{value}"), &[("--tw-mask-radial-size", value)]);
    }
    for value in ["top", "top-left", "top-right", "bottom", "bottom-left", "bottom-right", "left", "right", "center"] {
        r.static_decls(&format!("mask-radial-at-{value}"), &[("--tw-mask-radial-position", value.replace('-', " ").as_str())]);
    }
    r.functional("mask-radial-at", Functional::decl("--tw-mask-radial-position").default_value(DefaultValue::Null));
    r.functional(
        "mask-radial",
        Functional::new(|cx, v, _| {
            Some(cx.nodes([
                mask_properties_gradient(cx),
                mask_properties_radial(cx),
                cx.decl("mask-image", MASK_IMAGE),
                cx.decl("mask-composite", "intersect"),
                cx.decl("--tw-mask-radial", "radial-gradient(var(--tw-mask-radial-stops, var(--tw-mask-radial-size)))"),
                cx.decl("--tw-mask-radial-size", v),
            ]))
        })
        .default_value(DefaultValue::Null)
        .declaration(declaration_fn(|cx, v, _| {
            Some(Vec::from_iter_in(
                [
                    ("mask-image", MASK_IMAGE),
                    ("mask-composite", "intersect"),
                    ("--tw-mask-radial", "radial-gradient(var(--tw-mask-radial-stops, var(--tw-mask-radial-size)))"),
                    ("--tw-mask-radial-size", v),
                ],
                cx.alloc,
            ))
        })),
    );
    const RADIAL_STOPS: &str = "var(--tw-mask-radial-shape) var(--tw-mask-radial-size) at var(--tw-mask-radial-position), var(--tw-mask-radial-from-color) var(--tw-mask-radial-from-position), var(--tw-mask-radial-to-color) var(--tw-mask-radial-to-position)";
    for stop in ["from", "to"] {
        mask_shape_stop(r, "radial", stop, mask_properties_radial, RADIAL_STOPS, "radial-gradient");
    }

    r.functional(
        "mask-conic",
        Functional::new(|cx, v, _| {
            Some(cx.nodes([
                mask_properties_gradient(cx),
                mask_properties_conic(cx),
                cx.decl("mask-image", MASK_IMAGE),
                cx.decl("mask-composite", "intersect"),
                cx.decl("--tw-mask-conic", "conic-gradient(var(--tw-mask-conic-stops, var(--tw-mask-conic-position)))"),
                cx.decl("--tw-mask-conic-position", v),
            ]))
        })
        .negative()
        .default_value(DefaultValue::Null)
        .bare(bare_angle)
        .negative_bare(negative_bare_angle)
        .declaration(declaration_fn(|cx, v, _| {
            Some(Vec::from_iter_in(
                [
                    ("mask-image", MASK_IMAGE),
                    ("mask-composite", "intersect"),
                    ("--tw-mask-conic", "conic-gradient(var(--tw-mask-conic-stops, var(--tw-mask-conic-position)))"),
                    ("--tw-mask-conic-position", v),
                ],
                cx.alloc,
            ))
        })),
    );
    r.suggest("mask-conic", vec![Suggest::values(&ANGLES).negative(true)]);
    const CONIC_STOPS: &str = "from var(--tw-mask-conic-position), var(--tw-mask-conic-from-color) var(--tw-mask-conic-from-position), var(--tw-mask-conic-to-color) var(--tw-mask-conic-to-position)";
    for stop in ["from", "to"] {
        mask_shape_stop(r, "conic", stop, mask_properties_conic, CONIC_STOPS, "conic-gradient");
    }
}
