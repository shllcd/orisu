use orisu_allocator::{AllocExt, Allocator};

#[repr(C, align(64))]
pub struct ByteTable(pub [bool; 256]);

impl ByteTable {
    pub const fn new(bytes: &[u8]) -> Self {
        let mut table = [false; 256];
        let mut i = 0;
        while i < bytes.len() {
            table[bytes[i] as usize] = true;
            i += 1;
        }
        Self(table)
    }

    #[inline(always)]
    pub const fn has(&self, b: u8) -> bool {
        self.0[b as usize]
    }

    #[inline]
    pub fn find(&self, haystack: &[u8], from: usize) -> Option<usize> {
        const BATCH: usize = 16;
        let mut pos = from;
        let len = haystack.len();
        while pos + BATCH <= len {
            let chunk = &haystack[pos..pos + BATCH];
            for (i, &b) in chunk.iter().enumerate() {
                if self.has(b) {
                    return Some(pos + i);
                }
            }
            pos += BATCH;
        }
        haystack[pos.min(len)..].iter().position(|&b| self.has(b)).map(|i| i + pos)
    }

    #[inline]
    pub fn skip(&self, haystack: &[u8], mut from: usize) -> usize {
        while from < haystack.len() && self.has(haystack[from]) {
            from += 1;
        }
        from
    }
}

#[inline]
fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{FEFF}'
}

/// `String.prototype.trim` semantics with an ASCII fast path.
#[inline]
pub fn trim(s: &str) -> &str {
    let b = s.as_bytes();
    let mut start = 0;
    let mut end = b.len();
    while start < end && matches!(b[start], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        start += 1;
    }
    while end > start && matches!(b[end - 1], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        end -= 1;
    }
    let s = &s[start..end];
    let b = s.as_bytes();
    if b.first().is_some_and(|&c| c >= 0x80) || b.last().is_some_and(|&c| c >= 0x80) { s.trim_matches(is_js_space) } else { s }
}

/// CSS whitespace: space, tab, line feed, carriage return and form feed.
pub static CSS_WS: ByteTable = ByteTable::new(b" \t\n\r\x0c");

/// What Tailwind's CSS and attribute-selector parsers skip as whitespace: CSS whitespace without form feed.
pub static WS_WITHOUT_FORM_FEED: ByteTable = ByteTable::new(b" \t\n\r");

/// Index of the quote closing the one at `open`; a backslash escapes the byte after it.
pub fn string_end(bytes: &[u8], open: usize) -> Option<usize> {
    let quote = bytes[open];
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Index of the `)` or `]` closing the bracket at `open`. Only that bracket pair nests; quotes and escapes are
/// plain bytes, as in Tailwind's selector parser.
pub fn matching_close(bytes: &[u8], open: usize) -> Option<usize> {
    let open_byte = bytes[open];
    let close_byte = if open_byte == b'(' { b')' } else { b']' };
    let mut nesting = 0usize;
    for (i, &b) in bytes.iter().enumerate().skip(open + 1) {
        if b == open_byte {
            nesting += 1;
        } else if b == close_byte {
            if nesting == 0 {
                return Some(i);
            }
            nesting -= 1;
        }
    }
    None
}

/// Copies `input` into `alloc` with every `\r\n` replaced by `\n`.
pub fn normalize_newlines<'a>(input: &str, alloc: &'a Allocator) -> &'a str {
    if input.contains("\r\n") { alloc.str(&input.replace("\r\n", "\n")) } else { alloc.str(input) }
}
