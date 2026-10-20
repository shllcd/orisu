use crate::classify::classify;
use crate::cursor;
use crate::extractor::arbitrary_value_machine::ArbitraryValueMachine;
use crate::extractor::arbitrary_variable_machine::ArbitraryVariableMachine;
use crate::extractor::boundary::is_valid_after_boundary;
use crate::extractor::machine::{Machine, MachineState};
use std::marker::PhantomData;

#[derive(Debug, Default)]
pub struct IdleState;

#[derive(Debug, Default)]
pub struct ParsingState;

/// Extracts named utilities from an input.
///
/// E.g.:
///
/// ```text
/// flex
/// ^^^^
///
/// bg-red-500
/// ^^^^^^^^^^
/// ```
#[derive(Debug, Default)]
pub struct NamedUtilityMachine<State = IdleState> {
    /// Start position of the utility
    start_pos: usize,

    arbitrary_variable_machine: ArbitraryVariableMachine,
    arbitrary_value_machine: ArbitraryValueMachine,

    _state: PhantomData<State>,
}

impl<State> NamedUtilityMachine<State> {
    #[inline(always)]
    fn transition<NextState>(&self) -> NamedUtilityMachine<NextState> {
        NamedUtilityMachine {
            start_pos: self.start_pos,
            arbitrary_variable_machine: Default::default(),
            arbitrary_value_machine: Default::default(),
            _state: PhantomData,
        }
    }
}

impl Machine for NamedUtilityMachine<IdleState> {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match cursor.curr().into() {
            Class::AlphaLower => match cursor.next().into() {
                Class::Whitespace | Class::Quote | Class::End => self.done(cursor.pos, cursor),

                _ => {
                    self.start_pos = cursor.pos;
                    cursor.advance();
                    self.transition::<ParsingState>().next(cursor)
                }
            },

            Class::At => {
                self.start_pos = cursor.pos;
                cursor.advance();
                self.transition::<ParsingState>().next(cursor)
            }

            Class::Dash => match cursor.next().into() {
                Class::AlphaLower => {
                    self.start_pos = cursor.pos;
                    cursor.advance();
                    self.transition::<ParsingState>().next(cursor)
                }

                _ => MachineState::Idle,
            },

            _ => MachineState::Idle,
        }
    }
}

impl Machine for NamedUtilityMachine<ParsingState> {
    #[inline(always)]
    fn reset(&mut self) {
        self.start_pos = 0;
    }

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        let len = cursor.input.len();

        while cursor.pos < len {
            match cursor.curr().into() {
                Class::AlphaLower | Class::AlphaUpper => {
                    if is_valid_after_boundary(&cursor.next()) || {
                        matches!(cursor.next().into(), Class::Colon | Class::Slash | Class::Exclamation)
                    } {
                        return self.done(self.start_pos, cursor);
                    }

                    cursor.advance()
                }

                Class::Dash => match cursor.next().into() {
                    Class::OpenBracket => {
                        cursor.advance();
                        return match self.arbitrary_value_machine.next(cursor) {
                            MachineState::Idle => self.restart(),
                            MachineState::Done(_) => self.done(self.start_pos, cursor),
                        };
                    }

                    Class::OpenParen => {
                        cursor.advance();
                        return match self.arbitrary_variable_machine.next(cursor) {
                            MachineState::Idle => self.restart(),
                            MachineState::Done(_) => self.done(self.start_pos, cursor),
                        };
                    }

                    Class::AlphaLower | Class::AlphaUpper | Class::Number | Class::Dash => {
                        cursor.advance();
                    }

                    _ => return self.restart(),
                },

                Class::Underscore => match cursor.next().into() {
                    Class::AlphaLower | Class::AlphaUpper | Class::Number | Class::Underscore => {
                        cursor.advance();
                    }

                    _ if is_valid_after_boundary(&cursor.next()) || {
                        matches!(cursor.next().into(), Class::Colon | Class::Slash | Class::Exclamation)
                    } =>
                    {
                        return self.done(self.start_pos, cursor);
                    }

                    _ => return self.restart(),
                },

                Class::Dot => {
                    if !matches!(cursor.prev().into(), Class::Number) {
                        return self.restart();
                    }

                    if !matches!(cursor.next().into(), Class::Number) {
                        return self.restart();
                    }

                    cursor.advance();
                }

                Class::Number => {
                    if !matches!(
                        cursor.prev().into(),
                        Class::Dash | Class::Underscore | Class::Dot | Class::Number | Class::AlphaLower | Class::AlphaUpper
                    ) {
                        return self.restart();
                    }

                    if !matches!(
                        cursor.next().into(),
                        Class::Dot
                            | Class::Number
                            | Class::AlphaLower
                            | Class::AlphaUpper
                            | Class::Percent
                            | Class::Underscore
                            | Class::Dash
                    ) {
                        return self.done(self.start_pos, cursor);
                    }

                    cursor.advance();
                }

                Class::Percent => {
                    if !matches!(cursor.prev().into(), Class::Number) {
                        return self.restart();
                    }

                    return self.done(self.start_pos, cursor);
                }

                _ => return self.restart(),
            };
        }

        self.restart()
    }
}

classify! {
    enum Class {
        AlphaLower = b'a'..=b'z',
        AlphaUpper = b'A'..=b'Z',
        At = b'@',
        Colon = b':',
        Dash = b'-',
        Dot = b'.',
        End = b'\0',
        Exclamation = b'!',
        Number = b'0'..=b'9',
        OpenBracket = b'[',
        CloseBracket = b']',
        OpenParen = b'(',
        Percent = b'%',
        Quote = b'"' | b'\'' | b'`',
        Slash = b'/',
        Underscore = b'_',
        Whitespace = b' ' | b'\t' | b'\n' | b'\r' | b'\x0C',
        _ => Other,
    }
}
