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
