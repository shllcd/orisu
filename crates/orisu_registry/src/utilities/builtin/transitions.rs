use super::PLAIN;
use crate::utilities::{Cx, DefaultValue, Functional, Registrar, Suggest};
use orisu_ast::{Node, Nodes};
use orisu_candidate::{CandidateKind, UtilityValue};
use orisu_parser::infer::is_positive_integer;

const DEFAULT_TRANSITION_PROPERTY: &str = "color, background-color, border-color, outline-color, text-decoration-color, fill, stroke, --tw-gradient-from, --tw-gradient-via, --tw-gradient-to, opacity, box-shadow, transform, translate, scale, rotate, filter, -webkit-backdrop-filter, backdrop-filter, display, content-visibility, overlay, pointer-events";

const CONTAIN_VALUE: &str = "var(--tw-contain-size,) var(--tw-contain-layout,) var(--tw-contain-paint,) var(--tw-contain-style,)";

const FONT_VARIANT_NUMERIC_VALUE: &str =
    "var(--tw-ordinal,) var(--tw-slashed-zero,) var(--tw-numeric-figure,) var(--tw-numeric-spacing,) var(--tw-numeric-fraction,)";

fn ease_property<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([cx.property("--tw-ease", None, None)])
}

fn contain_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-contain-size", None, None),
        cx.property("--tw-contain-layout", None, None),
        cx.property("--tw-contain-paint", None, None),
        cx.property("--tw-contain-style", None, None),
    ])
}

fn font_variant_numeric_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([
        cx.property("--tw-ordinal", None, None),
        cx.property("--tw-slashed-zero", None, None),
        cx.property("--tw-numeric-figure", None, None),
        cx.property("--tw-numeric-spacing", None, None),
        cx.property("--tw-numeric-fraction", None, None),
    ])
}

fn leading<'a>(cx: &Cx<'a, '_>, value: &'a str) -> Option<Nodes<'a>> {
    Some(cx.nodes([cx.at_root([cx.property("--tw-leading", None, None)]), cx.decl("--tw-leading", value), cx.decl("line-height", value)]))
}

pub(super) fn register<'a>(r: &mut Registrar<'a, '_>) {
    let ease = r.theme.resolve(None, &["--default-transition-timing-function"], 0).unwrap_or("ease");
    let ease = r.cat(&["var(--tw-ease, ", ease, ")"]);
    let duration = r.theme.resolve(None, &["--default-transition-duration"], 0).unwrap_or("0s");
    let duration = r.cat(&["var(--tw-duration, ", duration, ")"]);
    let timed = |property: &'a str| -> [(&'a str, &'a str); 3] {
        [("transition-property", property), ("transition-timing-function", ease), ("transition-duration", duration)]
    };
    let statics = r.statics(&[
        ("none", &[("transition-property", "none")]),
        ("all", &timed("all")),
        (
            "colors",
            &timed(
                "color, background-color, border-color, outline-color, text-decoration-color, fill, stroke, --tw-gradient-from, --tw-gradient-via, --tw-gradient-to",
            ),
        ),
        ("opacity", &timed("opacity")),
        ("shadow", &timed("box-shadow")),
        ("transform", &timed("transform, translate, scale, rotate")),
    ]);
    r.functional(
        "transition",
        Functional::new(move |cx, v, _| {
            Some(cx.nodes([
                cx.decl("transition-property", v),
                cx.decl("transition-timing-function", ease),
                cx.decl("transition-duration", duration),
            ]))
        })
        .default_value(DefaultValue::Value(DEFAULT_TRANSITION_PROPERTY))
        .keys(&["--transition-property"])
        .statics(statics),
    );
    r.static_decls("transition-discrete", &[("transition-behavior", "allow-discrete")]);
    r.static_decls("transition-normal", &[("transition-behavior", "normal")]);
    r.functional(
        "delay",
        Functional::decl("transition-delay")
            .keys(&["--transition-delay"])
            .bare(|cx, n| is_positive_integer(n.value).then(|| cx.cat(&[n.value, "ms"]))),
    );
    r.static_fn("duration-initial", |cx, _| {
        Some(cx.nodes([cx.at_root([cx.property("--tw-duration", None, None)]), cx.decl("--tw-duration", "initial")]))
    });
    r.functional_fn("duration", |cx, c| {
        let CandidateKind::Functional { value, modifier: None, .. } = c.kind else {
            return None;
        };
        let value = match value? {
            UtilityValue::Arbitrary { value, .. } => value,
            UtilityValue::Named { value, fraction } => cx
                .resolve(Some(fraction.unwrap_or(value)), &["--transition-duration"])
                .or_else(|| is_positive_integer(value).then(|| cx.cat(&[value, "ms"])))?,
        };
        Some(cx.nodes([
            cx.at_root([cx.property("--tw-duration", None, None)]),
            cx.decl("--tw-duration", value),
            cx.decl("transition-duration", value),
        ]))
    });
    const TIMES: [&str; 8] = ["75", "100", "150", "200", "300", "500", "700", "1000"];
    r.suggest("delay", vec![Suggest::values(&TIMES).keys(&["--transition-delay"])]);
    r.suggest("duration", vec![Suggest::values(&TIMES).keys(&["--transition-duration"])]);

    r.functional(
        "ease",
        Functional::new(|cx, v, _| Some(cx.nodes([ease_property(cx), cx.decl("--tw-ease", v), cx.decl("transition-timing-function", v)])))
            .keys(&["--ease"])
            .static_nodes(&["initial", "linear"], |cx, key| {
                if key == "initial" {
                    return Some(cx.nodes([ease_property(cx), cx.decl("--tw-ease", "initial")]));
                }
                Some(cx.nodes([ease_property(cx), cx.decl("--tw-ease", "linear"), cx.decl("transition-timing-function", "linear")]))
            }),
    );

    r.static_decls("will-change-auto", &[("will-change", "auto")]);
    r.static_decls("will-change-scroll", &[("will-change", "scroll-position")]);
    r.static_decls("will-change-contents", &[("will-change", "contents")]);
    r.static_decls("will-change-transform", &[("will-change", "transform")]);
    r.functional("will-change", Functional::decl("will-change"));

    r.static_decls("content-none", &[("--tw-content", "none"), ("content", "none")]);
    r.functional(
        "content",
        Functional::new(|cx, v, _| {
            Some(cx.nodes([
                cx.at_root([cx.property("--tw-content", Some("\"\""), None)]),
                cx.decl("--tw-content", v),
                cx.decl("content", "var(--tw-content)"),
            ]))
        })
        .keys(&["--content"]),
    );

    for value in ["none", "content", "strict"] {
        r.static_decls(&format!("contain-{value}"), &[("contain", value)]);
    }
    for (name, var, value) in [
        ("size", "--tw-contain-size", "size"),
        ("inline-size", "--tw-contain-size", "inline-size"),
        ("layout", "--tw-contain-layout", "layout"),
        ("paint", "--tw-contain-paint", "paint"),
        ("style", "--tw-contain-style", "style"),
    ] {
        r.static_fn(&format!("contain-{name}"), move |cx, _| {
            Some(cx.nodes([contain_properties(cx), cx.decl(var, value), cx.decl("contain", CONTAIN_VALUE)]))
        });
    }
    r.functional("contain", Functional::decl("contain"));

    r.static_decls("forced-color-adjust-none", &[("forced-color-adjust", "none")]);
    r.static_decls("forced-color-adjust-auto", &[("forced-color-adjust", "auto")]);
    r.spacing_with("leading", &["--leading", "--spacing"], PLAIN, leading, |desc| desc.static_nodes(&["none"], |cx, _| leading(cx, "1")));
    r.functional(
        "tracking",
        Functional::new(|cx, v, _| {
            Some(cx.nodes([
                cx.at_root([cx.property("--tw-tracking", None, None)]),
                cx.decl("--tw-tracking", v),
                cx.decl("letter-spacing", v),
            ]))
        })
        .negative()
        .keys(&["--tracking"]),
    );
    r.static_decls("antialiased", &[("-webkit-font-smoothing", "antialiased"), ("-moz-osx-font-smoothing", "grayscale")]);
    r.static_decls("subpixel-antialiased", &[("-webkit-font-smoothing", "auto"), ("-moz-osx-font-smoothing", "auto")]);

    r.static_decls("normal-nums", &[("font-variant-numeric", "normal")]);
    for (name, var) in [
        ("ordinal", "--tw-ordinal"),
        ("slashed-zero", "--tw-slashed-zero"),
        ("lining-nums", "--tw-numeric-figure"),
        ("oldstyle-nums", "--tw-numeric-figure"),
        ("proportional-nums", "--tw-numeric-spacing"),
        ("tabular-nums", "--tw-numeric-spacing"),
        ("diagonal-fractions", "--tw-numeric-fraction"),
        ("stacked-fractions", "--tw-numeric-fraction"),
    ] {
        r.static_fn(name, move |cx, _| {
            Some(cx.nodes([
                font_variant_numeric_properties(cx),
                cx.decl(var, name),
                cx.decl("font-variant-numeric", FONT_VARIANT_NUMERIC_VALUE),
            ]))
        });
    }
}
