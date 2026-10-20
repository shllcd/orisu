use crate::classify::classify;
use crate::cursor;
use crate::extractor::bracket_stack::BracketStack;
use crate::extractor::machine::{Machine, MachineState};
use crate::extractor::string_machine::StringMachine;

/// Extracts arbitrary values including the brackets.
///
/// E.g.:
///
/// ```text
/// bg-[#0088cc]
///    ^^^^^^^^^
///
/// bg-red-500/[20%]
///            ^^^^^
/// ```
#[derive(Debug, Default)]
pub struct ArbitraryValueMachine {
    /// Track brackets to ensure they are balanced
    bracket_stack: BracketStack,

    string_machine: StringMachine,
}

impl Machine for ArbitraryValueMachine {
    #[inline(always)]
    fn reset(&mut self) {
        self.bracket_stack.reset();
    }

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        if Class::OpenBracket != cursor.curr().into() {
            return MachineState::Idle;
        }

        let start_pos = cursor.pos;
        cursor.advance();

        let len = cursor.input.len();

        while cursor.pos < len {
            match cursor.curr().into() {
                Class::Escape => match cursor.next().into() {
                    Class::Whitespace => {
                        cursor.advance_twice();
                        return self.restart();
                    }

                    _ => cursor.advance_twice(),
                },

                Class::OpenParen | Class::OpenBracket | Class::OpenCurly => {
                    if !self.bracket_stack.push(cursor.curr()) {
                        return self.restart();
                    }
                    cursor.advance();
                }

                Class::CloseParen | Class::CloseBracket | Class::CloseCurly if !self.bracket_stack.is_empty() => {
                    if !self.bracket_stack.pop(cursor.curr()) {
                        return self.restart();
                    }
                    cursor.advance();
                }

                Class::CloseBracket if start_pos + 1 != cursor.pos && self.bracket_stack.is_empty() => {
                    return self.done(start_pos, cursor);
                }

                Class::Quote => match self.string_machine.next(cursor) {
                    MachineState::Idle => return self.restart(),
                    MachineState::Done(_) => cursor.advance(),
                },

                Class::Whitespace => return self.restart(),

                Class::Dollar if matches!(cursor.next().into(), Class::OpenCurly) => {
                    return self.restart();
                }

                _ => cursor.advance(),
            };
        }

        self.restart()
    }
}

classify! {
    #[derive(PartialEq)]
    enum Class {
        Escape = b'\\',
        OpenParen = b'(',
        CloseParen = b')',
        OpenBracket = b'[',
        CloseBracket = b']',
        OpenCurly = b'{',
        CloseCurly = b'}',
        Quote = b'"' | b'\'' | b'`',
        Whitespace = b' ' | b'\t' | b'\n' | b'\r' | b'\x0C',
        Dollar = b'$',
        _ => Other,
    }
}
