use crate::cursor;
use crate::extractor::bracket_stack;
use crate::extractor::pre_processors::pre_processor::PreProcessor;
use crate::scanner::pre_process_input;
use bstr::ByteVec;
use regex::{Regex, RegexBuilder};
use std::sync;

static TEMPLATE_START_REGEX: sync::LazyLock<Regex> =
    sync::LazyLock::new(|| RegexBuilder::new(r#"\s*([a-z0-9_-]+)_template\s*<<[-~]?([A-Z]+)$"#).multi_line(true).build().unwrap());

static TEMPLATE_END_REGEX: sync::LazyLock<Regex> =
    sync::LazyLock::new(|| RegexBuilder::new(r#"^\s*([A-Z]+)"#).multi_line(true).build().unwrap());

#[derive(Debug, Default)]
pub struct Ruby;

impl PreProcessor for Ruby {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        let len = content.len();
        let mut result = content.to_vec();
        let mut cursor = cursor::Cursor::new(content);
        let mut bracket_stack = bracket_stack::BracketStack::default();

        if let Ok(content_as_str) = std::str::from_utf8(content) {
            let starts = TEMPLATE_START_REGEX.captures_iter(content_as_str).collect::<Vec<_>>();
            let ends = TEMPLATE_END_REGEX.captures_iter(content_as_str).collect::<Vec<_>>();

            for start in starts.iter() {
                let lang = start.get(1).unwrap().as_str();

                let delimiter_start = start.get(2).unwrap().as_str();

                let body_start = start.get(0).unwrap().end();

                for end in ends.iter() {
                    let body_end = end.get(0).unwrap().start();
                    if body_end < body_start {
                        continue;
                    }

                    let delimiter_end = end.get(1).unwrap().as_str();
                    if delimiter_end != delimiter_start {
                        continue;
                    }

                    let body = &content_as_str[body_start..body_end];
                    let replaced = pre_process_input(body.as_bytes().to_vec(), &lang.to_ascii_lowercase());

                    result.replace_range(body_start..body_end, replaced);
                    break;
                }
            }
        }

        while cursor.pos < len {
            match cursor.curr() {
                b'"' => {
                    cursor.advance();

                    while cursor.pos < len {
                        match cursor.curr() {
                            b'\\' => cursor.advance_twice(),

                            b'"' => break,

                            _ => cursor.advance(),
                        };
                    }

                    cursor.advance();
                    continue;
                }

                b'\'' => {
                    cursor.advance();

                    while cursor.pos < len {
                        match cursor.curr() {
                            b'\\' => cursor.advance_twice(),

                            b'\'' => break,

                            _ => cursor.advance(),
                        };
                    }

                    cursor.advance();
                    continue;
                }

                b'#' if !matches!(cursor.prev(), b'%') && !matches!(cursor.next(), b'{') => {
                    result[cursor.pos] = b' ';
                    cursor.advance();

                    while cursor.pos < len {
                        match cursor.curr() {
                            b'\n' => break,

                            _ => {
                                result[cursor.pos] = b' ';
                                cursor.advance();
                            }
                        };
                    }

                    cursor.advance();
                    continue;
                }

                _ => {}
            }

            if cursor.curr() != b'%' || !matches!(cursor.next(), b'w' | b'W' | b'p') {
                cursor.advance();
                continue;
            }

            cursor.advance_twice();

            let boundary = match cursor.curr() {
                b'[' => b']',
                b'(' => b')',
                b'{' => b'}',
                b'#' => b'#',
                b' ' => b'\n',
                _ => {
                    cursor.advance();
                    continue;
                }
            };

            bracket_stack.reset();

            result[cursor.pos] = b' ';

            cursor.advance();

            while cursor.pos < len {
                match cursor.curr() {
                    b'\\' => {
                        if cursor.next() == b' ' {
                            result[cursor.pos] = b' ';
                        }

                        cursor.advance();
                    }

                    b'[' | b'(' | b'{' => {
                        bracket_stack.push(cursor.curr());
                    }

                    b']' | b')' | b'}' if !bracket_stack.is_empty() => {
                        if !bracket_stack.pop(cursor.curr()) {
                            cursor.advance();
                        }
                    }

                    _ if cursor.curr() == boundary => {
                        if boundary != b'\n' {
                            result[cursor.pos] = b' ';
                        }

                        break;
                    }

                    _ => {}
                }

                cursor.advance();
            }
        }

        result
    }
}
