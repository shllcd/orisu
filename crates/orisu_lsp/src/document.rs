use crate::context::Scan;
use lsp_types::{Position, Range};
use std::cell::OnceCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Css,
    Markup,
}

impl Language {
    pub fn detect(language_id: &str, path: &str) -> Self {
        let css = matches!(language_id, "css" | "scss" | "sass" | "less" | "postcss" | "tailwindcss")
            || [".css", ".scss", ".sass", ".less", ".pcss"].iter().any(|ext| path.ends_with(ext));
        if css { Self::Css } else { Self::Markup }
    }
}

pub struct Document {
    pub text: String,
    pub language: Language,
    pub version: i32,
    lines: Vec<usize>,
    scan: OnceCell<Scan>,
}

impl Document {
    pub fn new(text: String, language: Language, version: i32) -> Self {
        let lines = std::iter::once(0).chain(memchr::memchr_iter(b'\n', text.as_bytes()).map(|i| i + 1)).collect();
        Self { text, language, version, lines, scan: OnceCell::new() }
    }

    /// Replaces `range`, or with `None` the whole text, by `new`.
    pub fn edit(&mut self, range: Option<Range>, new: &str, encoding: Encoding) {
        let Some(range) = range else {
            *self = Self::new(new.to_owned(), self.language, self.version);
            return;
        };
        let start = self.offset(range.start, encoding);
        let end = self.offset(range.end, encoding).max(start);
        self.text.replace_range(start..end, new);
        let kept = self.lines.partition_point(|&line| line <= start);
        let removed = self.lines.partition_point(|&line| line <= end);
        let added = memchr::memchr_iter(b'\n', new.as_bytes()).map(|i| start + i + 1);
        let shifted = kept + added.clone().count();
        self.lines.splice(kept..removed, added);
        for line in &mut self.lines[shifted..] {
            *line = *line - end + start + new.len();
        }
        self.scan.take();
    }

    /// The document's regions, scanned by `scan` on first use after a change.
    pub fn scan(&self, scan: impl FnOnce() -> Scan) -> &Scan {
        self.scan.get_or_init(scan)
    }

    /// Drops what was derived from the project and settings, which changed.
    pub fn invalidate(&mut self) {
        self.scan.take();
    }

    pub fn offset(&self, position: Position, encoding: Encoding) -> usize {
        let Some(&start) = self.lines.get(position.line as usize) else {
            return self.text.len();
        };
        let line = self.text[start..].split('\n').next().unwrap_or_default();
        let column = position.character as usize;
        match encoding {
            Encoding::Utf8 => start + line.floor_char_boundary(column),
            Encoding::Utf16 => {
                let mut units = 0;
                for (i, c) in line.char_indices() {
                    if units >= column {
                        return start + i;
                    }
                    units += c.len_utf16();
                }
                start + line.len()
            }
        }
    }

    pub fn position(&self, offset: usize, encoding: Encoding) -> Position {
        let line = self.lines.partition_point(|&start| start <= offset) - 1;
        let start = self.lines[line];
        let prefix = &self.text[start..offset];
        let character = match encoding {
            Encoding::Utf8 => prefix.len(),
            Encoding::Utf16 => prefix.encode_utf16().count(),
        };
        Position::new(line as u32, character as u32)
    }

    pub fn range(&self, start: usize, end: usize, encoding: Encoding) -> Range {
        Range::new(self.position(start, encoding), self.position(end, encoding))
    }
}
