use crate::classify::classify;

use crate::extractor::Span;

#[inline(always)]
pub fn is_valid_before_boundary(c: &u8) -> bool {
    matches!(c.into(), Class::Common | Class::Before)
}

#[inline(always)]
pub fn is_valid_after_boundary(c: &u8) -> bool {
    matches!(c.into(), Class::Common | Class::After)
}

#[inline(always)]
pub fn has_valid_boundaries(span: &Span, input: &[u8]) -> bool {
    let before = { if span.start == 0 { b'\0' } else { input[span.start - 1] } };

    let after = { if span.end >= input.len() - 1 { b'\0' } else { input[span.end + 1] } };

    is_valid_before_boundary(&before) && is_valid_after_boundary(&after)
}

classify! {
    #[derive(Debug)]
    enum Class {
        Common = b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'"' | b'\'' | b'`' | b'\0',
        Before = b'.' | b'}' | b'>',
        After = b']' | b'{' | b'=' | b'\\' | b'<',
        _ => Other,
    }
}
