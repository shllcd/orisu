use std::fmt::Write;

#[cfg(debug_assertions)]
use crate::Entry;
use crate::{Fix, Label, Position, Span, utf16_len};

/// `{key}` placeholders of `template` in order; braces around anything but `[a-z_]+` are text.
pub fn placeholders(template: &str) -> impl Iterator<Item = &str> {
    template.split('{').skip(1).filter_map(|rest| {
        let key = &rest[..rest.find('}')?];
        (!key.is_empty() && key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')).then_some(key)
    })
}

pub fn fill(template: &str, args: &[(&'static str, String)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            let key = &after[..close];
            args.iter().find(|(k, _)| *k == key).map(|(_, value)| (value, close))
        });
        match value {
            Some((value, close)) => {
                out.push_str(value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(debug_assertions)]
pub fn assert_args(entry: &Entry, args: &[(&'static str, String)], fixed: bool) {
    let fix = entry.fix.filter(|_| fixed);
    let mut expected: Vec<&str> = placeholders(entry.message).chain(fix.into_iter().flat_map(placeholders)).collect();
    expected.sort_unstable();
    expected.dedup();
    let mut given: Vec<&str> = args.iter().map(|(key, _)| *key).collect();
    given.sort_unstable();
    assert_eq!(expected, given, "arguments of {}", entry.name);
}

pub fn text(message: &str, labels: &[Label], fixes: &[Fix], file: &str, source: &str, lines: &LineIndex, span: Span) -> String {
    let mut out = String::from(message);
    let start = lines.position(source, span.start);
    let _ = write!(out, "\n  at {file}:{}:{}", start.line, start.column + 1);
    code_frame(&mut out, source, lines, span, start.line);
    for label in labels {
        let at = lines.position(source, label.start);
        let _ = write!(out, "\n  note: {} ({file}:{}:{})", label.message, at.line, at.column + 1);
    }
    for fix in fixes {
        out.push_str("\n  fix: ");
        out.push_str(&fix.title);
    }
    out
}

fn code_frame(out: &mut String, source: &str, lines: &LineIndex, span: Span, line: u32) {
    let first = line.saturating_sub(1).max(1);
    let last = (line + 1).min(lines.count());
    let width = last.to_string().len();
    for number in first..=last {
        let text = lines.text(source, number);
        let gutter = if number == line { '>' } else { ' ' };
        let _ = write!(out, "\n{gutter} {number:>width$} | {text}");
        if number != line {
            continue;
        }
        let line_start = lines.start(number);
        let from = (span.start as usize - line_start).min(text.len());
        let to = (span.end as usize).clamp(span.start as usize, line_start + text.len()) - line_start;
        let pad: String = text[..from].chars().map(|c| if c == '\t' { '\t' } else { ' ' }).collect();
        let carets = text.get(from..to).map_or(0, |s| s.chars().count()).max(1);
        let _ = write!(out, "\n  {:width$} | {pad}{}", "", "^".repeat(carets));
    }
}

/// Line starts of a text, for `offset → (line, column)`.
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.bytes().enumerate().filter(|&(_, b)| b == b'\n').map(|(i, _)| i + 1));
        Self { starts }
    }

    /// 0-based line and UTF-16 column.
    pub fn locate(&self, text: &str, offset: usize) -> (u32, u32) {
        let offset = offset.min(text.len());
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        (line as u32, utf16_len(&text[self.starts[line]..offset]))
    }

    pub fn position(&self, text: &str, offset: u32) -> Position {
        let (line, column) = self.locate(text, offset as usize);
        Position { offset, line: line + 1, column }
    }

    fn count(&self) -> u32 {
        self.starts.len() as u32
    }

    fn start(&self, line: u32) -> usize {
        self.starts[line as usize - 1]
    }

    fn text<'t>(&self, text: &'t str, line: u32) -> &'t str {
        let start = self.start(line);
        let end = self.starts.get(line as usize).map_or(text.len(), |&next| next - 1);
        text[start..end].strip_suffix('\r').unwrap_or(&text[start..end])
    }
}
