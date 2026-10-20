use crate::cursor;
use crate::extractor::bracket_stack::BracketStack;
use crate::extractor::machine::{Machine, MachineState};
use crate::extractor::pre_processors::pre_processor::PreProcessor;
use crate::extractor::variant_machine::VariantMachine;
use crate::scanner::pre_process_input;
use bstr::ByteVec;

#[derive(Debug, Default)]
pub struct Haml;

impl PreProcessor for Haml {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        let len = content.len();
        let mut result = content.to_vec();
        let mut cursor = cursor::Cursor::new(content);
        let mut bracket_stack = BracketStack::default();

        let mut last_newline_position = 0;

        while cursor.pos < len {
            match cursor.curr() {
                b'\\' => {
                    cursor.advance_twice();
                    continue;
                }

                b'\n' => {
                    last_newline_position = cursor.pos;
                    cursor.advance();
                    continue;
                }

                b'-' if cursor.input[last_newline_position..cursor.pos].iter().all(u8::is_ascii_whitespace)
                    && matches!(cursor.next(), b'#') =>
                {
                    let updated_last_newline_position = self.skip_indented_block(&mut cursor, last_newline_position);

                    last_newline_position = updated_last_newline_position;
                }

                b'/' if cursor.input[last_newline_position..cursor.pos].iter().all(u8::is_ascii_whitespace) => {
                    let updated_last_newline_position = self.skip_indented_block(&mut cursor, last_newline_position);

                    last_newline_position = updated_last_newline_position;
                }

                b'-' | b'=' | b'~' if cursor.input[last_newline_position..cursor.pos].iter().all(u8::is_ascii_whitespace) => {
                    let mut start = cursor.pos;
                    let end = self.skip_indented_block(&mut cursor, last_newline_position);

                    start += 1;

                    let ruby_code = &cursor.input[start..end];

                    last_newline_position = end;

                    let replaced = pre_process_input(ruby_code.to_vec(), "rb");
                    result.replace_range(start..end, replaced);
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

                b'#' | b'=' if bracket_stack.is_empty() => {
                    result[cursor.pos] = b' ';
                }

                b'(' | b'[' | b'{' => {
                    if bracket_stack.is_empty() {
                        result[cursor.pos] = b' ';
                    }
                    bracket_stack.push(cursor.curr());
                }

                b')' | b']' | b'}' if !bracket_stack.is_empty() => {
                    bracket_stack.pop(cursor.curr());

                    if bracket_stack.is_empty() {
                        result[cursor.pos] = b' ';
                    }
                }

                _ => {}
            };

            cursor.advance();
        }

        result
    }
}

impl Haml {
    fn skip_indented_block(&self, cursor: &mut cursor::Cursor, last_known_newline_position: usize) -> usize {
        let len = cursor.input.len();

        let evaluation_type = cursor.curr();

        let block_indentation_level = cursor.pos.saturating_sub(last_known_newline_position).saturating_sub(1);

        let mut last_newline_position = last_known_newline_position;

        while cursor.pos < len && cursor.curr() != b'\n' {
            cursor.advance();
        }

        if evaluation_type == b'=' && cursor.prev() != b',' {
            return cursor.pos;
        }

        'outer: while cursor.pos < len {
            match cursor.curr() {
                b'\\' => {
                    cursor.advance_twice();
                    continue;
                }

                b'\n' => {
                    last_newline_position = cursor.pos;

                    if evaluation_type == b'=' && cursor.prev() != b',' {
                        break;
                    }

                    cursor.advance();
                    continue;
                }

                x if x.is_ascii_whitespace() => {
                    while cursor.pos < len && cursor.curr().is_ascii_whitespace() {
                        if cursor.curr() == b'\n' {
                            last_newline_position = cursor.pos;

                            if evaluation_type == b'=' && cursor.prev() != b',' {
                                break 'outer;
                            }
                        }

                        cursor.advance();
                    }

                    let indentation = cursor.pos.saturating_sub(last_newline_position).saturating_sub(1);
                    if indentation < block_indentation_level {
                        break;
                    }
                }

                _ => break,
            };

            cursor.advance();
        }

        if last_known_newline_position == last_newline_position {
            return cursor.pos;
        }

        cursor.move_to(last_newline_position);

        last_newline_position
    }
}
