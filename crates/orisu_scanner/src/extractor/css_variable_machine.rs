use crate::classify::classify;
use crate::cursor;
use crate::extractor::machine::{Machine, MachineState};

/// Extract CSS variables from an input.
///
/// E.g.:
///
/// ```text
/// var(--my-variable)
///     ^^^^^^^^^^^^^
/// ```
#[derive(Debug, Default)]
pub struct CssVariableMachine;

impl Machine for CssVariableMachine {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        if Class::Dash != cursor.curr().into() || Class::Dash != cursor.next().into() {
            return MachineState::Idle;
        }

        let start_pos = cursor.pos;
        let len = cursor.input.len();

        cursor.advance_twice();

        while cursor.pos < len {
            match cursor.curr().into() {
                Class::AllowedCharacter | Class::Dash => match cursor.next().into() {
                    Class::AllowedCharacter | Class::Dash | Class::Escape => cursor.advance(),

                    _ => {
                        if cursor.pos - start_pos < 2 {
                            return self.restart();
                        } else {
                            return self.done(start_pos, cursor);
                        }
                    }
                },

                Class::Escape => match cursor.next().into() {
                    Class::Whitespace => return self.restart(),

                    Class::End => return self.restart(),

                    _ => cursor.advance_twice(),
                },

                _ => return self.restart(),
            }
        }

        MachineState::Idle
    }
}

classify! {
    #[derive(PartialEq)]
    enum Class {
        Dash = b'-',
        AllowedCharacter = b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | 0x80..=0xff,
        Escape = b'\\',
        Whitespace = b' ' | b'\t' | b'\n' | b'\r' | b'\x0C',
        End = b'\0',
        _ => Other,
    }
}
