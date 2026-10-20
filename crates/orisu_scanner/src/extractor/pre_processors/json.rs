use crate::cursor;
use crate::extractor::pre_processors::pre_processor::PreProcessor;

#[derive(Debug, Default)]
pub struct Json;

impl PreProcessor for Json {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        let len = content.len();
        let mut result = content.to_vec();
        let mut cursor = cursor::Cursor::new(content);

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
                }

                b'[' | b'{' | b']' | b'}' => {
                    result[cursor.pos] = b' ';
                }

                _ => {}
            };

            cursor.advance();
        }

        result
    }
}
