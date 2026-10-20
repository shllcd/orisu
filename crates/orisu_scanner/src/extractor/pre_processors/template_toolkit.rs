use crate::cursor;
use crate::extractor::pre_processors::pre_processor::PreProcessor;

#[derive(Debug, Default)]
pub struct TemplateToolkit;

impl PreProcessor for TemplateToolkit {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        let len = content.len();
        let mut result = content.to_vec();
        let mut cursor = cursor::Cursor::new(content);

        while cursor.pos < len {
            match (cursor.curr(), cursor.next()) {
                (b'[', b'%') => result[cursor.pos] = b' ',
                (b'%', b']') => result[cursor.pos + 1] = b' ',
                _ => {}
            }

            cursor.advance();
        }

        result
    }
}
