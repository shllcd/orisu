use crate::cursor;
use crate::extractor::pre_processors::pre_processor::PreProcessor;

#[derive(Debug, Default)]
pub struct Twig;

impl PreProcessor for Twig {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        let len = content.len();
        let mut result = content.to_vec();
        let mut cursor = cursor::Cursor::new(content);
        let mut bracket_stack = vec![];

        const ADD_CLASS: &[u8] = b"addClass";
        const REMOVE_CLASS: &[u8] = b"removeClass";

        while cursor.pos < len {
            match (!bracket_stack.is_empty(), cursor.curr()) {
                (false, b'(')
                    if cursor.pos >= ADD_CLASS.len() && matches!(&content[cursor.pos - ADD_CLASS.len()..cursor.pos], ADD_CLASS) =>
                {
                    bracket_stack.push(cursor.curr());
                    result[cursor.pos] = b' ';
                }

                (false, b'(')
                    if cursor.pos >= REMOVE_CLASS.len()
                        && matches!(&content[cursor.pos - REMOVE_CLASS.len()..cursor.pos], REMOVE_CLASS) =>
                {
                    bracket_stack.push(cursor.curr());
                    result[cursor.pos] = b' ';
                }

                (true, b'(') => {
                    bracket_stack.push(cursor.curr());
                }

                (true, b')') => {
                    bracket_stack.pop();

                    if bracket_stack.is_empty() {
                        result[cursor.pos] = b' ';
                    }
                }

                _ => {}
            }

            cursor.advance();
        }

        result
    }
}
