use super::{arbitrary_type, bare_px};
use crate::utilities::{Cx, DefaultValue, Functional, Registrar, Suggest};
use orisu_allocator::Vec;
use orisu_ast::Node;
use orisu_candidate::{CandidateKind, UtilityValue};
use orisu_parser::infer::{DataType, is_positive_integer};

const NOT_LAST_CHILD: &str = ":where(& > :not(:last-child))";

fn border_properties<'a>(cx: &Cx<'a, '_>) -> Node<'a> {
    cx.at_root([cx.property("--tw-border-style", Some("solid"), None)])
}

pub(super) fn register<'a>(r: &mut Registrar<'a, '_>) {
    for (root, side) in [
        ("border", "border"),
        ("border-x", "border-inline"),
        ("border-y", "border-block"),
        ("border-s", "border-inline-start"),
        ("border-e", "border-inline-end"),
        ("border-bs", "border-block-start"),
        ("border-be", "border-block-end"),
        ("border-t", "border-top"),
        ("border-r", "border-right"),
        ("border-b", "border-bottom"),
        ("border-l", "border-left"),
    ] {
        let style = r.cat(&[side, "-style"]);
        let width = r.cat(&[side, "-width"]);
        let color = r.cat(&[side, "-color"]);
        r.functional_decl(
            root,
            move |cx, c| {
                let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                    return None;
                };
                let with_width = |v| Some(cx.nodes([border_properties(cx), cx.decl(style, "var(--tw-border-style)"), cx.decl(width, v)]));
                let Some(value) = value else {
                    if modifier.is_some() {
                        return None;
                    }
                    return with_width(cx.theme.get(&["--default-border-width"]).unwrap_or("1px"));
                };
                match value {
                    UtilityValue::Arbitrary { data_type, value } => {
                        match arbitrary_type(data_type, value, &[DataType::Color, DataType::LineWidth, DataType::Length]) {
                            Some("line-width" | "length") => {
                                if modifier.is_some() {
                                    return None;
                                }
                                with_width(value)
                            }
                            _ => cx.one(color, cx.as_color(value, modifier)?),
                        }
                    }
                    UtilityValue::Named { value, .. } => {
                        if let Some(v) = cx.resolve_theme_color(value, modifier, &["--border-color", "--color"]) {
                            return cx.one(color, v);
                        }
                        if modifier.is_some() {
                            return None;
                        }
                        if let Some(v) = cx.resolve(Some(value), &["--border-width"]).filter(|v| !v.is_empty()) {
                            return with_width(v);
                        }
                        is_positive_integer(value).then(|| with_width(cx.cat(&[value, "px"])))?
                    }
                }
            },
            move |cx, c| {
                let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                    return None;
                };
                let with_width =
                    |cx: &Cx<'a, '_>, v: &'a str| Some(Vec::from_iter_in([(style, "var(--tw-border-style)"), (width, v)], cx.alloc));
                let Some(value) = value else {
                    if modifier.is_some() {
                        return None;
                    }
                    return with_width(cx, cx.theme.get(&["--default-border-width"]).unwrap_or("1px"));
                };
                match value {
                    UtilityValue::Arbitrary { data_type, value } => {
                        match arbitrary_type(data_type, value, &[DataType::Color, DataType::LineWidth, DataType::Length]) {
                            Some("line-width" | "length") => {
                                if modifier.is_some() {
                                    return None;
                                }
                                with_width(cx, value)
                            }
                            _ => {
                                let value = cx.as_color(value, modifier)?;
                                Some(Vec::from_iter_in([(color, value)], cx.alloc))
                            }
                        }
                    }
                    UtilityValue::Named { value, .. } => {
                        if let Some(v) = cx.resolve_theme_color(value, modifier, &["--border-color", "--color"]) {
                            return Some(Vec::from_iter_in([(color, v)], cx.alloc));
                        }
                        if modifier.is_some() {
                            return None;
                        }
                        if let Some(v) = cx.resolve(Some(value), &["--border-width"]).filter(|v| !v.is_empty()) {
                            return with_width(cx, v);
                        }
                        is_positive_integer(value).then(|| with_width(cx, cx.cat(&[value, "px"])))?
                    }
                }
            },
        );
        r.suggest(
            root,
            vec![
                Suggest::color(&["--border-color", "--color"]).default_value(true),
                Suggest::values(&["0", "2", "4", "8"]).keys(&["--border-width"]),
            ],
        );
    }

    for (axis, sort, styles, start, end) in [
        ("x", "divide-x-width", &["border-inline-style"][..], "border-inline-start-width", "border-inline-end-width"),
        ("y", "divide-y-width", &["border-bottom-style", "border-top-style"], "border-top-width", "border-bottom-width"),
    ] {
        let reverse = r.cat(&["--tw-divide-", axis, "-reverse"]);
        let start_tail = r.cat(&[" * var(", reverse, "))"]);
        let end_tail = r.cat(&[" * calc(1 - var(", reverse, ")))"]);
        let default = r.theme.get(&["--default-border-width"]).unwrap_or("1px");
        let name = format!("divide-{axis}");
        r.functional(
            &name,
            Functional::new(move |cx, v, _| {
                let mut nodes = Vec::with_capacity_in(5 + styles.len(), cx.alloc);
                nodes.push(cx.decl("--tw-sort", sort));
                nodes.push(border_properties(cx));
                nodes.push(cx.decl(reverse, "0"));
                nodes.extend(styles.iter().map(|s| cx.decl(s, "var(--tw-border-style)")));
                nodes.push(cx.decl(start, cx.cat(&["calc(", v, start_tail])));
                nodes.push(cx.decl(end, cx.cat(&["calc(", v, end_tail])));
                Some(cx.nodes([cx.at_root([cx.property(reverse, Some("0"), None)]), Node::Rule { selector: NOT_LAST_CHILD, nodes }]))
            })
            .default_value(DefaultValue::Value(default))
            .keys(&["--divide-width", "--border-width"])
            .bare(bare_px),
        );
    }
    for axis in ["x", "y"] {
        r.suggest(
            &format!("divide-{axis}"),
            vec![Suggest::values(&["0", "2", "4", "8"]).keys(&["--divide-width", "--border-width"]).default_value(true)],
        );
    }
    for axis in ["x", "y"] {
        let reverse = r.cat(&["--tw-divide-", axis, "-reverse"]);
        r.static_fn(&format!("divide-{axis}-reverse"), move |cx, _| {
            Some(cx.nodes([
                cx.at_root([cx.property(reverse, Some("0"), None)]),
                Node::Rule { selector: NOT_LAST_CHILD, nodes: cx.nodes([cx.decl(reverse, "1")]) },
            ]))
        });
    }
    for style in ["solid", "dashed", "dotted", "double", "none"] {
        r.static_fn(&format!("divide-{style}"), move |cx, _| {
            Some(cx.nodes([Node::Rule {
                selector: NOT_LAST_CHILD,
                nodes: cx.nodes([
                    cx.decl("--tw-sort", "divide-style"),
                    cx.decl("--tw-border-style", style),
                    cx.decl("border-style", style),
                ]),
            }]))
        });
    }
}
