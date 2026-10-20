use crate::classify::classify;
use crate::cursor;
use crate::extractor::machine::{Machine, MachineState};

/// Extracts a string (including the quotes) from the input.
///
/// Rules:
///
/// - The string must start and end with the same quote character.
/// - The string cannot contain any whitespace characters.
/// - The string can contain any other character except for the quote character (unless it's escaped).
/// - Balancing of brackets is not required.
///
///
/// E.g.:
///
/// ```text
/// 'hello_world'
/// ^^^^^^^^^^^^^
///
/// content-['hello_world']
///          ^^^^^^^^^^^^^
/// ```
#[derive(Debug, Default)]
pub struct StringMachine;

impl Machine for StringMachine {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        if Class::Quote != cursor.curr().into() {
            return MachineState::Idle;
        }

        let len = cursor.input.len();
        let start_pos = cursor.pos;
        let end_char = cursor.curr();

        cursor.advance();

        while cursor.pos < len {
            match cursor.curr().into() {
                Class::Escape => match cursor.next().into() {
                    Class::Whitespace => return MachineState::Idle,

                    _ => cursor.advance(),
                },

                Class::Quote if cursor.curr() == end_char => return self.done(start_pos, cursor),

                Class::Whitespace => return MachineState::Idle,

                _ => {}
            };

            cursor.advance()
        }

        MachineState::Idle
    }
}

classify! {
    #[derive(Debug, PartialEq, Eq)]
    enum Class {
        Quote = b'"' | b'\'' | b'`',
        Escape = b'\\',
        Whitespace = b' ' | b'\t' | b'\n' | b'\r' | b'\x0C',
        _ => Other,
    }
}
