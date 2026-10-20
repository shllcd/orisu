use crate::classify::classify;
use crate::cursor;
use crate::extractor::arbitrary_value_machine::ArbitraryValueMachine;
use crate::extractor::arbitrary_variable_machine::ArbitraryVariableMachine;
use crate::extractor::machine::{Machine, MachineState};
use crate::extractor::modifier_machine::ModifierMachine;
use std::marker::PhantomData;

#[derive(Debug, Default)]
pub struct IdleState;

/// Parsing a variant
#[derive(Debug, Default)]
pub struct ParsingState;

/// Parsing a modifier
///
/// E.g.:
///
/// ```text
/// group-hover/name:
///            ^^^^^
/// ```
///
#[derive(Debug, Default)]
pub struct ParsingModifierState;

/// Parsing the end of a variant
///
/// E.g.:
///
/// ```text
/// hover:
///      ^
/// ```
#[derive(Debug, Default)]
pub struct ParsingEndState;

/// Extract named variants from an input including the `:`.
///
/// E.g.:
///
/// ```text
/// hover:flex
/// ^^^^^^
///
/// data-[state=pending]:flex
/// ^^^^^^^^^^^^^^^^^^^^^
///
/// supports-(--my-variable):flex
/// ^^^^^^^^^^^^^^^^^^^^^^^^^
/// ```
#[derive(Debug, Default)]
pub struct NamedVariantMachine<State = IdleState> {
    /// Start position of the variant
    start_pos: usize,

    arbitrary_variable_machine: ArbitraryVariableMachine,
    arbitrary_value_machine: ArbitraryValueMachine,
    modifier_machine: ModifierMachine,

    _state: PhantomData<State>,
}

impl<State> NamedVariantMachine<State> {
    #[inline(always)]
    fn transition<NextState>(&self) -> NamedVariantMachine<NextState> {
        NamedVariantMachine {
            start_pos: self.start_pos,
            arbitrary_variable_machine: Default::default(),
            arbitrary_value_machine: Default::default(),
            modifier_machine: Default::default(),
            _state: PhantomData,
        }
    }
}

impl Machine for NamedVariantMachine<IdleState> {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline(always)]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match cursor.curr().into() {
            Class::AlphaLower | Class::Star => match cursor.next().into() {
                Class::Colon => {
                    cursor.advance();
                    self.transition::<ParsingEndState>().next(cursor)
                }

                _ => {
                    self.start_pos = cursor.pos;
                    cursor.advance();
                    self.transition::<ParsingState>().next(cursor)
                }
            },

            Class::Number | Class::At => {
                self.start_pos = cursor.pos;
                cursor.advance();
                self.transition::<ParsingState>().next(cursor)
            }

            _ => MachineState::Idle,
        }
    }
}

impl Machine for NamedVariantMachine<ParsingState> {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline(always)]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        let len = cursor.input.len();

        while cursor.pos < len {
            match cursor.curr().into() {
                Class::Dash => match cursor.next().into() {
                    Class::OpenBracket => {
                        cursor.advance();
                        return match self.arbitrary_value_machine.next(cursor) {
                            MachineState::Idle => self.restart(),
                            MachineState::Done(_) => self.parse_arbitrary_end(cursor),
                        };
                    }

                    Class::OpenParen => {
                        cursor.advance();
                        return match self.arbitrary_variable_machine.next(cursor) {
                            MachineState::Idle => self.restart(),
                            MachineState::Done(_) => self.parse_arbitrary_end(cursor),
                        };
                    }

                    Class::Dash | Class::Underscore | Class::AlphaLower | Class::AlphaUpper | Class::Number => cursor.advance(),

                    _ => return self.restart(),
                },

                Class::OpenBracket => {
                    return match self.arbitrary_value_machine.next(cursor) {
                        MachineState::Idle => self.restart(),
                        MachineState::Done(_) => self.parse_arbitrary_end(cursor),
                    };
                }

                Class::Underscore => match cursor.next().into() {
                    Class::Dash | Class::Underscore | Class::AlphaLower | Class::AlphaUpper | Class::Number => cursor.advance(),

                    _ => return self.restart(),
                },

                Class::AlphaLower | Class::AlphaUpper | Class::Number | Class::Star => cursor.advance(),

                Class::Slash => return self.transition::<ParsingModifierState>().next(cursor),

                Class::Colon => return self.done(self.start_pos, cursor),

                Class::Dot => {
                    if !matches!(cursor.prev().into(), Class::Number) {
                        return self.restart();
                    }

                    if !matches!(cursor.next().into(), Class::Number) {
                        return self.restart();
                    }

                    cursor.advance();
                }

                _ => return self.restart(),
            };
        }

        self.restart()
    }
}

impl NamedVariantMachine<ParsingState> {
    #[inline(always)]
    fn parse_arbitrary_end(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match cursor.next().into() {
            Class::Slash => {
                cursor.advance();
                self.transition::<ParsingModifierState>().next(cursor)
            }
            Class::Colon => {
                cursor.advance();
                self.transition::<ParsingEndState>().next(cursor)
            }
            _ => self.restart(),
        }
    }
}

impl Machine for NamedVariantMachine<ParsingModifierState> {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline(always)]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match self.modifier_machine.next(cursor) {
            MachineState::Idle => self.restart(),
            MachineState::Done(_) => match cursor.next().into() {
                Class::Colon => {
                    cursor.advance();
                    self.transition::<ParsingEndState>().next(cursor)
                }

                _ => self.restart(),
            },
        }
    }
}

impl Machine for NamedVariantMachine<ParsingEndState> {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline(always)]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match cursor.curr().into() {
            Class::Colon => self.done(self.start_pos, cursor),

            _ => self.restart(),
        }
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
        Number = b'0'..=b'9',
        OpenBracket = b'[',
        OpenParen = b'(',
        Star = b'*',
        Slash = b'/',
        Underscore = b'_',
        _ => Other,
    }
}
