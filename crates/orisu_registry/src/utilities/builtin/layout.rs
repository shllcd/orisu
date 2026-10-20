use super::{FRAC, NEG, NEG_FRAC, PLAIN, bare_deg, bare_integer, bare_percent, bare_strict_integer_repeat, spacing_bare};
use crate::utilities::{Cx, DefaultValue, Functional, ONE_TO_12, ONE_TO_13, Registrar, Suggest};
use orisu_allocator::Vec;
use orisu_ast::Node;
use orisu_candidate::{CandidateKind, UtilityValue};
use orisu_parser::compare::{Direction, compare_breakpoints, dimension};
use orisu_parser::infer::{DataType, infer_data_type, is_length, is_multiple_of_quarter, is_positive_integer};
use orisu_parser::segment::segment;

fn translate_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-translate-x", Some("0"), None),
        cx.property("--tw-translate-y", Some("0"), None),
        cx.property("--tw-translate-z", Some("0"), None),
    ])
}

fn scale_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-scale-x", Some("1"), None),
        cx.property("--tw-scale-y", Some("1"), None),
        cx.property("--tw-scale-z", Some("1"), None),
    ])
}

fn transform_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-rotate-x", None, None),
        cx.property("--tw-rotate-y", None, None),
        cx.property("--tw-rotate-z", None, None),
        cx.property("--tw-skew-x", None, None),
        cx.property("--tw-skew-y", None, None),
    ])
}

fn touch_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([cx.property("--tw-pan-x", None, None), cx.property("--tw-pan-y", None, None), cx.property("--tw-pinch-zoom", None, None)])
}

fn snap_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([cx.property("--tw-scroll-snap-strictness", Some("proximity"), Some("*"))])
}

fn border_spacing_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-border-spacing-x", Some("0"), Some("<length>")),
        cx.property("--tw-border-spacing-y", Some("0"), Some("<length>")),
    ])
}

const TRANSFORM_VALUE: &str = "var(--tw-rotate-x,) var(--tw-rotate-y,) var(--tw-rotate-z,) var(--tw-skew-x,) var(--tw-skew-y,)";

fn is_zero_spacing(value: &str) -> bool {
    if value == "--spacing(0)" || value == "--spacing(-0)" {
        return true;
    }
    matches!(dimension(value), Some((n, unit)) if n == 0.0 && (unit.is_none() || is_length(value)))
}

pub(super) fn register<'a>(r: &mut Registrar<'a, '_>) {
    r.static_decls(
        "sr-only",
        &[
            ("position", "absolute"),
            ("width", "1px"),
            ("height", "1px"),
            ("padding", "0"),
            ("margin", "-1px"),
            ("overflow", "hidden"),
            ("clip-path", "inset(50%)"),
            ("white-space", "nowrap"),
            ("border-width", "0"),
        ],
    );
    r.static_decls(
        "not-sr-only",
        &[
            ("position", "static"),
            ("width", "auto"),
            ("height", "auto"),
            ("padding", "0"),
            ("margin", "0"),
            ("overflow", "visible"),
            ("clip-path", "none"),
            ("white-space", "normal"),
        ],
    );
    r.static_decls("pointer-events-none", &[("pointer-events", "none")]);
    r.static_decls("pointer-events-auto", &[("pointer-events", "auto")]);
    r.static_decls("visible", &[("visibility", "visible")]);
    r.static_decls("invisible", &[("visibility", "hidden")]);
    r.static_decls("collapse", &[("visibility", "collapse")]);
    r.static_decls("static", &[("position", "static")]);
    r.static_decls("fixed", &[("position", "fixed")]);
    r.static_decls("absolute", &[("position", "absolute")]);
    r.static_decls("relative", &[("position", "relative")]);
    r.static_decls("sticky", &[("position", "sticky")]);

    for (name, property) in [
        ("inset", "inset"),
        ("inset-x", "inset-inline"),
        ("inset-y", "inset-block"),
        ("inset-s", "inset-inline-start"),
        ("inset-e", "inset-inline-end"),
        ("inset-bs", "inset-block-start"),
        ("inset-be", "inset-block-end"),
        ("top", "top"),
        ("right", "right"),
        ("bottom", "bottom"),
        ("left", "left"),
    ] {
        r.static_decls(&format!("{name}-auto"), &[(property, "auto")]);
        r.static_decls(&format!("{name}-full"), &[(property, "100%")]);
        r.static_decls(&format!("-{name}-full"), &[(property, "-100%")]);
        r.spacing_decl(name, &["--inset", "--spacing"], NEG_FRAC, property);
    }

    r.static_decls("isolate", &[("isolation", "isolate")]);
    r.static_decls("isolation-auto", &[("isolation", "auto")]);

    r.functional(
        "z",
        Functional::decl("z-index").negative().bare(bare_integer).keys(&["--z-index"]).statics(&[("auto", &[("z-index", "auto")])]),
    );
    r.suggest("z", vec![Suggest::values(&["0", "10", "20", "30", "40", "50"]).negative(true).keys(&["--z-index"])]);

    r.functional(
        "order",
        Functional::decl("order")
            .negative()
            .bare(bare_integer)
            .keys(&["--order"])
            .statics(&[("first", &[("order", "-9999")]), ("last", &[("order", "9999")])]),
    );
    r.suggest("order", vec![Suggest::values(&ONE_TO_12).negative(true).keys(&["--order"])]);

    r.functional(
        "col",
        Functional::decl("grid-column")
            .negative()
            .bare(bare_integer)
            .keys(&["--grid-column"])
            .statics(&[("auto", &[("grid-column", "auto")])]),
    );
    r.functional(
        "col-span",
        Functional::new(|cx, v, _| cx.one("grid-column", cx.cat(&["span ", v, " / span ", v])))
            .bare(bare_integer)
            .statics(&[("full", &[("grid-column", "1 / -1")])]),
    );
    r.functional(
        "col-start",
        Functional::decl("grid-column-start")
            .negative()
            .bare(bare_integer)
            .keys(&["--grid-column-start"])
            .statics(&[("auto", &[("grid-column-start", "auto")])]),
    );
    r.functional(
        "col-end",
        Functional::decl("grid-column-end")
            .negative()
            .bare(bare_integer)
            .keys(&["--grid-column-end"])
            .statics(&[("auto", &[("grid-column-end", "auto")])]),
    );
    r.suggest("col-span", vec![Suggest::values(&ONE_TO_12)]);
    r.suggest("col-start", vec![Suggest::values(&ONE_TO_13).negative(true).keys(&["--grid-column-start"])]);
    r.suggest("col-end", vec![Suggest::values(&ONE_TO_13).negative(true).keys(&["--grid-column-end"])]);

    r.functional(
        "row",
        Functional::decl("grid-row").negative().bare(bare_integer).keys(&["--grid-row"]).statics(&[("auto", &[("grid-row", "auto")])]),
    );
    r.functional(
        "row-span",
        Functional::new(|cx, v, _| cx.one("grid-row", cx.cat(&["span ", v, " / span ", v])))
            .bare(bare_integer)
            .statics(&[("full", &[("grid-row", "1 / -1")])]),
    );
    r.functional(
        "row-start",
        Functional::decl("grid-row-start")
            .negative()
            .bare(bare_integer)
            .keys(&["--grid-row-start"])
            .statics(&[("auto", &[("grid-row-start", "auto")])]),
    );
    r.functional(
        "row-end",
        Functional::decl("grid-row-end")
            .negative()
            .bare(bare_integer)
            .keys(&["--grid-row-end"])
            .statics(&[("auto", &[("grid-row-end", "auto")])]),
    );
    r.suggest("row-span", vec![Suggest::values(&ONE_TO_12)]);
    r.suggest("row-start", vec![Suggest::values(&ONE_TO_13).negative(true).keys(&["--grid-row-start"])]);
    r.suggest("row-end", vec![Suggest::values(&ONE_TO_13).negative(true).keys(&["--grid-row-end"])]);

    r.static_decls("float-start", &[("float", "inline-start")]);
    r.static_decls("float-end", &[("float", "inline-end")]);
    r.static_decls("float-right", &[("float", "right")]);
    r.static_decls("float-left", &[("float", "left")]);
    r.static_decls("float-none", &[("float", "none")]);
    r.static_decls("clear-start", &[("clear", "inline-start")]);
    r.static_decls("clear-end", &[("clear", "inline-end")]);
    r.static_decls("clear-right", &[("clear", "right")]);
    r.static_decls("clear-left", &[("clear", "left")]);
    r.static_decls("clear-both", &[("clear", "both")]);
    r.static_decls("clear-none", &[("clear", "none")]);

    for (namespace, property) in [
        ("m", "margin"),
        ("mx", "margin-inline"),
        ("my", "margin-block"),
        ("ms", "margin-inline-start"),
        ("me", "margin-inline-end"),
        ("mbs", "margin-block-start"),
        ("mbe", "margin-block-end"),
        ("mt", "margin-top"),
        ("mr", "margin-right"),
        ("mb", "margin-bottom"),
        ("ml", "margin-left"),
    ] {
        r.static_decls(&format!("{namespace}-auto"), &[(property, "auto")]);
        r.spacing_decl(namespace, &["--margin", "--spacing"], NEG, property);
    }

    r.static_decls("box-border", &[("box-sizing", "border-box")]);
    r.static_decls("box-content", &[("box-sizing", "content-box")]);

    r.functional(
        "line-clamp",
        Functional::new(|cx, v, _| {
            Some(cx.nodes([
                cx.decl("overflow", "hidden"),
                cx.decl("display", "-webkit-box"),
                cx.decl("-webkit-box-orient", "vertical"),
                cx.decl("-webkit-line-clamp", v),
            ]))
        })
        .keys(&["--line-clamp"])
        .bare(bare_integer)
        .statics(&[(
            "none",
            &[("overflow", "visible"), ("display", "block"), ("-webkit-box-orient", "horizontal"), ("-webkit-line-clamp", "unset")],
        )]),
    );
    r.suggest("line-clamp", vec![Suggest::values(&["1", "2", "3", "4", "5", "6"]).keys(&["--line-clamp"])]);

    for (name, value) in [
        ("block", "block"),
        ("inline-block", "inline-block"),
        ("inline", "inline"),
        ("hidden", "none"),
        ("inline-flex", "inline-flex"),
        ("table", "table"),
        ("inline-table", "inline-table"),
        ("table-caption", "table-caption"),
        ("table-cell", "table-cell"),
        ("table-column", "table-column"),
        ("table-column-group", "table-column-group"),
        ("table-footer-group", "table-footer-group"),
        ("table-header-group", "table-header-group"),
        ("table-row-group", "table-row-group"),
        ("table-row", "table-row"),
        ("flow-root", "flow-root"),
        ("flex", "flex"),
        ("grid", "grid"),
        ("inline-grid", "inline-grid"),
        ("contents", "contents"),
        ("list-item", "list-item"),
    ] {
        r.static_decls(name, &[("display", value)]);
    }
    r.static_decls("field-sizing-content", &[("field-sizing", "content")]);
    r.static_decls("field-sizing-fixed", &[("field-sizing", "fixed")]);

    r.functional(
        "aspect",
        Functional::decl("aspect-ratio")
            .keys(&["--aspect"])
            .bare(|_, n| {
                let fraction = n.fraction?;
                let parts = segment(fraction, b'/');
                let rhs = parts.get(1).copied().unwrap_or("");
                (is_multiple_of_quarter(parts[0]) && is_multiple_of_quarter(rhs)).then_some(fraction)
            })
            .statics(&[("auto", &[("aspect-ratio", "auto")]), ("square", &[("aspect-ratio", "1 / 1")])]),
    );

    for (key, value) in [
        ("full", "100%"),
        ("svw", "100svw"),
        ("lvw", "100lvw"),
        ("dvw", "100dvw"),
        ("svh", "100svh"),
        ("lvh", "100lvh"),
        ("dvh", "100dvh"),
        ("min", "min-content"),
        ("max", "max-content"),
        ("fit", "fit-content"),
    ] {
        r.static_decls(&format!("size-{key}"), &[("--tw-sort", "size"), ("width", value), ("height", value)]);
        r.static_decls(&format!("w-{key}"), &[("width", value)]);
        r.static_decls(&format!("h-{key}"), &[("height", value)]);
        r.static_decls(&format!("min-w-{key}"), &[("min-width", value)]);
        r.static_decls(&format!("min-h-{key}"), &[("min-height", value)]);
        r.static_decls(&format!("max-w-{key}"), &[("max-width", value)]);
        r.static_decls(&format!("max-h-{key}"), &[("max-height", value)]);
    }
    r.static_decls("size-auto", &[("--tw-sort", "size"), ("width", "auto"), ("height", "auto")]);
    r.static_decls("w-auto", &[("width", "auto")]);
    r.static_decls("h-auto", &[("height", "auto")]);
    r.static_decls("min-w-auto", &[("min-width", "auto")]);
    r.static_decls("min-h-auto", &[("min-height", "auto")]);
    r.static_decls("h-lh", &[("height", "1lh")]);
    r.static_decls("min-h-lh", &[("min-height", "1lh")]);
    r.static_decls("max-h-lh", &[("max-height", "1lh")]);
    r.static_decls("w-screen", &[("width", "100vw")]);
    r.static_decls("min-w-screen", &[("min-width", "100vw")]);
    r.static_decls("max-w-screen", &[("max-width", "100vw")]);
    r.static_decls("h-screen", &[("height", "100vh")]);
    r.static_decls("min-h-screen", &[("min-height", "100vh")]);
    r.static_decls("max-h-screen", &[("max-height", "100vh")]);
    r.static_decls("max-w-none", &[("max-width", "none")]);
    r.static_decls("max-h-none", &[("max-height", "none")]);

    r.spacing_declarations(
        "size",
        &["--size", "--spacing"],
        FRAC,
        |cx, v| Some(cx.nodes([cx.decl("--tw-sort", "size"), cx.decl("width", v), cx.decl("height", v)])),
        |cx, v| Vec::from_iter_in([("--tw-sort", "size"), ("width", v), ("height", v)], cx.alloc),
    );
    for (name, namespaces, property) in [
        ("w", &["--width", "--spacing", "--container"][..], "width"),
        ("min-w", &["--min-width", "--spacing", "--container"], "min-width"),
        ("max-w", &["--max-width", "--spacing", "--container"], "max-width"),
        ("h", &["--height", "--spacing"], "height"),
        ("min-h", &["--min-height", "--height", "--spacing"], "min-height"),
        ("max-h", &["--max-height", "--height", "--spacing"], "max-height"),
    ] {
        r.spacing_decl(name, namespaces, FRAC, property);
    }

    for (key, value) in [("full", "100%"), ("min", "min-content"), ("max", "max-content"), ("fit", "fit-content")] {
        r.static_decls(&format!("inline-{key}"), &[("inline-size", value)]);
        r.static_decls(&format!("block-{key}"), &[("block-size", value)]);
        r.static_decls(&format!("min-inline-{key}"), &[("min-inline-size", value)]);
        r.static_decls(&format!("min-block-{key}"), &[("min-block-size", value)]);
        r.static_decls(&format!("max-inline-{key}"), &[("max-inline-size", value)]);
        r.static_decls(&format!("max-block-{key}"), &[("max-block-size", value)]);
    }
    for (key, value) in [("svw", "100svw"), ("lvw", "100lvw"), ("dvw", "100dvw")] {
        r.static_decls(&format!("inline-{key}"), &[("inline-size", value)]);
        r.static_decls(&format!("min-inline-{key}"), &[("min-inline-size", value)]);
        r.static_decls(&format!("max-inline-{key}"), &[("max-inline-size", value)]);
    }
    for (key, value) in [("svh", "100svh"), ("lvh", "100lvh"), ("dvh", "100dvh")] {
        r.static_decls(&format!("block-{key}"), &[("block-size", value)]);
        r.static_decls(&format!("min-block-{key}"), &[("min-block-size", value)]);
        r.static_decls(&format!("max-block-{key}"), &[("max-block-size", value)]);
    }
    r.static_decls("inline-auto", &[("inline-size", "auto")]);
    r.static_decls("block-auto", &[("block-size", "auto")]);
    r.static_decls("min-inline-auto", &[("min-inline-size", "auto")]);
    r.static_decls("min-block-auto", &[("min-block-size", "auto")]);
    r.static_decls("block-lh", &[("block-size", "1lh")]);
    r.static_decls("min-block-lh", &[("min-block-size", "1lh")]);
    r.static_decls("max-block-lh", &[("max-block-size", "1lh")]);
    r.static_decls("inline-screen", &[("inline-size", "100vw")]);
    r.static_decls("min-inline-screen", &[("min-inline-size", "100vw")]);
    r.static_decls("max-inline-screen", &[("max-inline-size", "100vw")]);
    r.static_decls("block-screen", &[("block-size", "100vh")]);
    r.static_decls("min-block-screen", &[("min-block-size", "100vh")]);
    r.static_decls("max-block-screen", &[("max-block-size", "100vh")]);
    r.static_decls("max-inline-none", &[("max-inline-size", "none")]);
    r.static_decls("max-block-none", &[("max-block-size", "none")]);
    for (name, namespaces, property) in [
        ("inline", &["--spacing", "--container"][..], "inline-size"),
        ("min-inline", &["--spacing", "--container"], "min-inline-size"),
        ("max-inline", &["--spacing", "--container"], "max-inline-size"),
        ("block", &["--spacing"], "block-size"),
        ("min-block", &["--spacing"], "min-block-size"),
        ("max-block", &["--spacing"], "max-block-size"),
    ] {
        r.spacing_decl(name, namespaces, FRAC, property);
    }

    r.static_fn("container", |cx, _| {
        let mut breakpoints: std::vec::Vec<&str> = cx.theme.namespace("--breakpoint").into_iter().map(|(_, v)| v).collect();
        breakpoints.sort_by(|a, z| compare_breakpoints(a, z, Direction::Asc));
        let mut decls = cx.nodes([cx.decl("--tw-sort", "--tw-container-component"), cx.decl("width", "100%")]);
        for breakpoint in breakpoints {
            decls.push(Node::AtRule {
                name: "@media",
                params: cx.cat(&["(width >= ", breakpoint, ")"]),
                nodes: cx.nodes([cx.decl("max-width", breakpoint)]),
            });
        }
        Some(decls)
    });

    r.static_decls("flex-auto", &[("flex", "auto")]);
    r.static_decls("flex-initial", &[("flex", "0 auto")]);
    r.static_decls("flex-none", &[("flex", "none")]);
    r.functional_fn("flex", |cx, c| {
        let CandidateKind::Functional { value, modifier, .. } = c.kind else {
            return None;
        };
        match value? {
            UtilityValue::Arbitrary { value, .. } => {
                if modifier.is_some() {
                    return None;
                }
                cx.one("flex", value)
            }
            UtilityValue::Named { fraction: Some(fraction), .. } => {
                let parts = segment(fraction, b'/');
                let rhs = parts.get(1).copied().unwrap_or("");
                if !is_positive_integer(parts[0]) || !is_positive_integer(rhs) {
                    return None;
                }
                cx.one("flex", cx.cat(&["calc(", fraction, " * 100%)"]))
            }
            UtilityValue::Named { value, .. } => {
                if !is_positive_integer(value) || modifier.is_some() {
                    return None;
                }
                cx.one("flex", value)
            }
        }
    });
    r.suggest("flex", vec![Suggest::default().fractions(true), Suggest::values(&ONE_TO_12)]);

    r.functional("shrink", Functional::decl("flex-shrink").default_value(DefaultValue::Value("1")).bare(bare_integer));
    r.functional("grow", Functional::decl("flex-grow").default_value(DefaultValue::Value("1")).bare(bare_integer));
    r.suggest("shrink", vec![Suggest::values(&["0"]).default_value(true)]);
    r.suggest("grow", vec![Suggest::values(&["0"]).default_value(true)]);

    r.static_decls("basis-auto", &[("flex-basis", "auto")]);
    r.static_decls("basis-full", &[("flex-basis", "100%")]);
    r.spacing_decl("basis", &["--flex-basis", "--spacing", "--container"], FRAC, "flex-basis");

    r.static_decls("table-auto", &[("table-layout", "auto")]);
    r.static_decls("table-fixed", &[("table-layout", "fixed")]);
    r.static_decls("caption-top", &[("caption-side", "top")]);
    r.static_decls("caption-bottom", &[("caption-side", "bottom")]);
    r.static_decls("border-collapse", &[("border-collapse", "collapse")]);
    r.static_decls("border-separate", &[("border-collapse", "separate")]);

    const BORDER_SPACING: &str = "var(--tw-border-spacing-x) var(--tw-border-spacing-y)";
    r.spacing("border-spacing", &["--border-spacing", "--spacing"], PLAIN, |cx, v| {
        Some(cx.nodes([
            border_spacing_properties(cx),
            cx.decl("--tw-border-spacing-x", v),
            cx.decl("--tw-border-spacing-y", v),
            cx.decl("border-spacing", BORDER_SPACING),
        ]))
    });
    r.spacing("border-spacing-x", &["--border-spacing", "--spacing"], PLAIN, |cx, v| {
        Some(cx.nodes([border_spacing_properties(cx), cx.decl("--tw-border-spacing-x", v), cx.decl("border-spacing", BORDER_SPACING)]))
    });
    r.spacing("border-spacing-y", &["--border-spacing", "--spacing"], PLAIN, |cx, v| {
        Some(cx.nodes([border_spacing_properties(cx), cx.decl("--tw-border-spacing-y", v), cx.decl("border-spacing", BORDER_SPACING)]))
    });

    for (name, property) in [("origin", "transform-origin"), ("perspective-origin", "perspective-origin")] {
        let keys = r.keys(&[&format!("--{property}")]);
        let statics = r.statics(&[
            ("center", &[(property, "center")]),
            ("top", &[(property, "top")]),
            ("top-right", &[(property, "100% 0")]),
            ("right", &[(property, "100%")]),
            ("bottom-right", &[(property, "100% 100%")]),
            ("bottom", &[(property, "bottom")]),
            ("bottom-left", &[(property, "0 100%")]),
            ("left", &[(property, "0")]),
            ("top-left", &[(property, "0 0")]),
        ]);
        r.functional(name, Functional::decl(property).keys(keys).statics(statics));
    }
    r.functional("perspective", Functional::decl("perspective").keys(&["--perspective"]).statics(&[("none", &[("perspective", "none")])]));

    const TRANSLATE: &str = "var(--tw-translate-x) var(--tw-translate-y)";
    const TRANSLATE_3D: &str = "var(--tw-translate-x) var(--tw-translate-y) var(--tw-translate-z)";
    r.static_decls("translate-none", &[("translate", "none")]);
    r.static_fn("-translate-full", |cx, _| {
        Some(cx.nodes([
            translate_properties(cx),
            cx.decl("--tw-translate-x", "-100%"),
            cx.decl("--tw-translate-y", "-100%"),
            cx.decl("translate", TRANSLATE),
        ]))
    });
    r.static_fn("translate-full", |cx, _| {
        Some(cx.nodes([
            translate_properties(cx),
            cx.decl("--tw-translate-x", "100%"),
            cx.decl("--tw-translate-y", "100%"),
            cx.decl("translate", TRANSLATE),
        ]))
    });
    r.spacing_declarations(
        "translate",
        &["--translate", "--spacing"],
        NEG_FRAC,
        |cx, v| {
            Some(cx.nodes([
                translate_properties(cx),
                cx.decl("--tw-translate-x", v),
                cx.decl("--tw-translate-y", v),
                cx.decl("translate", TRANSLATE),
            ]))
        },
        |cx, v| Vec::from_iter_in([("--tw-translate-x", v), ("--tw-translate-y", v), ("translate", TRANSLATE)], cx.alloc),
    );
    for axis in ["x", "y"] {
        let property = r.cat(&["--tw-translate-", axis]);
        r.static_fn(&format!("-translate-{axis}-full"), move |cx, _| {
            Some(cx.nodes([translate_properties(cx), cx.decl(property, "-100%"), cx.decl("translate", TRANSLATE)]))
        });
        r.static_fn(&format!("translate-{axis}-full"), move |cx, _| {
            Some(cx.nodes([translate_properties(cx), cx.decl(property, "100%"), cx.decl("translate", TRANSLATE)]))
        });
        r.spacing_declarations(
            &format!("translate-{axis}"),
            &["--translate", "--spacing"],
            NEG_FRAC,
            move |cx, v| Some(cx.nodes([translate_properties(cx), cx.decl(property, v), cx.decl("translate", TRANSLATE)])),
            move |cx, v| Vec::from_iter_in([(property, v), ("translate", TRANSLATE)], cx.alloc),
        );
    }
    r.spacing_declarations(
        "translate-z",
        &["--translate", "--spacing"],
        NEG,
        |cx, v| Some(cx.nodes([translate_properties(cx), cx.decl("--tw-translate-z", v), cx.decl("translate", TRANSLATE_3D)])),
        |cx, v| Vec::from_iter_in([("--tw-translate-z", v), ("translate", TRANSLATE_3D)], cx.alloc),
    );
    r.static_fn("translate-3d", |cx, _| Some(cx.nodes([translate_properties(cx), cx.decl("translate", TRANSLATE_3D)])));

    r.static_decls("scale-none", &[("scale", "none")]);
    for negative in [true, false] {
        r.functional_fn(if negative { "-scale" } else { "scale" }, move |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            if modifier.is_some() {
                return None;
            }
            let negate = |v| if negative { cx.negate(v) } else { v };
            let value = match value? {
                UtilityValue::Arbitrary { value, .. } => return Some(cx.nodes([cx.decl("scale", negate(value))])),
                UtilityValue::Named { value, .. } => {
                    let resolved = cx.resolve(Some(value), &["--scale"]).filter(|v| !v.is_empty());
                    match resolved {
                        Some(v) => negate(v),
                        None if is_positive_integer(value) => negate(cx.cat(&[value, "%"])),
                        None => return None,
                    }
                }
            };
            Some(cx.nodes([
                scale_properties(cx),
                cx.decl("--tw-scale-x", value),
                cx.decl("--tw-scale-y", value),
                cx.decl("--tw-scale-z", value),
                cx.decl("scale", "var(--tw-scale-x) var(--tw-scale-y)"),
            ]))
        });
    }
    const SCALE_VALUES: [&str; 11] = ["0", "50", "75", "90", "95", "100", "105", "110", "125", "150", "200"];
    r.suggest("scale", vec![Suggest::values(&SCALE_VALUES).negative(true).keys(&["--scale"])]);
    for axis in ["x", "y", "z"] {
        let property = r.cat(&["--tw-scale-", axis]);
        let scale =
            if axis == "z" { "var(--tw-scale-x) var(--tw-scale-y) var(--tw-scale-z)" } else { "var(--tw-scale-x) var(--tw-scale-y)" };
        let name = format!("scale-{axis}");
        r.functional(
            &name,
            Functional::new(move |cx, v, _| Some(cx.nodes([scale_properties(cx), cx.decl(property, v), cx.decl("scale", scale)])))
                .negative()
                .keys(&["--scale"])
                .bare(bare_percent),
        );
        r.suggest(&name, vec![Suggest::values(&SCALE_VALUES).negative(true).keys(&["--scale"])]);
    }
    r.static_fn("scale-3d", |cx, _| {
        Some(cx.nodes([scale_properties(cx), cx.decl("scale", "var(--tw-scale-x) var(--tw-scale-y) var(--tw-scale-z)")]))
    });

    r.static_decls("rotate-none", &[("rotate", "none")]);
    for negative in [true, false] {
        r.functional_fn(if negative { "-rotate" } else { "rotate" }, move |cx, c| {
            let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                return None;
            };
            if modifier.is_some() {
                return None;
            }
            let value = match value? {
                UtilityValue::Arbitrary { value, data_type } => {
                    let ty = match data_type {
                        Some(t) => DataType::from_name(t),
                        None => infer_data_type(value, &[DataType::Angle, DataType::Vector]),
                    };
                    if ty == Some(DataType::Vector) {
                        return cx.one("rotate", cx.cat(&[value, " var(--tw-rotate)"]));
                    }
                    if ty != Some(DataType::Angle) {
                        return cx.one("rotate", if negative { cx.negate(value) } else { value });
                    }
                    value
                }
                UtilityValue::Named { value, .. } => match cx.resolve(Some(value), &["--rotate"]).filter(|v| !v.is_empty()) {
                    Some(v) => v,
                    None if is_positive_integer(value) => cx.cat(&[value, "deg"]),
                    None => return None,
                },
            };
            cx.one("rotate", if negative { cx.negate(value) } else { value })
        });
    }
    const ROTATE_VALUES: [&str; 9] = ["0", "1", "2", "3", "6", "12", "45", "90", "180"];
    r.suggest("rotate", vec![Suggest::values(&ROTATE_VALUES).negative(true).keys(&["--rotate"])]);

    for (axis, upper) in [("x", "X"), ("y", "Y"), ("z", "Z")] {
        let property = r.cat(&["--tw-rotate-", axis]);
        let func = r.cat(&["rotate", upper, "("]);
        let name = format!("rotate-{axis}");
        r.functional(
            &name,
            Functional::new(move |cx, v, _| {
                Some(cx.nodes([
                    transform_properties(cx),
                    cx.decl(property, cx.cat(&[func, v, ")"])),
                    cx.decl("transform", TRANSFORM_VALUE),
                ]))
            })
            .negative()
            .keys(&["--rotate"])
            .bare(bare_deg),
        );
        r.suggest(&name, vec![Suggest::values(&ROTATE_VALUES).negative(true).keys(&["--rotate"])]);
    }
    r.functional(
        "skew",
        Functional::new(|cx, v, _| {
            Some(cx.nodes([
                transform_properties(cx),
                cx.decl("--tw-skew-x", cx.cat(&["skewX(", v, ")"])),
                cx.decl("--tw-skew-y", cx.cat(&["skewY(", v, ")"])),
                cx.decl("transform", TRANSFORM_VALUE),
            ]))
        })
        .negative()
        .keys(&["--skew"])
        .bare(bare_deg),
    );
    r.functional(
        "skew-x",
        Functional::new(|cx, v, _| {
            Some(cx.nodes([
                transform_properties(cx),
                cx.decl("--tw-skew-x", cx.cat(&["skewX(", v, ")"])),
                cx.decl("transform", TRANSFORM_VALUE),
            ]))
        })
        .negative()
        .keys(&["--skew"])
        .bare(bare_deg),
    );
    r.functional(
        "skew-y",
        Functional::new(|cx, v, _| {
            Some(cx.nodes([
                transform_properties(cx),
                cx.decl("--tw-skew-y", cx.cat(&["skewY(", v, ")"])),
                cx.decl("transform", TRANSFORM_VALUE),
            ]))
        })
        .negative()
        .keys(&["--skew"])
        .bare(bare_deg),
    );
    for name in ["skew", "skew-x", "skew-y"] {
        r.suggest(name, vec![Suggest::values(&["0", "1", "2", "3", "6", "12"]).negative(true).keys(&["--skew"])]);
    }
    r.functional_fn("transform", |cx, c| {
        let CandidateKind::Functional { value, modifier, .. } = c.kind else {
            return None;
        };
        if modifier.is_some() {
            return None;
        }
        let value = match value {
            None => TRANSFORM_VALUE,
            Some(UtilityValue::Arbitrary { value, .. }) => value,
            Some(UtilityValue::Named { .. }) => return None,
        };
        Some(cx.nodes([transform_properties(cx), cx.decl("transform", value)]))
    });
    r.suggest("transform", vec![Suggest::default().default_value(true)]);
    r.static_decls("transform-cpu", &[("transform", TRANSFORM_VALUE)]);
    r.static_decls("transform-gpu", &[("transform", &format!("translateZ(0) {TRANSFORM_VALUE}"))]);
    r.static_decls("transform-none", &[("transform", "none")]);

    r.functional("zoom", Functional::decl("zoom").bare(bare_percent));
    r.suggest("zoom", vec![Suggest::values(&["50", "75", "90", "95", "100", "105", "110", "125", "150", "200"])]);

    r.static_decls("transform-flat", &[("transform-style", "flat")]);
    r.static_decls("transform-3d", &[("transform-style", "preserve-3d")]);
    r.static_decls("transform-content", &[("transform-box", "content-box")]);
    r.static_decls("transform-border", &[("transform-box", "border-box")]);
    r.static_decls("transform-fill", &[("transform-box", "fill-box")]);
    r.static_decls("transform-stroke", &[("transform-box", "stroke-box")]);
    r.static_decls("transform-view", &[("transform-box", "view-box")]);
    r.static_decls("backface-visible", &[("backface-visibility", "visible")]);
    r.static_decls("backface-hidden", &[("backface-visibility", "hidden")]);

    for value in [
        "auto",
        "default",
        "pointer",
        "wait",
        "text",
        "move",
        "help",
        "not-allowed",
        "none",
        "context-menu",
        "progress",
        "cell",
        "crosshair",
        "vertical-text",
        "alias",
        "copy",
        "no-drop",
        "grab",
        "grabbing",
        "all-scroll",
        "col-resize",
        "row-resize",
        "n-resize",
        "e-resize",
        "s-resize",
        "w-resize",
        "ne-resize",
        "nw-resize",
        "se-resize",
        "sw-resize",
        "ew-resize",
        "ns-resize",
        "nesw-resize",
        "nwse-resize",
        "zoom-in",
        "zoom-out",
    ] {
        r.static_decls(&format!("cursor-{value}"), &[("cursor", value)]);
    }
    r.functional("cursor", Functional::decl("cursor").keys(&["--cursor"]));

    for value in ["auto", "none", "manipulation"] {
        r.static_decls(&format!("touch-{value}"), &[("touch-action", value)]);
    }
    const TOUCH: &str = "var(--tw-pan-x,) var(--tw-pan-y,) var(--tw-pinch-zoom,)";
    for (property, values) in [("--tw-pan-x", ["x", "left", "right"]), ("--tw-pan-y", ["y", "up", "down"])] {
        for value in values {
            let pan = r.cat(&["pan-", value]);
            r.static_fn(&format!("touch-pan-{value}"), move |cx, _| {
                Some(cx.nodes([touch_properties(cx), cx.decl(property, pan), cx.decl("touch-action", TOUCH)]))
            });
        }
    }
    r.static_fn("touch-pinch-zoom", |cx, _| {
        Some(cx.nodes([touch_properties(cx), cx.decl("--tw-pinch-zoom", "pinch-zoom"), cx.decl("touch-action", TOUCH)]))
    });

    for value in ["none", "text", "all", "auto"] {
        r.static_decls(&format!("select-{value}"), &[("-webkit-user-select", value), ("user-select", value)]);
    }
    r.static_decls("resize-none", &[("resize", "none")]);
    r.static_decls("resize-x", &[("resize", "horizontal")]);
    r.static_decls("resize-y", &[("resize", "vertical")]);
    r.static_decls("resize", &[("resize", "both")]);

    r.static_decls("snap-none", &[("scroll-snap-type", "none")]);
    for value in ["x", "y", "both"] {
        let snap = r.cat(&[value, " var(--tw-scroll-snap-strictness)"]);
        r.static_fn(&format!("snap-{value}"), move |cx, _| Some(cx.nodes([snap_properties(cx), cx.decl("scroll-snap-type", snap)])));
    }
    r.static_fn("snap-mandatory", |cx, _| Some(cx.nodes([snap_properties(cx), cx.decl("--tw-scroll-snap-strictness", "mandatory")])));
    r.static_fn("snap-proximity", |cx, _| Some(cx.nodes([snap_properties(cx), cx.decl("--tw-scroll-snap-strictness", "proximity")])));
    r.static_decls("snap-align-none", &[("scroll-snap-align", "none")]);
    r.static_decls("snap-start", &[("scroll-snap-align", "start")]);
    r.static_decls("snap-end", &[("scroll-snap-align", "end")]);
    r.static_decls("snap-center", &[("scroll-snap-align", "center")]);
    r.static_decls("snap-normal", &[("scroll-snap-stop", "normal")]);
    r.static_decls("snap-always", &[("scroll-snap-stop", "always")]);

    for (namespace, property) in [
        ("scroll-m", "scroll-margin"),
        ("scroll-mx", "scroll-margin-inline"),
        ("scroll-my", "scroll-margin-block"),
        ("scroll-ms", "scroll-margin-inline-start"),
        ("scroll-me", "scroll-margin-inline-end"),
        ("scroll-mbs", "scroll-margin-block-start"),
        ("scroll-mbe", "scroll-margin-block-end"),
        ("scroll-mt", "scroll-margin-top"),
        ("scroll-mr", "scroll-margin-right"),
        ("scroll-mb", "scroll-margin-bottom"),
        ("scroll-ml", "scroll-margin-left"),
    ] {
        r.spacing_decl(namespace, &["--scroll-margin", "--spacing"], NEG, property);
    }
    for (namespace, property) in [
        ("scroll-p", "scroll-padding"),
        ("scroll-px", "scroll-padding-inline"),
        ("scroll-py", "scroll-padding-block"),
        ("scroll-ps", "scroll-padding-inline-start"),
        ("scroll-pe", "scroll-padding-inline-end"),
        ("scroll-pbs", "scroll-padding-block-start"),
        ("scroll-pbe", "scroll-padding-block-end"),
        ("scroll-pt", "scroll-padding-top"),
        ("scroll-pr", "scroll-padding-right"),
        ("scroll-pb", "scroll-padding-bottom"),
        ("scroll-pl", "scroll-padding-left"),
    ] {
        r.spacing_decl(namespace, &["--scroll-padding", "--spacing"], PLAIN, property);
    }

    r.static_decls("list-inside", &[("list-style-position", "inside")]);
    r.static_decls("list-outside", &[("list-style-position", "outside")]);
    r.functional(
        "list",
        Functional::decl("list-style-type").keys(&["--list-style-type"]).statics(&[
            ("none", &[("list-style-type", "none")]),
            ("disc", &[("list-style-type", "disc")]),
            ("decimal", &[("list-style-type", "decimal")]),
        ]),
    );
    r.functional(
        "list-image",
        Functional::decl("list-style-image").keys(&["--list-style-image"]).statics(&[("none", &[("list-style-image", "none")])]),
    );

    r.static_decls("appearance-none", &[("appearance", "none")]);
    r.static_decls("appearance-auto", &[("appearance", "auto")]);
    r.static_decls("scheme-normal", &[("color-scheme", "normal")]);
    r.static_decls("scheme-dark", &[("color-scheme", "dark")]);
    r.static_decls("scheme-light", &[("color-scheme", "light")]);
    r.static_decls("scheme-light-dark", &[("color-scheme", "light dark")]);
    r.static_decls("scheme-only-dark", &[("color-scheme", "only dark")]);
    r.static_decls("scheme-only-light", &[("color-scheme", "only light")]);

    r.functional(
        "columns",
        Functional::decl("columns").keys(&["--columns", "--container"]).bare(bare_integer).statics(&[("auto", &[("columns", "auto")])]),
    );
    r.suggest("columns", vec![Suggest::values(&ONE_TO_12).keys(&["--columns", "--container"])]);

    for value in ["auto", "avoid", "all", "avoid-page", "page", "left", "right", "column"] {
        r.static_decls(&format!("break-before-{value}"), &[("break-before", value)]);
    }
    for value in ["auto", "avoid", "avoid-page", "avoid-column"] {
        r.static_decls(&format!("break-inside-{value}"), &[("break-inside", value)]);
    }
    for value in ["auto", "avoid", "all", "avoid-page", "page", "left", "right", "column"] {
        r.static_decls(&format!("break-after-{value}"), &[("break-after", value)]);
    }

    r.static_decls("grid-flow-row", &[("grid-auto-flow", "row")]);
    r.static_decls("grid-flow-col", &[("grid-auto-flow", "column")]);
    r.static_decls("grid-flow-dense", &[("grid-auto-flow", "dense")]);
    r.static_decls("grid-flow-row-dense", &[("grid-auto-flow", "row dense")]);
    r.static_decls("grid-flow-col-dense", &[("grid-auto-flow", "column dense")]);

    for (name, property) in [("auto-cols", "grid-auto-columns"), ("auto-rows", "grid-auto-rows")] {
        let keys = r.keys(&[&format!("--{property}")]);
        let statics = r.statics(&[
            ("auto", &[(property, "auto")]),
            ("min", &[(property, "min-content")]),
            ("max", &[(property, "max-content")]),
            ("fr", &[(property, "minmax(0, 1fr)")]),
        ]);
        r.functional(name, Functional::decl(property).keys(keys).bare(spacing_bare).statics(statics));
    }
    for (name, property) in [("grid-cols", "grid-template-columns"), ("grid-rows", "grid-template-rows")] {
        let keys = r.keys(&[&format!("--{property}")]);
        let statics = r.statics(&[("none", &[(property, "none")]), ("subgrid", &[(property, "subgrid")])]);
        r.functional(name, Functional::decl(property).keys(keys).bare(bare_strict_integer_repeat).statics(statics));
        r.suggest(name, vec![Suggest::values(&ONE_TO_12).keys(keys)]);
    }

    r.static_decls("flex-row", &[("flex-direction", "row")]);
    r.static_decls("flex-row-reverse", &[("flex-direction", "row-reverse")]);
    r.static_decls("flex-col", &[("flex-direction", "column")]);
    r.static_decls("flex-col-reverse", &[("flex-direction", "column-reverse")]);
    r.static_decls("flex-wrap", &[("flex-wrap", "wrap")]);
    r.static_decls("flex-nowrap", &[("flex-wrap", "nowrap")]);
    r.static_decls("flex-wrap-reverse", &[("flex-wrap", "wrap-reverse")]);

    for (name, value) in [
        ("center", "center"),
        ("start", "start"),
        ("end", "end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe end"),
        ("between", "space-between"),
        ("around", "space-around"),
        ("evenly", "space-evenly"),
        ("baseline", "baseline"),
        ("stretch", "stretch"),
    ] {
        r.static_decls(&format!("place-content-{name}"), &[("place-content", value)]);
    }
    for (name, value) in [
        ("center", "center"),
        ("start", "start"),
        ("end", "end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe end"),
        ("baseline", "baseline"),
        ("stretch", "stretch"),
    ] {
        r.static_decls(&format!("place-items-{name}"), &[("place-items", value)]);
    }
    for (name, value) in [
        ("normal", "normal"),
        ("center", "center"),
        ("start", "flex-start"),
        ("end", "flex-end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe flex-end"),
        ("between", "space-between"),
        ("around", "space-around"),
        ("evenly", "space-evenly"),
        ("baseline", "baseline"),
        ("stretch", "stretch"),
    ] {
        r.static_decls(&format!("content-{name}"), &[("align-content", value)]);
    }
    for (name, value) in [
        ("center", "center"),
        ("start", "flex-start"),
        ("end", "flex-end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe flex-end"),
        ("baseline", "baseline"),
        ("baseline-last", "last baseline"),
        ("stretch", "stretch"),
    ] {
        r.static_decls(&format!("items-{name}"), &[("align-items", value)]);
    }
    for (name, value) in [
        ("normal", "normal"),
        ("center", "center"),
        ("start", "flex-start"),
        ("end", "flex-end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe flex-end"),
        ("between", "space-between"),
        ("around", "space-around"),
        ("evenly", "space-evenly"),
        ("baseline", "baseline"),
        ("stretch", "stretch"),
    ] {
        r.static_decls(&format!("justify-{name}"), &[("justify-content", value)]);
    }
    for (name, value) in [
        ("normal", "normal"),
        ("center", "center"),
        ("start", "start"),
        ("end", "end"),
        ("center-safe", "safe center"),
        ("end-safe", "safe end"),
        ("stretch", "stretch"),
    ] {
        r.static_decls(&format!("justify-items-{name}"), &[("justify-items", value)]);
    }

    r.spacing_decl("gap", &["--gap", "--spacing"], PLAIN, "gap");
    r.spacing_decl("gap-x", &["--gap", "--spacing"], PLAIN, "column-gap");
    r.spacing_decl("gap-y", &["--gap", "--spacing"], PLAIN, "row-gap");

    for (axis, sort, start, end) in
        [("x", "row-gap", "margin-inline-start", "margin-inline-end"), ("y", "column-gap", "margin-block-start", "margin-block-end")]
    {
        let reverse = r.cat(&["--tw-space-", axis, "-reverse"]);
        let start_tail = r.cat(&[" * var(", reverse, "))"]);
        let end_tail = r.cat(&[" * calc(1 - var(", reverse, ")))"]);
        r.spacing(&format!("space-{axis}"), &["--space", "--spacing"], NEG, move |cx, v| {
            let zero = is_zero_spacing(v);
            Some(cx.nodes([
                cx.at_root([cx.property(reverse, Some("0"), None)]),
                Node::Rule {
                    selector: ":where(& > :not(:last-child))",
                    nodes: cx.nodes([
                        cx.decl("--tw-sort", sort),
                        cx.decl(reverse, "0"),
                        cx.decl(start, if zero { "0" } else { cx.cat(&["calc(", v, start_tail]) }),
                        cx.decl(end, if zero { "0" } else { cx.cat(&["calc(", v, end_tail]) }),
                    ]),
                },
            ]))
        });
    }
    for (axis, sort) in [("x", "row-gap"), ("y", "column-gap")] {
        let reverse = r.cat(&["--tw-space-", axis, "-reverse"]);
        r.static_fn(&format!("space-{axis}-reverse"), move |cx, _| {
            Some(cx.nodes([
                cx.at_root([cx.property(reverse, Some("0"), None)]),
                Node::Rule {
                    selector: ":where(& > :not(:last-child))",
                    nodes: cx.nodes([cx.decl("--tw-sort", sort), cx.decl(reverse, "1")]),
                },
            ]))
        });
    }

    r.static_decls("accent-auto", &[("accent-color", "auto")]);
    r.color_decl(
        "accent",
        &["--accent-color", "--color"],
        |cx, v| cx.one("accent-color", v),
        |cx, v| Vec::from_iter_in([("accent-color", v)], cx.alloc),
    );
    r.color_decl(
        "caret",
        &["--caret-color", "--color"],
        |cx, v| cx.one("caret-color", v),
        |cx, v| Vec::from_iter_in([("caret-color", v)], cx.alloc),
    );
    r.color("divide", &["--divide-color", "--border-color", "--color"], |cx, v| {
        Some(cx.nodes([Node::Rule {
            selector: ":where(& > :not(:last-child))",
            nodes: cx.nodes([cx.decl("--tw-sort", "divide-color"), cx.decl("border-color", v)]),
        }]))
    });

    for (name, value) in [
        ("auto", "auto"),
        ("start", "start"),
        ("end", "end"),
        ("center", "center"),
        ("end-safe", "safe end"),
        ("center-safe", "safe center"),
        ("stretch", "stretch"),
    ] {
        r.static_decls(&format!("place-self-{name}"), &[("place-self", value)]);
    }
    for (name, value) in [
        ("auto", "auto"),
        ("start", "flex-start"),
        ("end", "flex-end"),
        ("center", "center"),
        ("end-safe", "safe flex-end"),
        ("center-safe", "safe center"),
        ("stretch", "stretch"),
        ("baseline", "baseline"),
        ("baseline-last", "last baseline"),
    ] {
        r.static_decls(&format!("self-{name}"), &[("align-self", value)]);
    }
    for (name, value) in [
        ("auto", "auto"),
        ("start", "flex-start"),
        ("end", "flex-end"),
        ("center", "center"),
        ("end-safe", "safe flex-end"),
        ("center-safe", "safe center"),
        ("stretch", "stretch"),
    ] {
        r.static_decls(&format!("justify-self-{name}"), &[("justify-self", value)]);
    }

    for value in ["auto", "hidden", "clip", "visible", "scroll"] {
        r.static_decls(&format!("overflow-{value}"), &[("overflow", value)]);
        r.static_decls(&format!("overflow-x-{value}"), &[("overflow-x", value)]);
        r.static_decls(&format!("overflow-y-{value}"), &[("overflow-y", value)]);
    }
    for value in ["auto", "contain", "none"] {
        r.static_decls(&format!("overscroll-{value}"), &[("overscroll-behavior", value)]);
        r.static_decls(&format!("overscroll-x-{value}"), &[("overscroll-behavior-x", value)]);
        r.static_decls(&format!("overscroll-y-{value}"), &[("overscroll-behavior-y", value)]);
    }
    r.static_decls("scroll-auto", &[("scroll-behavior", "auto")]);
    r.static_decls("scroll-smooth", &[("scroll-behavior", "smooth")]);
    r.static_decls("scrollbar-auto", &[("scrollbar-width", "auto")]);
    r.static_decls("scrollbar-thin", &[("scrollbar-width", "thin")]);
    r.static_decls("scrollbar-none", &[("scrollbar-width", "none")]);
    for (name, property) in [("scrollbar-thumb", "--tw-scrollbar-thumb"), ("scrollbar-track", "--tw-scrollbar-track")] {
        r.color_decl(
            name,
            &["--color"],
            move |cx, v| {
                Some(cx.nodes([
                    cx.at_root([
                        cx.property("--tw-scrollbar-thumb", Some("#0000"), Some("<color>")),
                        cx.property("--tw-scrollbar-track", Some("#0000"), Some("<color>")),
                    ]),
                    cx.decl(property, v),
                    cx.decl("scrollbar-color", "var(--tw-scrollbar-thumb) var(--tw-scrollbar-track)"),
                ]))
            },
            move |cx, v| {
                Vec::from_iter_in([(property, v), ("scrollbar-color", "var(--tw-scrollbar-thumb) var(--tw-scrollbar-track)")], cx.alloc)
            },
        );
    }
    r.static_decls("scrollbar-gutter-auto", &[("scrollbar-gutter", "auto")]);
    r.static_decls("scrollbar-gutter-stable", &[("scrollbar-gutter", "stable")]);
    r.static_decls("scrollbar-gutter-both", &[("scrollbar-gutter", "stable both-edges")]);

    r.static_decls("truncate", &[("overflow", "hidden"), ("text-overflow", "ellipsis"), ("white-space", "nowrap")]);
    r.static_decls("text-ellipsis", &[("text-overflow", "ellipsis")]);
    r.static_decls("text-clip", &[("text-overflow", "clip")]);
    for value in ["none", "manual", "auto"] {
        r.static_decls(&format!("hyphens-{value}"), &[("-webkit-hyphens", value), ("hyphens", value)]);
    }
    for value in ["normal", "nowrap", "pre", "pre-line", "pre-wrap", "break-spaces"] {
        r.static_decls(&format!("whitespace-{value}"), &[("white-space", value)]);
    }
    r.functional("tab", Functional::decl("tab-size").bare(bare_integer));
    r.suggest("tab", vec![Suggest::values(&["2", "4", "8"])]);
    for value in ["wrap", "nowrap", "balance", "pretty"] {
        r.static_decls(&format!("text-{value}"), &[("text-wrap", value)]);
    }
    r.static_decls("break-normal", &[("overflow-wrap", "normal"), ("word-break", "normal")]);
    r.static_decls("break-all", &[("word-break", "break-all")]);
    r.static_decls("break-keep", &[("word-break", "keep-all")]);
    r.static_decls("wrap-anywhere", &[("overflow-wrap", "anywhere")]);
    r.static_decls("wrap-break-word", &[("overflow-wrap", "break-word")]);
    r.static_decls("wrap-normal", &[("overflow-wrap", "normal")]);

    for (root, properties) in [
        ("rounded", &["border-radius"][..]),
        ("rounded-s", &["border-start-start-radius", "border-end-start-radius"]),
        ("rounded-e", &["border-start-end-radius", "border-end-end-radius"]),
        ("rounded-t", &["border-top-left-radius", "border-top-right-radius"]),
        ("rounded-r", &["border-top-right-radius", "border-bottom-right-radius"]),
        ("rounded-b", &["border-bottom-right-radius", "border-bottom-left-radius"]),
        ("rounded-l", &["border-top-left-radius", "border-bottom-left-radius"]),
        ("rounded-ss", &["border-start-start-radius"]),
        ("rounded-se", &["border-start-end-radius"]),
        ("rounded-ee", &["border-end-end-radius"]),
        ("rounded-es", &["border-end-start-radius"]),
        ("rounded-tl", &["border-top-left-radius"]),
        ("rounded-tr", &["border-top-right-radius"]),
        ("rounded-br", &["border-bottom-right-radius"]),
        ("rounded-bl", &["border-bottom-left-radius"]),
    ] {
        let none: std::vec::Vec<(&str, &str)> = properties.iter().map(|p| (*p, "0")).collect();
        let full: std::vec::Vec<(&str, &str)> = properties.iter().map(|p| (*p, "calc(infinity * 1px)")).collect();
        let statics = r.statics(&[("none", &none), ("full", &full)]);
        r.functional(
            root,
            Functional::new(move |cx, v, _| Some(orisu_allocator::Vec::from_iter_in(properties.iter().map(|p| cx.decl(p, v)), cx.alloc)))
                .keys(&["--radius"])
                .statics(statics),
        );
    }

    for style in ["solid", "dashed", "dotted", "double", "hidden", "none"] {
        r.static_decls(&format!("border-{style}"), &[("--tw-border-style", style), ("border-style", style)]);
    }
}
