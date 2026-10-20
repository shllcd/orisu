use crate::cursor;
use crate::extractor::bracket_stack::BracketStack;
use crate::extractor::pre_processors::pre_processor::PreProcessor;

#[derive(Debug, Default)]
pub struct Elixir;

impl PreProcessor for Elixir {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        let mut cursor = cursor::Cursor::new(content);
        let mut result = content.to_vec();
        let mut bracket_stack = BracketStack::default();

        while cursor.pos < content.len() {
            if cursor.curr() != b'~' {
                cursor.advance();
                continue;
            }

            if !matches!(cursor.next(), b'c' | b'C' | b's' | b'S' | b'w' | b'W') {
                cursor.advance();
                continue;
            }

            cursor.advance_twice();

            if !matches!(cursor.curr(), b'(' | b'[' | b'{') {
                continue;
            }

            result[cursor.pos] = b' ';

            bracket_stack.push(cursor.curr());

            while cursor.pos < content.len() {
                cursor.advance();

                match cursor.curr() {
                    b'\\' => cursor.advance_twice(),
                    b'(' | b'[' | b'{' => {
                        bracket_stack.push(cursor.curr());
                    }
                    b')' | b']' | b'}' if !bracket_stack.is_empty() => {
                        bracket_stack.pop(cursor.curr());

                        if bracket_stack.is_empty() {
                            result[cursor.pos] = b' ';
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }

        result
    }
}
