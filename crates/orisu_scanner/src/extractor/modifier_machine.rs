use crate::classify::classify;
use crate::cursor;
use crate::extractor::arbitrary_value_machine::ArbitraryValueMachine;
use crate::extractor::arbitrary_variable_machine::ArbitraryVariableMachine;
use crate::extractor::machine::{Machine, MachineState};

/// Extract modifiers from an input including the `/`.
///
/// E.g.:
///
/// ```text
/// bg-red-500/20
///           ^^^
///
/// bg-red-500/[20%]
///           ^^^^^^
///
/// bg-red-500/(--my-opacity)
///           ^^^^^^^^^^^^^^^
/// ```
#[derive(Debug, Default)]
pub struct ModifierMachine {
    arbitrary_value_machine: ArbitraryValueMachine,
    arbitrary_variable_machine: ArbitraryVariableMachine,
}

impl Machine for ModifierMachine {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        if Class::Slash != cursor.curr().into() {
            return MachineState::Idle;
        }

        let start_pos = cursor.pos;
        cursor.advance();

        match cursor.curr().into() {
            Class::OpenBracket => match self.arbitrary_value_machine.next(cursor) {
                MachineState::Idle => self.restart(),
                MachineState::Done(_) => self.done(start_pos, cursor),
            },

            Class::OpenParen => match self.arbitrary_variable_machine.next(cursor) {
                MachineState::Idle => self.restart(),
                MachineState::Done(_) => self.done(start_pos, cursor),
            },

            Class::ValidStart => {
                let len = cursor.input.len();
                while cursor.pos < len {
                    match cursor.curr().into() {
                        Class::ValidStart | Class::ValidInside => match cursor.next().into() {
                            Class::ValidStart | Class::ValidInside => cursor.advance(),

                            _ => return self.done(start_pos, cursor),
                        },

                        _ => return self.restart(),
                    }
                }

                MachineState::Idle
            }

            _ => MachineState::Idle,
        }
    }
}

classify! {
    #[derive(Debug, PartialEq)]
    enum Class {
        ValidStart = b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9',
        ValidInside = b'-' | b'_' | b'.',
        OpenBracket = b'[',
        OpenParen = b'(',
        Slash = b'/',
        _ => Other,
    }
}
