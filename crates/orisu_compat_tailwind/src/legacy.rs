// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use orisu_candidate::{CandidateKind, UtilityValue};
use orisu_parser::infer::{is_multiple_of_quarter, is_positive_integer};
use orisu_parser::segment::segment;
use orisu_registry::utilities::Registrar;

pub(crate) fn register<'a>(r: &mut Registrar<'a, '_>) {
    for (value, direction) in [
        ("t", "top"),
        ("tr", "top right"),
        ("r", "right"),
        ("br", "bottom right"),
        ("b", "bottom"),
        ("bl", "bottom left"),
        ("l", "left"),
        ("tl", "top left"),
    ] {
        let name = format!("bg-gradient-to-{value}");
        r.suggest(&name, vec![]);
        r.static_decls(
            &name,
            &[
                ("--tw-gradient-position", &format!("to {direction} in oklab")),
                ("background-image", "linear-gradient(var(--tw-gradient-stops))"),
            ],
        );
    }
    for (name, property, value) in [
        ("bg-left-top", "background-position", "left top"),
        ("bg-right-top", "background-position", "right top"),
        ("bg-left-bottom", "background-position", "left bottom"),
        ("bg-right-bottom", "background-position", "right bottom"),
        ("object-left-top", "object-position", "left top"),
        ("object-right-top", "object-position", "right top"),
        ("object-left-bottom", "object-position", "left bottom"),
        ("object-right-bottom", "object-position", "right bottom"),
    ] {
        r.suggest(name, vec![]);
        r.static_decls(name, &[(property, value)]);
    }

    r.suggest("max-w-screen", vec![]);
    r.functional_fn("max-w-screen", |cx, c| {
        let CandidateKind::Functional { value: Some(UtilityValue::Named { value, .. }), .. } = c.kind else {
            return None;
        };
        cx.one("max-width", cx.resolve(Some(value), &["--breakpoint"]).filter(|v| !v.is_empty())?)
    });

    r.suggest("overflow-ellipsis", vec![]);
    r.static_decls("overflow-ellipsis", &[("text-overflow", "ellipsis")]);
    for value in ["slice", "clone"] {
        let name = format!("decoration-{value}");
        r.suggest(&name, vec![]);
        r.static_decls(&name, &[("-webkit-box-decoration-break", value), ("box-decoration-break", value)]);
    }
    for (name, property) in [("flex-shrink", "flex-shrink"), ("flex-grow", "flex-grow")] {
        r.suggest(name, vec![]);
        r.functional_fn(name, move |cx, c| {
            let CandidateKind::Functional { value, modifier: None, .. } = c.kind else {
                return None;
            };
            match value {
                None => cx.one(property, "1"),
                Some(UtilityValue::Arbitrary { value, .. }) => cx.one(property, value),
                Some(UtilityValue::Named { value, .. }) => is_positive_integer(value).then(|| cx.one(property, value))?,
            }
        });
    }
    r.suggest("order-none", vec![]);
    r.static_decls("order-none", &[("order", "0")]);
    r.suggest("break-words", vec![]);
    r.static_decls("break-words", &[("overflow-wrap", "break-word")]);

    for (name, property) in [("start", "inset-inline-start"), ("end", "inset-inline-end")] {
        r.static_decls(&format!("{name}-auto"), &[(property, "auto")]);
        r.static_decls(&format!("{name}-full"), &[(property, "100%")]);
        r.static_decls(&format!("-{name}-full"), &[(property, "-100%")]);
        r.static_decls(&format!("{name}-px"), &[(property, "1px")]);
        r.static_decls(&format!("-{name}-px"), &[(property, "-1px")]);
        for negative in [true, false] {
            let root = if negative { format!("-{name}") } else { name.to_owned() };
            r.functional_fn(&root, move |cx, c| {
                let CandidateKind::Functional { value, modifier, .. } = c.kind else {
                    return None;
                };
                let (value, fraction) = match value? {
                    UtilityValue::Arbitrary { value, .. } => {
                        if modifier.is_some() {
                            return None;
                        }
                        return cx.one(property, if negative { cx.negate(value) } else { value });
                    }
                    UtilityValue::Named { value, fraction } => (value, fraction),
                };
                let mut resolved = cx.resolve(Some(fraction.unwrap_or(value)), &["--inset", "--spacing"]);
                if resolved.is_none()
                    && let Some(fraction) = fraction
                {
                    let parts = segment(fraction, b'/');
                    if !is_positive_integer(parts[0]) || !parts.get(1).is_some_and(|v| is_positive_integer(v)) {
                        return None;
                    }
                    resolved = Some(cx.cat(&["calc(", fraction, " * 100%)"]));
                }
                let multiplier = || cx.resolve(None, &["--spacing"]).filter(|v| !v.is_empty());
                if resolved.is_none()
                    && negative
                    && let Some(multiplier) = multiplier()
                    && is_multiple_of_quarter(value)
                {
                    return cx.one(property, cx.cat(&["calc(", multiplier, " * -", value, ")"]));
                }
                if resolved.is_none()
                    && let Some(multiplier) = multiplier()
                    && is_multiple_of_quarter(value)
                {
                    resolved = Some(cx.cat(&["calc(", multiplier, " * ", value, ")"]));
                }
                let value = resolved?;
                cx.one(property, if negative { cx.negate(value) } else { value })
            });
        }
    }
}
