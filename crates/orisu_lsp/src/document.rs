use lsp_types::{Position, Range};

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
}

impl Document {
    pub fn new(text: String, language: Language, version: i32) -> Self {
        let lines = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
        Self { text, language, version, lines }
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
