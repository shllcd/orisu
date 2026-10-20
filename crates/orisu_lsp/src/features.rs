use crate::color::Rgba;
use crate::context::{Region, Scope, Token};
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
use rustc_hash::FxHashMap;
use serde::Serialize;
use serde_json::json;

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
pub struct Completions {
    is_incomplete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_defaults: Option<Defaults>,
    items: Vec<Item>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Defaults {
    edit_range: Range,
    data: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    label: String,
    kind: CompletionItemKind,
    sort_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_edit: Option<TextEdit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

impl Ctx<'_> {
    fn regions(&self) -> Vec<Region<'_>> {
        lint::regions(self.project, self.settings, &self.doc.text, self.doc.language)
    }

    fn range(&self, start: usize, end: usize) -> Range {
        self.doc.range(start, end, self.encoding)
    }

    fn token_at(&self, offset: usize) -> Option<Token> {
        self.regions()
            .iter()
            .filter(|r| r.start <= offset && offset <= r.end)
            .find_map(|r| r.tokens(&self.doc.text).into_iter().find(|t| t.start <= offset && offset <= t.end))
    }

    pub fn hover(&self, offset: usize) -> Option<Hover> {
        let token = self.token_at(offset)?;
        let css = self.project.info(&token.class).css.clone()?;
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("```css\n{}\n```", css.trim_end()),
            }),
            range: Some(self.range(token.start, token.end)),
        })
    }

    pub fn completion(&self, offset: usize) -> Option<Completions> {
        let regions = self.regions();
        let region = regions.iter().rev().find(|r| r.start <= offset && offset <= r.end)?;
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
        let prefix = match region.scope {
            Scope::Attribute(name) => format!("{name}-"),
            _ => String::new(),
        };
        let data = json!({ "entry": self.project.entry, "prefix": prefix });
        let item = |label: &str, kind, sort_text: String, documentation: Option<String>, data: Option<&serde_json::Value>| Item {
            label: label.to_owned(),
            kind,
            sort_text,
            documentation,
            text_edit: (!self.item_defaults).then(|| TextEdit { range, new_text: label.to_owned() }),
            data: data.filter(|_| !self.item_defaults).cloned(),
        };
        let colors = self.project.theme_colors();
        let mut items = Vec::new();
        let mut truncated = false;
        for (i, entry) in self.project.classes().iter().enumerate() {
            if items.len() >= MAX_ITEMS {
                truncated = true;
                break;
            }
            let Some(label) = entry.name.strip_prefix(&prefix).filter(|label| matches(label)) else {
                continue;
            };
            let color = names_color(&entry.name, colors);
            let kind = if color.is_some() { CompletionItemKind::COLOR } else { CompletionItemKind::CONSTANT };
            items.push(item(label, kind, format!("a{i:05}"), color.map(|c| c.hex()), Some(&data)));
        }
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
                    items.push(item(&label, CompletionItemKind::MODULE, format!("b{n:05}"), None, None));
                }
            }
        }
        let item_defaults = self.item_defaults.then_some(Defaults { edit_range: range, data });
        Some(Completions { is_incomplete: word.is_empty() || truncated, item_defaults, items })
    }

    /// Lints the document; canonical spellings only when `canonical`, since the first one takes a while.
    pub fn diagnostics(&self, uri: &Uri, canonical: bool) -> Vec<Diagnostic> {
        let findings = lint::lint(self.project, self.settings, &self.doc.text, self.doc.language, canonical);
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
        for region in self.regions().iter().filter(|r| r.scope == Scope::Classes && r.start <= start && end <= r.end) {
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
        let colors = self.project.theme_colors();
        let mut out = Vec::new();
        for region in self.regions() {
            for token in region.tokens(&self.doc.text) {
                let utility = segment(&token.class, b':').last().copied().unwrap_or_default().to_owned();
                if region.scope == Scope::Classes
                    && names_color(&utility, colors).is_none()
                    && !utility.contains("-[")
                    && !utility.contains("-(")
                {
                    continue;
                }
                if let Some(color) = self.project.info(&token.class).color {
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

/// The theme color named by a dash-separated suffix of `class`, ignoring any `/modifier`.
fn names_color(class: &str, colors: &FxHashMap<String, Rgba>) -> Option<Rgba> {
    let class = class.split('/').next().unwrap_or(class);
    class.match_indices('-').find_map(|(i, _)| colors.get(&class[i + 1..]).copied())
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
            item.documentation = Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("```css\n{}\n```", css.trim_end()),
            }));
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
