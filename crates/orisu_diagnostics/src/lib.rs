mod catalog;
mod render;
mod skill;

use std::collections::BTreeMap;

use serde::Serialize;

pub use catalog::{CATALOG, Code, Entry, Example};
pub use render::{LineIndex, fill, placeholders};
pub use skill::render_skill;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warn,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warn => "warn",
        }
    }
}

/// Source bytes `start..end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start: start as u32, end: end as u32 }
    }
}

/// `offset` is a byte offset, `line` 1-based, `column` 0-based in UTF-16 code units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Position {
    pub offset: u32,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Label {
    pub start: u32,
    pub end: u32,
    pub message: String,
}

/// Replaces source bytes `start..end` with `text`.
#[derive(Clone, Debug, Serialize)]
pub struct Edit {
    pub start: u32,
    pub end: u32,
    pub text: String,
}

/// Edits that, applied together, remove the diagnostic.
#[derive(Clone, Debug, Serialize)]
pub struct Fix {
    pub title: String,
    pub edits: Vec<Edit>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub code: Code,
    pub severity: Severity,
    pub message: String,
    pub file: String,
    pub start: Position,
    pub end: Position,
    pub labels: Vec<Label>,
    pub fixes: Vec<Fix>,
    pub data: BTreeMap<String, String>,
    /// Message, location, code frame and fixes.
    pub rendered: String,
}

/// A diagnostic before its position is resolved and its text rendered from the catalog.
pub struct Report {
    pub code: Code,
    pub span: Span,
    args: Vec<(&'static str, String)>,
    labels: Vec<Label>,
    fixes: Vec<Vec<Edit>>,
}

impl Report {
    pub fn new(code: Code, span: Span) -> Self {
        Self { code, span, args: Vec::new(), labels: Vec::new(), fixes: Vec::new() }
    }

    /// Fills `{key}` in the catalog message and fix title, and becomes `data.key`.
    pub fn arg(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.args.push((key, value.into()));
        self
    }

    pub fn label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label { start: span.start, end: span.end, message: message.into() });
        self
    }

    /// Edits of the fix whose title is the catalog's `fix` template.
    pub fn fix(mut self, edits: Vec<Edit>) -> Self {
        self.fixes.push(edits);
        self
    }

    /// The catalog message with its `[CODE]` prefix.
    pub fn message(&self) -> String {
        let entry = self.code.entry();
        format!("[{}] {}", entry.name, fill(entry.message, &self.args))
    }

    pub fn fix_title(&self) -> Option<String> {
        self.code.entry().fix.map(|template| fill(template, &self.args))
    }

    pub fn code(&self) -> Code {
        self.code
    }

    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    pub fn fixes(&self) -> &[Vec<Edit>] {
        &self.fixes
    }
}

/// Sorts `reports` by start and resolves each against `source`.
pub fn resolve(mut reports: Vec<Report>, source: &str, file: &str) -> Vec<Diagnostic> {
    reports.sort_by_key(|report| report.span.start);
    let lines = LineIndex::new(source);
    reports.into_iter().map(|report| resolve_one(report, source, file, &lines)).collect()
}

fn resolve_one(report: Report, source: &str, file: &str, lines: &LineIndex) -> Diagnostic {
    #[cfg(debug_assertions)]
    render::assert_args(report.code.entry(), &report.args, !report.fixes.is_empty());
    let message = report.message();
    let title = report.fix_title().unwrap_or_default();
    let fixes: Vec<Fix> = report.fixes.into_iter().map(|edits| Fix { title: title.clone(), edits }).collect();
    let start = lines.position(source, report.span.start);
    let end = lines.position(source, report.span.end.max(report.span.start));
    let rendered = render::text(&message, &report.labels, &fixes, file, source, lines, report.span);
    Diagnostic {
        code: report.code,
        severity: report.code.severity(),
        message,
        file: file.to_owned(),
        start,
        end,
        labels: report.labels,
        fixes,
        data: report.args.into_iter().map(|(key, value)| (key.to_owned(), value)).collect(),
        rendered,
    }
}

/// Applies the first fix of every diagnostic whose edits do not overlap an earlier one.
pub fn apply_fixes(source: &str, diagnostics: &[Diagnostic]) -> String {
    let mut edits: Vec<&Edit> = Vec::new();
    for fix in diagnostics.iter().filter_map(|d| d.fixes.first()) {
        let free = fix.edits.iter().all(|e| edits.iter().all(|taken| e.end <= taken.start || taken.end <= e.start));
        if free {
            edits.extend(&fix.edits);
        }
    }
    edits.sort_by_key(|e| e.start);
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    for edit in edits {
        out.push_str(&source[at..edit.start as usize]);
        out.push_str(&edit.text);
        at = edit.end as usize;
    }
    out.push_str(&source[at..]);
    out
}

pub fn utf16_len(s: &str) -> u32 {
    if s.is_ascii() { s.len() as u32 } else { s.encode_utf16().count() as u32 }
}
