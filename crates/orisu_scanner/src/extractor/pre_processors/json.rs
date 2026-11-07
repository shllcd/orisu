use crate::cursor;

pub fn process(content: &[u8]) -> Vec<u8> {
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
