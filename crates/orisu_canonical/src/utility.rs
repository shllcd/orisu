// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use super::convert::Convert;
use super::fold::constant_fold;
use super::{Canon, Signature};
use indexmap::IndexSet;
use orisu_allocator::AllocExt;
use orisu_candidate::print::print_modifier;
use orisu_candidate::{Candidate, CandidateKind, Modifier, UtilityValue};
use orisu_parser::infer::{format_js_number, is_multiple_of_quarter, is_positive_integer, js_number};
use orisu_parser::value::{self as vp, ValueNode};
use orisu_registry::utilities::UtilityKind;
use rustc_hash::FxHashSet;

type Pass = for<'a, 'd> fn(&mut Canon<'a, 'd>, Candidate<'a>) -> Candidate<'a>;

pub(super) const UTILITY_PASSES: &[Pass] = &[
    bg_gradient_to_linear,
    theme_to_var,
    calc_to_spacing_function,
    optimize_arbitrary_value_expressions,
    arbitrary_utilities,
    bare_value_utilities,
    deprecated_utilities,
    drop_unnecessary_data_types,
    arbitrary_value_to_bare_value,
    optimize_modifier,
];

fn with_modifier<'a>(mut candidate: Candidate<'a>, new: Option<Modifier<'a>>) -> Candidate<'a> {
    match &mut candidate.kind {
        CandidateKind::Arbitrary { modifier, .. } | CandidateKind::Functional { modifier, .. } => *modifier = new,
        CandidateKind::Static { .. } => {}
    }
    candidate
}

/// The arbitrary value of an arbitrary property or a functional utility with an arbitrary value.
fn arbitrary_value<'a>(candidate: &Candidate<'a>) -> Option<&'a str> {
    match candidate.kind {
        CandidateKind::Arbitrary { value, .. } | CandidateKind::Functional { value: Some(UtilityValue::Arbitrary { value, .. }), .. } => {
            Some(value)
        }
        _ => None,
    }
}

fn with_arbitrary_value<'a>(mut candidate: Candidate<'a>, new: &'a str) -> Candidate<'a> {
    match &mut candidate.kind {
        CandidateKind::Arbitrary { value, .. } | CandidateKind::Functional { value: Some(UtilityValue::Arbitrary { value, .. }), .. } => {
            *value = new
        }
        _ => {}
    }
    candidate
}

fn with_root<'a>(mut candidate: Candidate<'a>, new: &'a str) -> Candidate<'a> {
    if let CandidateKind::Functional { root, .. } = &mut candidate.kind {
        *root = new;
    }
    candidate
}

fn with_value<'a>(mut candidate: Candidate<'a>, new: Option<UtilityValue<'a>>) -> Candidate<'a> {
    if let CandidateKind::Functional { value, .. } = &mut candidate.kind {
        *value = new;
    }
    candidate
}

fn named<'a>(value: &'a str, fraction: Option<&'a str>) -> Option<UtilityValue<'a>> {
    Some(UtilityValue::Named { value, fraction })
}

const DIRECTIONS: [&str; 8] = ["t", "tr", "r", "br", "b", "bl", "l", "tl"];

fn bg_gradient_to_linear<'a>(cx: &mut Canon<'a, '_>, candidate: Candidate<'a>) -> Candidate<'a> {
    if let CandidateKind::Static { root } = candidate.kind
        && let Some(direction) = root.strip_prefix("bg-gradient-to-")
        && DIRECTIONS.contains(&direction)
    {
        return Candidate { kind: CandidateKind::Static { root: cx.ds.alloc.concat(&["bg-linear-to-", direction]) }, ..candidate };
    }
    candidate
}

fn theme_to_var<'a>(cx: &mut Canon<'a, '_>, candidate: Candidate<'a>) -> Candidate<'a> {
    let Some(value) = arbitrary_value(&candidate) else {
        return candidate;
    };
    let mode = if candidate.modifier().is_none() { Convert::MigrateModifier } else { Convert::All };
    let (new_value, modifier) = cx.convert(value, mode);
    if new_value == value {
        return candidate;
    }
    let candidate = with_arbitrary_value(candidate, cx.ds.alloc.str(&new_value));
    if modifier.is_some() { with_modifier(candidate, modifier) } else { candidate }
}

fn calc_to_spacing_function<'a>(cx: &mut Canon<'a, '_>, candidate: Candidate<'a>) -> Candidate<'a> {
    let Some(value) = arbitrary_value(&candidate) else {
        return candidate;
    };
    let variable = match cx.prefix() {
        Some(prefix) => format!("--{prefix}-spacing"),
        None => "--spacing".to_owned(),
    };
    if !value.contains(variable.as_str()) {
        return candidate;
    }
    let alloc = cx.ds.alloc;
    let mut ast = vp::parse(value, alloc);
    spacing_calc(&mut ast, &variable, alloc);
    with_arbitrary_value(candidate, alloc.str(&vp::to_string(&ast)))
}

fn spacing_calc<'a>(nodes: &mut orisu_allocator::Vec<'a, ValueNode<'a>>, variable: &str, alloc: &'a orisu_allocator::Allocator) {
    let mut i = 0;
    while i < nodes.len() {
        if let ValueNode::Function("calc", children) = &nodes[i]
            && children.len() == 5
            && matches!(children[2], ValueNode::Word("*"))
            && matches!(&children[0], ValueNode::Function("var", args) if matches!(args.as_slice(), [ValueNode::Word(v)] if *v == variable))
        {
            let replacement = vp::parse(&format!("--spacing({})", vp::to_string(std::slice::from_ref(&children[4]))), alloc);
            nodes.splice(i..=i, replacement);
            continue;
        }
        if let ValueNode::Function(_, children) = &mut nodes[i] {
            spacing_calc(children, variable, alloc);
        }
        i += 1;
    }
}

fn is_single_calc(ast: &[ValueNode<'_>]) -> bool {
    matches!(ast, [ValueNode::Function("calc", _)])
}

fn optimize_arbitrary_value_expressions<'a>(cx: &mut Canon<'a, '_>, mut candidate: Candidate<'a>) -> Candidate<'a> {
    let CandidateKind::Functional { root, value: Some(UtilityValue::Arbitrary { value, .. }), .. } = candidate.kind else {
        return candidate;
    };
    if !value.contains("calc(") && !root.starts_with('-') {
        return candidate;
    }
    let alloc = cx.ds.alloc;
    let mut value_ast = vp::parse(value, alloc);

    if is_single_calc(&value_ast) && constant_fold(&mut value_ast, None, false, alloc) {
        let replacement = with_arbitrary_value(candidate, alloc.str(&vp::to_string(&value_ast)));
        if cx.same_candidate_signature(&candidate, &replacement) {
            candidate = replacement;
        }
    }

    if let CandidateKind::Functional { root, value: Some(UtilityValue::Arbitrary { value, .. }), .. } = candidate.kind
        && let Some(positive) = root.strip_prefix('-')
    {
        if matches!(value_ast.as_slice(), [ValueNode::Function("var", _)]) {
            return candidate;
        }
        let mut expression = vp::parse(&format!("calc({value} * -1)"), alloc);
        if constant_fold(&mut expression, None, false, alloc) {
            let replacement = with_arbitrary_value(with_root(candidate, positive), alloc.str(&vp::to_string(&expression)));
            if cx.same_candidate_signature(&candidate, &replacement) {
                candidate = replacement;
                value_ast = expression;
            }
        }
    }
    let _ = root;

    if let [ValueNode::Function("calc", args)] = value_ast.as_slice()
        && args.len() == 5
        && matches!(args[1], ValueNode::Separator(" "))
        && matches!(args[2], ValueNode::Word("*"))
        && matches!(args[3], ValueNode::Separator(" "))
    {
        let arg = if matches!(args[4], ValueNode::Word("-1")) {
            Some(&args[0])
        } else if matches!(args[0], ValueNode::Word("-1")) {
            Some(&args[4])
        } else {
            None
        };
        if let Some(arg) = arg
            && let CandidateKind::Functional { root, .. } = candidate.kind
        {
            let replacement = with_arbitrary_value(
                with_root(candidate, alloc.concat(&["-", root])),
                alloc.str(&vp::to_string(std::slice::from_ref(arg))),
            );
            if cx.same_candidate_signature(&candidate, &replacement) {
                candidate = replacement;
            }
        }
    }
    candidate
}

fn arbitrary_utilities<'a>(cx: &mut Canon<'a, '_>, candidate: Candidate<'a>) -> Candidate<'a> {
    if arbitrary_value(&candidate).is_none() {
        return candidate;
    }
    let printed = cx.print(&candidate);
    let target = cx.utility_signature(&printed);
    if !target.is_valid() {
        return candidate;
    }
    let mut found = None;
    cx.try_arbitrary_replacements(target, candidate, &mut |cx, replacement| {
        if cx.same_candidate_signature(&candidate, &replacement) && cx.all_variables_are_used(&candidate, &replacement) {
            found = Some(replacement);
            return true;
        }
        false
    });
    found.unwrap_or(candidate)
}

fn bare_value_utilities<'a>(cx: &mut Canon<'a, '_>, candidate: Candidate<'a>) -> Candidate<'a> {
    if !matches!(candidate.kind, CandidateKind::Functional { value: Some(UtilityValue::Named { .. }), .. }) {
        return candidate;
    }
    let printed = cx.print(&candidate);
    let target = cx.utility_signature(&printed);
    if !target.is_valid() {
        return candidate;
    }
    let mut found = None;
    cx.try_precomputed_replacements(target, candidate, &mut |cx, replacement| {
        if cx.same_candidate_signature(&candidate, &replacement) {
            found = Some(replacement);
            return true;
        }
        false
    });
    found.unwrap_or(candidate)
}

type Yield<'f, 'a, 'd> = &'f mut dyn FnMut(&mut Canon<'a, 'd>, Candidate<'a>) -> bool;

impl<'a, 'd> Canon<'a, 'd> {
    fn yield_parsed(&mut self, input: &str, f: Yield<'_, 'a, 'd>) -> bool {
        self.parse(input).iter().any(|c| f(self, *c))
    }

    /// Yields the precomputed utilities with `target` as signature; `true` once `f` accepted one.
    fn try_precomputed_replacements(&mut self, target: Signature, candidate: Candidate<'a>, f: Yield<'_, 'a, 'd>) -> bool {
        self.try_replacements(target, candidate, false, f)
    }

    fn try_arbitrary_replacements(&mut self, target: Signature, candidate: Candidate<'a>, f: Yield<'_, 'a, 'd>) -> bool {
        self.try_replacements(target, candidate, true, f)
    }

    fn try_replacements(&mut self, target: Signature, candidate: Candidate<'a>, functional: bool, f: Yield<'_, 'a, 'd>) -> bool {
        let replacements = self.precomputed_utilities().get(&target).cloned().unwrap_or_default();

        if replacements.len() > 1 {
            let mut positive = replacements.iter().filter(|r| !r.starts_with('-'));
            let Some(first) = positive.next() else {
                return false;
            };
            if positive.next().is_some() {
                return false;
            }
            return self.yield_parsed(first, f);
        }

        if replacements.is_empty()
            && let Some(modifier) = candidate.modifier()
        {
            let without = with_modifier(candidate, None);
            let printed = self.print(&without);
            let target = self.utility_signature(&printed);
            if target.is_valid()
                && self
                    .try_replacements(target, without, functional, &mut |cx, replacement| f(cx, with_modifier(replacement, Some(modifier))))
            {
                return true;
            }
        }

        if let [replacement] = replacements.as_slice() {
            return self.yield_parsed(replacement, f);
        }
        if !functional || !replacements.is_empty() {
            return false;
        }

        let value = match candidate.kind {
            CandidateKind::Arbitrary { value, .. } => value,
            CandidateKind::Functional { value: Some(value), .. } => match value {
                UtilityValue::Arbitrary { value, .. } | UtilityValue::Named { value, .. } => value,
            },
            _ => return false,
        };

        if self.sig.rem.is_some()
            && let CandidateKind::Functional { value: Some(UtilityValue::Arbitrary { .. }), .. } = candidate.kind
            && let Some(bare) = self.spacing(value)
            && is_valid_spacing_number(bare)
            && self.is_reasonable_bare_value(bare)
            && f(self, with_value(candidate, named(self.ds.alloc.str(&format_js_number(bare)), None)))
        {
            return true;
        }

        let mut multiplier = self.spacing(value);
        let mut root_prefix = "";
        if let Some(m) = multiplier
            && m < 0.0
        {
            root_prefix = "-";
            multiplier = Some(m.abs());
        }
        let spacing = multiplier.filter(|m| is_valid_spacing_number(*m) && self.is_reasonable_bare_value(*m)).map(format_js_number);
        let modifier = candidate.modifier().map(print_modifier);

        let own = if let CandidateKind::Functional { root, .. } = candidate.kind { Some(root) } else { None };
        let roots = if candidate.modifier().is_some() { self.ordered_roots() } else { self.replacement_roots(target, own) };
        for root in roots {
            let root = format!("{root_prefix}{root}");
            if self.yield_parsed(&format!("{root}-{value}"), f) {
                return true;
            }
            if let Some(spacing) = &spacing {
                if self.yield_parsed(&format!("{root}-{spacing}"), f) {
                    return true;
                }
                if let Some(modifier) = &modifier
                    && self.yield_parsed(&format!("{root}-{spacing}{modifier}"), f)
                {
                    return true;
                }
            }
            if self.yield_parsed(&format!("{root}-[{value}]"), f) {
                return true;
            }
            if let Some(modifier) = &modifier
                && self.yield_parsed(&format!("{root}-[{value}]{modifier}"), f)
            {
                return true;
            }
        }
        false
    }

    /// All functional roots, negatives last, in registration order.
    fn ordered_roots(&self) -> std::vec::Vec<&'a str> {
        let mut roots: std::vec::Vec<&str> = self.ds.ds.utilities.keys(UtilityKind::Functional).collect();
        roots.sort_by_key(|root| root.starts_with('-'));
        roots
    }

    /// The functional roots that can produce every property of `target`, or all of them when that cannot be decided.
    fn replacement_roots(&mut self, target: Signature, own: Option<&'a str>) -> std::vec::Vec<&'a str> {
        let Some(allowed) = self.allowed_roots(target, own) else {
            return self.ordered_roots();
        };
        self.ordered_roots().into_iter().filter(|root| allowed.contains(root)).collect()
    }

    /// Roots whose known properties cover `target`'s; `None` when a property's producers are unknown.
    fn allowed_roots(&mut self, target: Signature, own: Option<&'a str>) -> Option<FxHashSet<&'a str>> {
        let properties = self.storage.signature_properties.get(&target).cloned()?;
        if properties.is_empty() {
            return None;
        }
        let key = self.sig_key();
        let index = self.storage.property_roots.get(&key)?;
        let seen = self.storage.property_seen.get(&key)?;
        let mut allowed: Option<FxHashSet<&'a str>> = None;
        for property in properties.iter() {
            let Some(roots) = index.get(property.as_str()) else {
                // A property no known utility produces is only ever an arbitrary property spelled out in full.
                if seen.contains(property) || property.starts_with("--") || own.is_none() {
                    continue;
                }
                return None;
            };
            let set: FxHashSet<&'a str> = roots.iter().copied().collect();
            allowed = Some(match allowed {
                None => set,
                Some(previous) => previous.into_iter().filter(|root| set.contains(root)).collect(),
            });
        }
        let mut allowed = allowed.unwrap_or_default();
        if let Some(own) = own {
            allowed.insert(own);
        }
        Some(allowed)
    }

    /// Every `var(…)` in the original value must still be read, and not redefined, by the replacement.
    fn all_variables_are_used(&mut self, candidate: &Candidate<'a>, replacement: &Candidate<'a>) -> bool {
        let Some(value) = arbitrary_value(candidate).filter(|v| v.contains("var(--")) else {
            return true;
        };
        let printed = self.print(replacement);
        let css = self
            .without_inline(|cx| cx.ds.candidates_to_css(&[&printed]))
            .into_iter()
            .map(Option::unwrap_or_default)
            .collect::<Vec<_>>()
            .join("\n");
        let mut variables = std::vec::Vec::new();
        collect_variables(&vp::parse(value, self.ds.alloc), &mut variables);
        variables.into_iter().all(|variable| {
            let used =
                css.match_indices(&format!("var({variable}")).any(|(at, m)| matches!(css.as_bytes().get(at + m.len()), Some(b',' | b')')));
            used && !css.contains(&format!("{variable}:"))
        })
    }
}

fn collect_variables<'a>(nodes: &[ValueNode<'a>], out: &mut std::vec::Vec<&'a str>) {
    for node in nodes {
        if let ValueNode::Function(name, children) = node {
            if *name == "var" {
                out.push(children.first().map_or("", |c| c.value()));
            }
            collect_variables(children, out);
        }
    }
}

pub(super) fn is_valid_spacing_number(n: f64) -> bool {
    n >= 0.0 && n % 0.25 == 0.0
}

const DEPRECATIONS: &[(&str, &str)] =
    &[("order-none", "order-0"), ("break-words", "wrap-break-word"), ("overflow-ellipsis", "text-ellipsis")];

fn deprecated_replacements(candidate: &str) -> std::vec::Vec<String> {
    let mut out = std::vec::Vec::new();
    if let Some((_, replacement)) = DEPRECATIONS.iter().find(|(from, _)| *from == candidate) {
        out.push((*replacement).to_owned());
    }
    let (negative, rest) = match candidate.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", candidate),
    };
    for (from, to) in [("start-", "inset-s-"), ("end-", "inset-e-")] {
        if let Some(value) = rest.strip_prefix(from) {
            out.push(format!("{negative}{to}{value}"));
        }
    }
    out
}

fn deprecated_utilities<'a>(cx: &mut Canon<'a, '_>, candidate: Candidate<'a>) -> Candidate<'a> {
    let printed = cx.print(&candidate);
    for replacement in deprecated_replacements(&cx.print_unprefixed(&candidate)) {
        if !cx.same_signature(&printed, &replacement) {
            continue;
        }
        return cx.parse(&replacement).first().copied().unwrap_or(candidate);
    }
    candidate
}

fn drop_unnecessary_data_types<'a>(cx: &mut Canon<'a, '_>, candidate: Candidate<'a>) -> Candidate<'a> {
    let CandidateKind::Functional { value: Some(UtilityValue::Arbitrary { data_type: Some(_), value }), .. } = candidate.kind else {
        return candidate;
    };
    let replacement = with_value(candidate, Some(UtilityValue::Arbitrary { data_type: None, value }));
    let (printed, replaced) = (cx.print(&candidate), cx.print(&replacement));
    if cx.utility_signature(&printed) == cx.utility_signature(&replaced) { replacement } else { candidate }
}

fn arbitrary_value_to_bare_value<'a>(cx: &mut Canon<'a, '_>, candidate: Candidate<'a>) -> Candidate<'a> {
    let CandidateKind::Functional { value: Some(UtilityValue::Arbitrary { value, .. }), .. } = candidate.kind else {
        return candidate;
    };
    let printed = cx.print(&candidate);
    let expected = cx.utility_signature(&printed);
    let mut seen = IndexSet::new();
    let mut found = None;
    try_value_replacements(value, &mut seen, &mut |value, fraction| {
        let alloc = cx.ds.alloc;
        let replacement = with_value(candidate, named(alloc.str(value), fraction.map(|f| alloc.str(f))));
        let printed = cx.print(&replacement);
        if cx.utility_signature(&printed) == expected {
            found = Some(replacement);
            return true;
        }
        false
    });
    found.unwrap_or(candidate)
}

fn try_value_replacements(value: &str, seen: &mut IndexSet<String>, f: &mut dyn FnMut(&str, Option<&str>) -> bool) -> bool {
    if !seen.insert(value.to_owned()) {
        return false;
    }
    if f(value, None) {
        return true;
    }
    if let Some(number) = value.strip_suffix('%')
        && is_multiple_of_quarter(number)
        && f(number, None)
    {
        return true;
    }
    if value.contains('/') {
        let mut parts = value.split('/');
        let (numerator, denominator) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        if is_positive_integer(numerator) && is_positive_integer(denominator) && f(numerator, Some(&format!("{numerator}/{denominator}"))) {
            return true;
        }
    }
    let mut options: std::vec::Vec<String> = numbers_and_fractions(value).into_iter().collect();
    options.sort_by_key(String::len);
    options.into_iter().any(|option| try_value_replacements(&option, seen, f))
}

/// The matches of `/(\d+\/\d+)|(\d+\.?\d+)/g`, deduplicated in order.
fn numbers_and_fractions(value: &str) -> IndexSet<String> {
    let b = value.as_bytes();
    let digits = |from: usize| b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut out = IndexSet::new();
    let mut i = 0;
    while i < b.len() {
        let run = digits(i);
        if run == 0 {
            i += 1;
            continue;
        }
        let after = i + run;
        let end = if matches!(b.get(after), Some(b'/' | b'.')) && digits(after + 1) > 0 {
            Some(after + 1 + digits(after + 1))
        } else if run >= 2 {
            Some(after)
        } else {
            None
        };
        match end {
            Some(end) => {
                out.insert(value[i..end].trim().to_owned());
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

fn optimize_modifier<'a>(cx: &mut Canon<'a, '_>, candidate: Candidate<'a>) -> Candidate<'a> {
    if matches!(candidate.kind, CandidateKind::Static { .. }) {
        return candidate;
    }
    let Some(modifier) = candidate.modifier() else {
        return candidate;
    };
    let printed = cx.print(&candidate);
    let target = cx.utility_signature(&printed);
    let without = with_modifier(candidate, None);
    let printed = cx.print(&without);
    if target == cx.utility_signature(&printed) {
        return without;
    }
    let value = modifier.value();
    let alloc = cx.ds.alloc;
    let stripped = match value.strip_suffix('%') {
        Some(number) if value.contains('.') => alloc.str(&format_js_number(js_number(number))),
        Some(number) => number,
        None => value,
    };
    let replacement = with_modifier(candidate, Some(Modifier::Named(stripped)));
    let printed = cx.print(&replacement);
    if target == cx.utility_signature(&printed) {
        return replacement;
    }
    let scaled = alloc.str(&format_js_number(parse_float(value) * 100.0));
    let replacement = with_modifier(candidate, Some(Modifier::Named(scaled)));
    let printed = cx.print(&replacement);
    if target == cx.utility_signature(&printed) {
        return replacement;
    }
    candidate
}

/// JavaScript `parseFloat`: the longest numeric prefix, `NaN` when there is none.
fn parse_float(value: &str) -> f64 {
    let s = value.trim_start();
    let b = s.as_bytes();
    let mut end = 0;
    let mut best = f64::NAN;
    while end < b.len() && (b[end].is_ascii_digit() || matches!(b[end], b'.' | b'-' | b'+' | b'e' | b'E')) {
        end += 1;
        if let Ok(n) = s[..end].parse::<f64>() {
            best = n;
        }
    }
    best
}
