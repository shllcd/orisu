use crate::cursor;
use crate::extractor::bracket_stack::BracketStack;
use crate::extractor::machine::{Machine, MachineState};
use crate::extractor::pre_processors::pre_processor::PreProcessor;
use crate::extractor::variant_machine::VariantMachine;

#[derive(Debug, Default)]
pub struct Slim;

impl PreProcessor for Slim {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        let len = content.len();
        let mut result = content.to_vec();
        let mut cursor = cursor::Cursor::new(content);
        let mut bracket_stack = BracketStack::default();

        while cursor.pos < len {
            match cursor.curr() {
                b'.' => {
                    if !bracket_stack.is_empty() {
                        cursor.advance();
                        continue;
                    }

                    if cursor.prev().is_ascii_digit() && cursor.next().is_ascii_digit() {
                        let mut next_cursor = cursor;
                        next_cursor.advance();

                        let mut variant_machine = VariantMachine::default();
                        if let MachineState::Done(_) = variant_machine.next(&mut next_cursor) {
                            result[cursor.pos] = b' ';
                        }
                    } else {
                        result[cursor.pos] = b' ';
                    }
                }

                b'%' if matches!(cursor.next(), b'w' | b'W') && matches!(cursor.input.get(cursor.pos + 2), Some(b'[' | b'(' | b'{')) => {
                    result[cursor.pos] = b' ';
                    cursor.advance();
                    result[cursor.pos] = b' ';
                    cursor.advance();
                    result[cursor.pos] = b' ';
                    bracket_stack.push(cursor.curr());
                    cursor.advance();
                    continue;
                }

                b'[' if bracket_stack.is_empty() && matches!(cursor.prev(), b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9') => {
                    result[cursor.pos] = b' ';
                    bracket_stack.push(cursor.curr());
                }

                b'(' if bracket_stack.is_empty() && !matches!(cursor.prev(), b'-' | b'/') => {
                    result[cursor.pos] = b' ';
                    bracket_stack.push(cursor.curr());
                }

                b'(' | b'[' | b'{' => {
                    bracket_stack.push(cursor.curr());
                }

                b')' | b']' | b'}' if !bracket_stack.is_empty() => {
                    bracket_stack.pop(cursor.curr());
                }

                _ => {}
            };

            cursor.advance();
        }

        result
    }
}
