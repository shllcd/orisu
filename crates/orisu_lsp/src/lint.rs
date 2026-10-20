//! The lints behind both the editor diagnostics and `orisu check`, reported against the diagnostic catalog.

use crate::context::{Matcher, Region, Scope, Token};
use crate::document::Language;
use crate::project::Project;
use crate::settings::{Level, Settings};
use orisu_diagnostics::{Code, Edit, Report, Span};
use orisu_parser::segment::segment;
use rustc_hash::FxHashMap;

pub struct Finding {
    pub report: Report,
    pub level: Level,
}

pub fn regions<'t>(project: &Project, settings: &Settings, text: &'t str, language: Language) -> Vec<Region<'t>> {
    let matcher =
        Matcher { attributes: &settings.class_attributes, functions: &settings.class_functions, attributify: project.attributify.as_ref() };
    matcher.regions(text, language)
}

/// Lints `text`; canonical spellings only when `canonical`, since the first one takes a while.
pub fn lint(project: &Project, settings: &Settings, text: &str, language: Language, canonical: bool) -> Vec<Finding> {
    let lint = &settings.lint;
    let mut out = Vec::new();
    let mut push = |report: Report, level: Level| {
        if level != Level::Ignore {
            out.push(Finding { report, level });
        }
    };
    for region in regions(project, settings, text, language).iter().filter(|r| r.scope == Scope::Classes) {
        let tokens = region.tokens(text);
        for token in tokens.iter().filter(|t| !t.grouped) {
            let span = Span::new(token.start, token.end);
            let known = project.info(&token.class).css.is_some();
            if !known {
                if lint.unknown_classes != Level::Ignore
                    && let Some(report) = unknown(project, token)
                {
                    push(report, lint.unknown_classes);
                }
                continue;
            }
            for key in referenced_variables(&token.class).filter(|key| project.is_missing_theme_key(key)) {
                push(Report::new(Code::InvalidThemeKey, span).arg("key", key).arg("class", token.class.as_str()), lint.invalid_theme_key);
            }
            if canonical
                && lint.suggest_canonical_classes != Level::Ignore
                && let Some(canonical) = project.canonical(&token.class, settings.root_font_size)
            {
                let report = Report::new(Code::NonCanonicalClass, span)
                    .arg("class", token.class.as_str())
                    .arg("canonical", canonical.as_str())
                    .fix(vec![replace(token, canonical)]);
                push(report, lint.suggest_canonical_classes);
            }
        }
        if lint.css_conflict != Level::Ignore {
            for report in conflicts(project, &tokens) {
                push(report, lint.css_conflict);
            }
        }
    }
    out
}

fn replace(token: &Token, text: String) -> Edit {
    Edit { start: token.start as u32, end: token.end as u32, text }
}

/// `[UNKNOWN_VARIANT]` when the utility alone is known, else `[UNKNOWN_UTILITY]` with the closest spelling as a fix.
fn unknown(project: &Project, token: &Token) -> Option<Report> {
    let span = Span::new(token.start, token.end);
    let parts = segment(&token.class, b':');
    let (base, variants) = parts.split_last()?;
    if !variants.is_empty() && project.info(base).css.is_some() {
        let variant = variants.iter().find(|v| project.info(&format!("{v}:{base}")).css.is_none())?;
        return Some(Report::new(Code::UnknownVariant, span).arg("variant", *variant).arg("class", token.class.as_str()));
    }
    let report = Report::new(Code::UnknownUtility, span).arg("class", token.class.as_str());
    let utility = base.trim_start_matches('!').trim_end_matches('!');
    let Some(suggestion) = project.suggestion(utility) else {
        return Some(report);
    };
    let replacement = token.class.replacen(utility, suggestion, 1);
    Some(report.arg("suggestion", suggestion).fix(vec![replace(token, replacement)]))
}

fn conflicts(project: &Project, tokens: &[Token]) -> Vec<Report> {
    let mut groups: FxHashMap<(String, Vec<String>), Vec<&Token>> = FxHashMap::default();
    for token in tokens {
        let info = project.info(&token.class);
        if info.properties.is_empty() {
            continue;
        }
        let variants = segment(&token.class, b':');
        let context = variants[..variants.len() - 1].join(":");
        groups.entry((context, info.properties.clone())).or_default().push(token);
    }
    let mut out = Vec::new();
    for group in groups.into_values().filter(|g| g.len() > 1) {
        for token in &group {
            let others: Vec<&&Token> = group.iter().filter(|o| o.start != token.start).collect();
            let names = others.iter().map(|o| format!("`{}`", o.class)).collect::<Vec<_>>().join(", ");
            let mut report = Report::new(Code::ConflictingClasses, Span::new(token.start, token.end))
                .arg("class", token.class.as_str())
                .arg("others", names);
            for other in others {
                report = report.label(Span::new(other.start, other.end), other.class.as_str());
            }
            out.push(report);
        }
    }
    out.sort_by_key(|report| report.span.start);
    out
}

/// Custom properties `class` reads through `(--name)` or `var(--name)`.
fn referenced_variables(class: &str) -> impl Iterator<Item = &str> {
    let bytes = class.as_bytes();
    class.match_indices("--").filter_map(move |(at, _)| {
        if at == 0 || !matches!(bytes[at - 1], b'(' | b',') {
            return None;
        }
        let end = bytes[at..].iter().position(|&b| !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_')).map_or(bytes.len(), |n| at + n);
        Some(&class[at..end])
    })
}
