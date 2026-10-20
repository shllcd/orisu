use crate::cursor;
use crate::extractor::pre_processors::pre_processor::PreProcessor;
use bstr::ByteSlice;

#[derive(Debug, Default)]
pub struct Clojure;

/// This is meant to be a rough estimate of a valid ClojureScript keyword
///
/// This can be approximated by the following regex:
/// /::?[a-zA-Z0-9!#$%&*+./:<=>?_|-]+/
///
/// However, keywords are intended to be detected as utilities. Since the set
/// of valid characters in a utility (outside of arbitrary values) is smaller,
/// along with the fact that neither `[]` nor `()` are allowed in keywords we
/// can simplify this list quite a bit.
#[inline]
fn is_keyword_character(byte: u8) -> bool {
    (matches!(byte, b'!' | b'#' | b'%' | b'*' | b'+' | b'-' | b'.' | b'/' | b':' | b'_') | byte.is_ascii_alphanumeric())
}

impl PreProcessor for Clojure {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        let content = content.replace(":class", "      ").replace(":className", "          ");
        let len = content.len();
        let mut result = content.to_vec();
        let mut cursor = cursor::Cursor::new(&content);

        while cursor.pos < len {
            match cursor.curr() {
                b'"' => {
                    result[cursor.pos] = b' ';
                    cursor.advance();

                    while cursor.pos < len {
                        match cursor.curr() {
                            b'\\' => cursor.advance_twice(),

                            b'"' => {
                                result[cursor.pos] = b' ';
                                break;
                            }

                            _ => cursor.advance(),
                        };
                    }
                }

                b';' if matches!(cursor.next(), b';') => {
                    while cursor.pos < len && cursor.curr() != b'\n' {
                        result[cursor.pos] = b' ';
                        cursor.advance();
                    }
                }

                b':' => {
                    result[cursor.pos] = b' ';
                    cursor.advance();

                    while cursor.pos < len {
                        match cursor.curr() {
                            b'.' if cursor.prev().is_ascii_digit() && cursor.next().is_ascii_digit() => {}
                            b'.' => {
                                result[cursor.pos] = b' ';
                            }
                            _ if !is_keyword_character(cursor.curr()) => {
                                result[cursor.pos] = b' ';
                                break;
                            }

                            _ => {}
                        };

                        cursor.advance();
                    }
                }

                b'\'' if matches!(cursor.next(), b'[' | b'(') => {
                    result[cursor.pos] = b' ';
                    cursor.advance();
                    result[cursor.pos] = b' ';
                    let end = match cursor.curr() {
                        b'[' => b']',
                        b'(' => b')',
                        _ => unreachable!(),
                    };

                    while cursor.pos < len {
                        match cursor.curr() {
                            x if x == end => {
                                result[cursor.pos] = b' ';
                                break;
                            }

                            b'"' => {
                                result[cursor.pos] = b' ';
                                cursor.advance();

                                while cursor.pos < len {
                                    match cursor.curr() {
                                        b'\\' => cursor.advance_twice(),

                                        b'"' => {
                                            result[cursor.pos] = b' ';
                                            break;
                                        }

                                        _ => cursor.advance(),
                                    };
                                }
                            }
                            _ => {}
                        };

                        cursor.advance();
                    }
                }

                b'\'' if !cursor.next().is_ascii_whitespace() => {
                    result[cursor.pos] = b' ';
                    cursor.advance();

                    while cursor.pos < len {
                        match cursor.curr() {
                            _ if !is_keyword_character(cursor.curr()) => {
                                result[cursor.pos] = b' ';
                                break;
                            }

                            _ => {}
                        };

                        cursor.advance();
                    }
                }

                _ => {
                    result[cursor.pos] = b' ';
                }
            };

            cursor.advance();
        }

        result
    }
}
