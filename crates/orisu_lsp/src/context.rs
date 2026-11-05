//! Finds where a document holds classes: class attributes, string arguments of class functions,
//! `@apply` and, with `@attributify`, utility attributes.

use crate::document::Language;
use memchr::memmem;
use orisu_scanner::attributify::Attributify;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Classes,
    /// The value of `bg="red-500 hover:blue-500"`.
    Attribute(String),
    /// A valueless attribute such as `p-4`, which is the utility itself.
    Bare,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub start: usize,
    pub end: usize,
    pub scope: Scope,
    pub tokens: Vec<Token>,
}

/// A document's regions in order of `(start, end)`, one per start, each with its tokens.
pub struct Scan {
    pub regions: Vec<Region>,
    /// The largest `end` among `regions[..=i]`.
    reach: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    /// The class the text stands for, with enclosing variant groups and attributify applied.
    pub class: String,
    pub grouped: bool,
}

pub struct Matcher<'s> {
    pub attributes: &'s [String],
    pub functions: &'s [String],
    pub attributify: Option<&'s Attributify>,
}

impl Matcher<'_> {
    pub fn scan(&self, text: &str, language: Language) -> Scan {
        let mut out = Vec::new();
        apply_regions(text, &mut out);
        if language == Language::Markup {
            self.attribute_regions(text, &mut out);
            self.function_regions(text, &mut out);
            if let Some(attributify) = self.attributify {
                self.attributify_regions(text, attributify, &mut out);
            }
        }
        out.sort_by_key(|r| (r.start, r.end));
        out.dedup_by_key(|r| r.start);
        for region in &mut out {
            region.tokens = region.tokenize(text);
        }
        let reach = out
            .iter()
            .scan(0, |reach, r| {
                *reach = r.end.max(*reach);
                Some(*reach)
            })
            .collect();
        Scan { regions: out, reach }
    }

    fn attribute_regions(&self, text: &str, out: &mut Vec<Region>) {
        let bytes = text.as_bytes();
        let mut names = Words::new(self.attributes, bytes, is_attribute_byte);
        let mut i = 0;
        while let Some((start, end)) = names.next(bytes, i, |start, end| {
            (start == 0 || bytes[start - 1].is_ascii_whitespace()) && !bytes.get(end).is_some_and(|&b| is_attribute_byte(b))
        }) {
            i = end;
            let mut j = skip_whitespace(bytes, i);
            if bytes.get(j) != Some(&b'=') {
                continue;
            }
            j = skip_whitespace(bytes, j + 1);
            let bound = bytes[start] == b':' || bytes[start] == b'[' || bytes[start..i].starts_with(b"v-bind:");
            match bytes.get(j) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let end = memchr_from(bytes, j + 1, quote);
                    if bound {
                        expression_strings(bytes, j + 1, end, out);
                    } else {
                        out.push(Region::new(j + 1, end, Scope::Classes));
                    }
                    i = end;
                }
                Some(b'{') => i = js_strings(bytes, j, out),
                _ => {}
            }
        }
    }

    fn function_regions(&self, text: &str, out: &mut Vec<Region>) {
        let bytes = text.as_bytes();
        let mut names = Words::new(self.functions, bytes, is_ident_byte);
        let mut i = 0;
        while let Some((start, end)) = names.next(bytes, i, |start, end| {
            (start == 0 || !is_ident_byte(bytes[start - 1])) && !bytes.get(end).is_some_and(|&b| is_ident_byte(b))
        }) {
            i = end;
            if start > 0 && bytes[start - 1] == b'.' {
                continue;
            }
            let line = memchr::memrchr(b'\n', &bytes[..start]).map_or(0, |n| n + 1);
            if memmem::find(&bytes[line..start], b"//").is_some() {
                continue;
            }
            match bytes.get(i) {
                Some(b'`') => i = template(bytes, i + 1, out),
                _ => {
                    let open = skip_whitespace(bytes, i);
                    if bytes.get(open) == Some(&b'(') {
                        i = js_strings(bytes, open, out);
                    }
                }
            }
        }
    }

    fn attributify_regions(&self, text: &str, attributify: &Attributify, out: &mut Vec<Region>) {
        let bytes = text.as_bytes();
        let mut i = 0;
        while let Some(at) = memchr(&bytes[i..], b'<') {
            i += at + 1;
            if !bytes.get(i).is_some_and(u8::is_ascii_alphabetic) {
                continue;
            }
            while bytes.get(i).is_some_and(|&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b':' | b'_')) {
                i += 1;
            }
            loop {
                i = skip_whitespace(bytes, i);
                match bytes.get(i) {
                    None | Some(b'>' | b'<') => break,
                    Some(b'/') if bytes.get(i + 1) == Some(&b'>') => break,
                    Some(b'{') => {
                        i = js_strings(bytes, i, &mut Vec::new());
                        continue;
                    }
                    Some(b'"' | b'\'' | b'=' | b'}' | b'/') => {
                        i += 1;
                        continue;
                    }
                    _ => {}
                }
                let start = i;
                while bytes
                    .get(i)
                    .is_some_and(|&b| !b.is_ascii_whitespace() && !matches!(b, b'=' | b'>' | b'<' | b'"' | b'\'' | b'{' | b'}'))
                {
                    i += 1;
                }
                let name = &text[start..i];
                let accepted = attributify.accepts(name.as_bytes()) && !self.attributes.iter().any(|a| a == name);
                let mut j = skip_whitespace(bytes, i);
                if bytes.get(j) != Some(&b'=') {
                    if accepted {
                        out.push(Region::new(start, i, Scope::Bare));
                    }
                    continue;
                }
                j = skip_whitespace(bytes, j + 1);
                match bytes.get(j) {
                    Some(&quote @ (b'"' | b'\'')) => {
                        let end = memchr_from(bytes, j + 1, quote);
                        if accepted {
                            out.push(Region::new(j + 1, end, Scope::Attribute(name.to_owned())));
                        }
                        i = end + 1;
                    }
                    Some(b'{') => i = js_strings(bytes, j, &mut Vec::new()),
                    _ => i = j,
                }
            }
        }
    }
}

/// Occurrences of any of a few words in a text, earliest first.
struct Words<'w> {
    /// Each word's finder and its next occurrence, `usize::MAX` once there is none.
    finders: Vec<(memmem::Finder<'w>, usize)>,
}

impl<'w> Words<'w> {
    /// Finds `words` in `bytes`, skipping those with a byte outside `valid`, which can never be a whole word.
    fn new(words: &'w [String], bytes: &[u8], valid: fn(u8) -> bool) -> Self {
        let finders = words
            .iter()
            .filter(|word| !word.is_empty() && word.bytes().all(valid))
            .map(|word| {
                let finder = memmem::Finder::new(word.as_bytes());
                let at = finder.find(bytes).unwrap_or(usize::MAX);
                (finder, at)
            })
            .collect();
        Self { finders }
    }

    /// The first occurrence at or after `from` that `accept` takes, as its start and end.
    fn next(&mut self, bytes: &[u8], from: usize, accept: impl Fn(usize, usize) -> bool) -> Option<(usize, usize)> {
        loop {
            let mut first: Option<(usize, usize)> = None;
            for (k, (finder, at)) in self.finders.iter_mut().enumerate() {
                if *at < from {
                    *at = finder.find(&bytes[from..]).map_or(usize::MAX, |n| from + n);
                }
                if *at != usize::MAX && first.is_none_or(|(start, _)| *at < start) {
                    first = Some((*at, k));
                }
            }
            let (start, k) = first?;
            let (finder, at) = &mut self.finders[k];
            let end = start + finder.needle().len();
            if accept(start, end) {
                return Some((start, end));
            }
            *at = finder.find(&bytes[start + 1..]).map_or(usize::MAX, |n| start + 1 + n);
        }
    }
}

impl Scan {
    /// The regions holding `offset`, in order.
    pub fn around(&self, offset: usize) -> impl DoubleEndedIterator<Item = &Region> {
        let end = self.regions.partition_point(|r| r.start <= offset);
        let start = self.reach[..end].partition_point(|&reach| reach < offset);
        self.regions[start..end].iter().filter(move |r| offset <= r.end)
    }
}

impl Region {
    fn new(start: usize, end: usize, scope: Scope) -> Self {
        Self { start, end, scope, tokens: Vec::new() }
    }

    fn tokenize(&self, text: &str) -> Vec<Token> {
        let source = &text[self.start..self.end];
        match &self.scope {
            Scope::Bare => vec![Token { start: self.start, end: self.end, class: format!("[{source}=\"\"]"), grouped: false }],
            Scope::Attribute(name) => words(source)
                .filter(|&(_, word)| !word.contains('"'))
                .map(|(at, word)| Token {
                    start: self.start + at,
                    end: self.start + at + word.len(),
                    class: format!("[{name}~=\"{word}\"]"),
                    grouped: false,
                })
                .collect(),
            Scope::Classes => {
                let mut out = Vec::new();
                class_tokens(source, self.start, &mut out);
                out
            }
        }
    }
}

fn words(source: &str) -> impl Iterator<Item = (usize, &str)> {
    source.split_ascii_whitespace().map(move |word| (word.as_ptr() as usize - source.as_ptr() as usize, word))
}

/// Splits classes, expanding `hover:(a focus:(b c))` groups into `hover:a`, `hover:focus:b`, …
fn class_tokens(source: &str, base: usize, out: &mut Vec<Token>) {
    let bytes = source.as_bytes();
    let mut prefixes = vec![String::new()];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i] == b')' {
            if prefixes.len() > 1 {
                prefixes.pop();
            }
            i += 1;
            continue;
        }
        let start = i;
        let mut group = false;
        while i < bytes.len() {
            match bytes[i] {
                b if b.is_ascii_whitespace() || b == b')' => break,
                b'(' if i > start && bytes[i - 1] == b':' => {
                    group = true;
                    break;
                }
                open @ (b'[' | b'(') => i = close(bytes, i, open),
                _ => i += 1,
            }
        }
        let text = &source[start..i];
        let prefix = prefixes.last().map_or("", String::as_str);
        if group {
            prefixes.push(format!("{prefix}{text}"));
            i += 1;
        } else if !text.is_empty() {
            out.push(Token { start: base + start, end: base + i, class: [prefix, text].concat(), grouped: prefixes.len() > 1 });
        }
    }
}

/// The index after the bracket balancing the one at `open`.
fn close(bytes: &[u8], open: usize, kind: u8) -> usize {
    let closing = if kind == b'[' { b']' } else { b')' };
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if b == kind {
            depth += 1;
        } else if b == closing {
            depth -= 1;
            if depth == 0 {
                return i + 1;
            }
        } else if b.is_ascii_whitespace() {
            return i;
        }
    }
    bytes.len()
}

fn apply_regions(text: &str, out: &mut Vec<Region>) {
    let bytes = text.as_bytes();
    for (at, _) in text.match_indices("@apply") {
        let start = at + "@apply".len();
        if !bytes.get(start).is_some_and(u8::is_ascii_whitespace) {
            continue;
        }
        let end = bytes[start..].iter().position(|&b| matches!(b, b';' | b'}')).map_or(bytes.len(), |n| start + n);
        let end = text[start..end].find("!important").map_or(end, |n| start + n);
        out.push(Region::new(start, end, Scope::Classes));
    }
}

/// Collects every string literal in the JS expression bracketed at `open`; returns the index
/// after its closing bracket.
fn js_strings(bytes: &[u8], open: usize, out: &mut Vec<Region>) -> usize {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            quote @ (b'"' | b'\'') => {
                let end = string_end(bytes, i + 1, quote);
                out.push(Region::new(i + 1, end, Scope::Classes));
                i = end;
            }
            b'`' => {
                i = template(bytes, i + 1, out);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = memchr_from(bytes, i, b'\n'),
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = bytes[i + 2..].windows(2).position(|w| w == b"*/").map_or(bytes.len(), |n| i + n + 3);
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// Collects the string literals of a JS expression held in an attribute value, like Vue's
/// `:class="['a', ok && 'b']"`.
fn expression_strings(bytes: &[u8], start: usize, end: usize, out: &mut Vec<Region>) {
    let mut i = start;
    while i < end {
        match bytes[i] {
            quote @ (b'"' | b'\'') => {
                let close = string_end(bytes, i + 1, quote).min(end);
                out.push(Region::new(i + 1, close, Scope::Classes));
                i = close + 1;
            }
            b'`' => i = template(&bytes[..end], i + 1, out),
            _ => i += 1,
        }
    }
}

/// Collects the text of the template literal starting at `start`, recursing into `${…}`; returns
/// the index after the closing backtick.
fn template(bytes: &[u8], start: usize, out: &mut Vec<Region>) -> usize {
    let mut text_start = start;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => {
                out.push(Region::new(text_start, i, Scope::Classes));
                return i + 1;
            }
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                out.push(Region::new(text_start, i, Scope::Classes));
                i = js_strings(bytes, i + 1, out);
                text_start = i;
            }
            _ => i += 1,
        }
    }
    out.push(Region::new(text_start, bytes.len(), Scope::Classes));
    bytes.len()
}

fn string_end(bytes: &[u8], mut i: usize, quote: u8) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' => return i,
            b if b == quote => return i,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn memchr(bytes: &[u8], needle: u8) -> Option<usize> {
    bytes.iter().position(|&b| b == needle)
}

fn memchr_from(bytes: &[u8], from: usize, needle: u8) -> usize {
    memchr(&bytes[from..], needle).map_or(bytes.len(), |n| from + n)
}

fn skip_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
}

fn is_attribute_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'[' | b']' | b'@' | b'.')
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$')
}
