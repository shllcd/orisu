use crate::color::Rgba;
use crate::context::{Scan, Scope, Token};
use crate::document::{Document, Encoding};
use crate::lint;
use crate::project::Project;
use crate::settings::{Level, Settings};
use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Color, ColorInformation, CompletionItem, CompletionItemKind, Diagnostic,
    DiagnosticRelatedInformation, DiagnosticSeverity, Documentation, Hover, HoverContents, Location, MarkupContent, MarkupKind,
    NumberOrString, Range, TextEdit, Uri, WorkspaceEdit,
};
use orisu_diagnostics::Report;
use orisu_parser::segment::segment;
use serde::Serialize;
use serde_json::json;
use std::borrow::Cow;

pub const SOURCE: &str = "orisu";

/// Completion lists are cut here; `isIncomplete` asks the client to re-query on the next keystroke.
const MAX_ITEMS: usize = 1000;

pub struct Ctx<'a> {
    pub project: &'a Project,
    pub doc: &'a Document,
    pub settings: &'a Settings,
    pub encoding: Encoding,
    /// Whether the client fills in `editRange` and `data` from a completion list's `itemDefaults`.
    pub item_defaults: bool,
}

/// A completion list with `itemDefaults`, which `lsp-types` lacks; it keeps a list of every class small.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Completions<'a> {
    is_incomplete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_defaults: Option<Defaults>,
    items: Vec<Item<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Defaults {
    edit_range: Range,
    data: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Item<'a> {
    label: Cow<'a, str>,
    kind: CompletionItemKind,
    sort_text: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_edit: Option<TextEdit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

impl<'a> Ctx<'a> {
    fn scan(&self) -> &Scan {
        self.doc.scan(|| lint::scan(self.project, self.settings, &self.doc.text, self.doc.language))
    }

    fn range(&self, start: usize, end: usize) -> Range {
        self.doc.range(start, end, self.encoding)
    }

    fn token_at(&self, offset: usize) -> Option<&Token> {
        self.scan().around(offset).find_map(|r| r.tokens.iter().find(|t| t.start <= offset && offset <= t.end))
    }

    pub fn hover(&self, offset: usize) -> Option<Hover> {
        let token = self.token_at(offset)?;
        let css = self.project.info(&token.class).css.clone()?;
        Some(Hover { contents: HoverContents::Markup(css_markup(&css)), range: Some(self.range(token.start, token.end)) })
    }

    pub fn completion(&self, offset: usize) -> Option<Completions<'a>> {
        let region = self.scan().around(offset).next_back()?;
        let text = &self.doc.text;
        let bytes = text.as_bytes();
        let mut start = offset;
        while start > region.start
            && !bytes[start - 1].is_ascii_whitespace()
            && !(bytes[start - 1] == b'(' && start >= 2 && bytes[start - 2] == b':')
        {
            start -= 1;
        }
        let typed = &text[start..offset];
        let variants = segment(typed, b':');
        let mut word = variants.last().copied().unwrap_or_default();
        let mut word_start = offset - word.len();
        if let Some(rest) = word.strip_prefix('!') {
            word = rest;
            word_start += 1;
        }
        let range = self.range(word_start, offset);
        let first = word.bytes().next();
        let matches = |label: &str| first.is_none_or(|f| label.as_bytes().first() == Some(&f));
        let prefix = match &region.scope {
            Scope::Attribute(name) => format!("{name}-"),
            _ => String::new(),
        };
        let data = json!({ "entry": self.project.entry, "prefix": prefix });
        let item = |label: Cow<'a, str>, kind, sort_text, documentation, data: Option<&serde_json::Value>| Item {
            text_edit: (!self.item_defaults).then(|| TextEdit { range, new_text: label.to_string() }),
            label,
            kind,
            sort_text,
            documentation,
            data: data.filter(|_| !self.item_defaults).cloned(),
        };
        let classes = self.project.classes();
        let completions = self.project.completions();
        let indices: Box<dyn Iterator<Item = usize>> = match first.filter(|_| prefix.is_empty()) {
            Some(first) => Box::new(completions.by_first_byte[first as usize].iter().map(|&i| i as usize)),
            None => Box::new(0..classes.len()),
        };
        let mut items = Vec::new();
        let mut last = 0;
        for i in indices {
            if items.len() >= MAX_ITEMS {
                break;
            }
            let Some(label) = classes[i].name.strip_prefix(&prefix).filter(|label| matches(label)) else {
                continue;
            };
            let class = &completions.items[i];
            let kind = if class.color.is_some() { CompletionItemKind::COLOR } else { CompletionItemKind::CONSTANT };
            items.push(item(Cow::Borrowed(label), kind, Cow::Borrowed(&class.sort_text), class.color.as_deref(), Some(&data)));
            last = i;
        }
        let mut truncated = items.len() >= MAX_ITEMS && last + 1 < classes.len();
        let mut n = 0;
        for variant in self.project.variants() {
            if items.len() >= MAX_ITEMS {
                truncated = true;
                break;
            }
            let separator = if variant.has_dash { "-" } else { "" };
            let bare = (!variant.is_arbitrary).then(|| format!("{}:", variant.name));
            for label in bare.into_iter().chain(variant.values.iter().map(|value| format!("{}{separator}{value}:", variant.name))) {
                if items.len() >= MAX_ITEMS {
                    truncated = true;
                    break;
                }
                if matches(&label) {
                    n += 1;
                    items.push(item(Cow::Owned(label), CompletionItemKind::MODULE, Cow::Owned(format!("b{n:05}")), None, None));
                }
            }
        }
        let item_defaults = self.item_defaults.then_some(Defaults { edit_range: range, data });
        Some(Completions { is_incomplete: word.is_empty() || truncated, item_defaults, items })
    }

    /// Lints the document; canonical spellings only when `canonical`, since the first one takes a while.
    pub fn diagnostics(&self, uri: &Uri, canonical: bool) -> Vec<Diagnostic> {
        let findings = lint::lint(self.project, self.settings, self.scan(), if canonical { lint::Pass::Full } else { lint::Pass::Quick });
        findings.into_iter().filter_map(|finding| Some(self.diagnostic(uri, &finding.report, finding.level.severity()?))).collect()
    }

    fn diagnostic(&self, uri: &Uri, report: &Report, severity: DiagnosticSeverity) -> Diagnostic {
        let related: Vec<DiagnosticRelatedInformation> = report
            .labels()
            .iter()
            .map(|label| DiagnosticRelatedInformation {
                location: Location { uri: uri.clone(), range: self.range(label.start as usize, label.end as usize) },
                message: label.message.clone(),
            })
            .collect();
        let data = report
            .fixes()
            .first()
            .and_then(|edits| edits.first())
            .map(|edit| json!({ "replacement": edit.text, "title": report.fix_title() }));
        Diagnostic {
            range: self.range(report.span.start as usize, report.span.end as usize),
            severity: Some(severity),
            code: Some(NumberOrString::String(report.code.name().to_owned())),
            source: Some(SOURCE.to_owned()),
            message: report.message(),
            related_information: (!related.is_empty()).then_some(related),
            data,
            ..Diagnostic::default()
        }
    }

    pub fn code_actions(&self, uri: &Uri, range: Range, diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
        let mut out = Vec::new();
        for diagnostic in diagnostics.iter().filter(|d| d.source.as_deref() == Some(SOURCE)) {
            let data = diagnostic.data.as_ref();
            let Some(replacement) = data.and_then(|d| d.get("replacement")).and_then(|r| r.as_str()) else {
                continue;
            };
            let title = data.and_then(|d| d.get("title")).and_then(|t| t.as_str()).unwrap_or(replacement);
            out.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: capitalize(title),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(edit(uri, diagnostic.range, replacement.to_owned())),
                is_preferred: Some(true),
                ..CodeAction::default()
            }));
        }
        let start = self.doc.offset(range.start, self.encoding);
        let end = self.doc.offset(range.end, self.encoding);
        let text = &self.doc.text;
        for region in self.scan().around(start).filter(|r| r.scope == Scope::Classes && end <= r.end) {
            let source = &text[region.start..region.end];
            if source.contains(['(', '\n']) || source.trim().is_empty() {
                continue;
            }
            let classes: Vec<&str> = source.split_ascii_whitespace().collect();
            let sorted = self.project.sort(&classes);
            if sorted == classes {
                continue;
            }
            let leading = &source[..source.len() - source.trim_start().len()];
            let trailing = &source[source.trim_end().len()..];
            out.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Sort classes".to_owned(),
                kind: Some(CodeActionKind::REFACTOR_REWRITE),
                edit: Some(edit(uri, self.range(region.start, region.end), format!("{leading}{}{trailing}", sorted.join(" ")))),
                ..CodeAction::default()
            }));
        }
        out
    }

    pub fn colors(&self) -> Vec<ColorInformation> {
        let mut out = Vec::new();
        for region in &self.scan().regions {
            for token in &region.tokens {
                let info = self.project.info(&token.class);
                if region.scope == Scope::Classes && !info.swatch {
                    continue;
                }
                if let Some(color) = info.color {
                    out.push(ColorInformation { range: self.range(token.start, token.end), color: lsp_color(color) });
                }
            }
        }
        out
    }
}

fn capitalize(title: &str) -> String {
    let mut chars = title.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn lsp_color(c: Rgba) -> Color {
    Color { red: c.r, green: c.g, blue: c.b, alpha: c.a }
}

fn edit(uri: &Uri, range: Range, new_text: String) -> WorkspaceEdit {
    WorkspaceEdit { changes: Some([(uri.clone(), vec![TextEdit { range, new_text }])].into_iter().collect()), ..WorkspaceEdit::default() }
}

pub fn resolve_completion(project: &Project, mut item: CompletionItem) -> CompletionItem {
    if item.label.ends_with(':') {
        return item;
    }
    let prefix = item.data.as_ref().and_then(|d| d.get("prefix")).and_then(|p| p.as_str()).unwrap_or_default();
    let info = project.info(&format!("{prefix}{}", item.label));
    if let Some(css) = &info.css {
        if info.color.is_some() {
            item.detail = Some(css.split_whitespace().collect::<Vec<_>>().join(" "));
        } else {
            item.documentation = Some(Documentation::MarkupContent(css_markup(css)));
        }
    }
    item
}

impl Level {
    pub fn severity(self) -> Option<DiagnosticSeverity> {
        use DiagnosticSeverity as S;
        match self {
            Level::Ignore => None,
            Level::Hint => Some(S::HINT),
            Level::Info => Some(S::INFORMATION),
            Level::Warning => Some(S::WARNING),
            Level::Error => Some(S::ERROR),
        }
    }
}

fn css_markup(css: &str) -> MarkupContent {
    MarkupContent { kind: MarkupKind::Markdown, value: format!("```css\n{}\n```", css.trim_end()) }
}
