use crate::extractor::MachineState;
use crate::extractor::bracket_stack;
use crate::extractor::cursor;
use crate::extractor::machine::Machine;
use crate::extractor::pre_processors::pre_processor::PreProcessor;
use crate::extractor::variant_machine::VariantMachine;
use bstr::ByteSlice;

#[derive(Debug, Default)]
pub struct Rust;

impl PreProcessor for Rust {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        let replaced_content = content.replace(" class:", " class ").replace("\tclass:", " class ").replace("\nclass:", " class ");

        if replaced_content.contains_str(b"html!") { self.process_maud_templates(&replaced_content) } else { replaced_content }
    }
}

impl Rust {
    fn process_maud_templates(&self, replaced_content: &[u8]) -> Vec<u8> {
        let len = replaced_content.len();
        let mut result = replaced_content.to_vec();
        let mut cursor = cursor::Cursor::new(replaced_content);
        let mut bracket_stack = bracket_stack::BracketStack::default();

        while cursor.pos < len {
            match cursor.curr() {
                b'\\' => {
                    cursor.advance_twice();
                    continue;
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

                b'[' => {
                    bracket_stack.push(cursor.curr());

                    if !matches!(cursor.prev(), b'-') {
                        result[cursor.pos] = b' ';
                    }
                }

                b']' if !bracket_stack.is_empty() => {
                    bracket_stack.pop(cursor.curr());
                }

                _ => {}
            };

            cursor.advance();
        }

        result
    }
}
