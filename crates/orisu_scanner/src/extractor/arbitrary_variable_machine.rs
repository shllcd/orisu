use crate::classify::classify;
use crate::cursor;
use crate::extractor::bracket_stack::BracketStack;
use crate::extractor::css_variable_machine::CssVariableMachine;
use crate::extractor::machine::{Machine, MachineState};
use crate::extractor::string_machine::StringMachine;
use std::marker::PhantomData;

#[derive(Debug, Default)]
pub struct IdleState;

/// Currently parsing the inside of the arbitrary variable
///
/// ```text
/// (--my-opacity)
///  ^^^^^^^^^^^^
/// ```
#[derive(Debug, Default)]
pub struct ParsingState;

/// Currently parsing the data type of the arbitrary variable
///
/// ```text
/// (length:--my-opacity)
///  ^^^^^^^
/// ```
#[derive(Debug, Default)]
pub struct ParsingDataTypeState;

/// Currently parsing the fallback of the arbitrary variable
///
/// ```text
/// (--my-opacity,50%)
///              ^^^^
/// ```
#[derive(Debug, Default)]
pub struct ParsingFallbackState;

/// Extracts arbitrary variables including the parens.
///
/// E.g.:
///
/// ```text
/// (--my-value)
/// ^^^^^^^^^^^^
///
/// bg-red-500/(--my-opacity)
///            ^^^^^^^^^^^^^^
/// ```
#[derive(Debug, Default)]
pub struct ArbitraryVariableMachine<State = IdleState> {
    /// Start position of the arbitrary variable
    start_pos: usize,

    /// Track brackets to ensure they are balanced
    bracket_stack: BracketStack,

    string_machine: StringMachine,
    css_variable_machine: CssVariableMachine,

    _state: PhantomData<State>,
}

impl<State> ArbitraryVariableMachine<State> {
    #[inline(always)]
    fn transition<NextState>(&self) -> ArbitraryVariableMachine<NextState> {
        ArbitraryVariableMachine {
            start_pos: self.start_pos,
            bracket_stack: Default::default(),
            string_machine: Default::default(),
            css_variable_machine: Default::default(),
            _state: PhantomData,
        }
    }
}

impl Machine for ArbitraryVariableMachine<IdleState> {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline(always)]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match cursor.curr().into() {
            Class::OpenParen => match cursor.next().into() {
                Class::Dash => {
                    self.start_pos = cursor.pos;
                    cursor.advance();
                    self.transition::<ParsingState>().next(cursor)
                }

                Class::AlphaLower => {
                    self.start_pos = cursor.pos;
                    cursor.advance();
                    self.transition::<ParsingDataTypeState>().next(cursor)
                }

                _ => MachineState::Idle,
            },

            _ => MachineState::Idle,
        }
    }
}

impl Machine for ArbitraryVariableMachine<ParsingState> {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline(always)]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match self.css_variable_machine.next(cursor) {
            MachineState::Idle => self.restart(),
            MachineState::Done(_) => match cursor.next().into() {
                Class::Comma => {
                    cursor.advance_twice();
                    self.transition::<ParsingFallbackState>().next(cursor)
                }

                _ => {
                    cursor.advance();

                    match cursor.curr().into() {
                        Class::CloseParen => self.done(self.start_pos, cursor),

                        _ => self.restart(),
                    }
                }
            },
        }
    }
}

impl Machine for ArbitraryVariableMachine<ParsingDataTypeState> {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline(always)]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        let len = cursor.input.len();

        while cursor.pos < len {
            match cursor.curr().into() {
                Class::AlphaLower | Class::Dash => {
                    cursor.advance();
                }

                Class::Colon => match cursor.next().into() {
                    Class::Dash => {
                        cursor.advance();
                        return self.transition::<ParsingState>().next(cursor);
                    }

                    _ => return self.restart(),
                },

                _ => return self.restart(),
            };
        }
        self.restart()
    }
}

impl Machine for ArbitraryVariableMachine<ParsingFallbackState> {
    #[inline(always)]
    fn reset(&mut self) {
        self.bracket_stack.reset();
    }

    #[inline(always)]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        let len = cursor.input.len();
        while cursor.pos < len {
            match cursor.curr().into() {
                Class::Escape => match cursor.next().into() {
                    Class::Whitespace => return self.restart(),

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

                Class::CloseParen => return self.done(self.start_pos, cursor),

                Class::Quote => match self.string_machine.next(cursor) {
                    MachineState::Idle => return self.restart(),
                    MachineState::Done(_) => cursor.advance(),
                },

                Class::Colon if self.bracket_stack.is_empty() => return self.restart(),

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
        AlphaLower = b'a'..=b'z',
        AlphaUpper = b'A'..=b'Z',
        At = b'@',
        Colon = b':',
        Comma = b',',
        Dash = b'-',
        Dot = b'.',
        Dollar = b'$',
        Escape = b'\\',
        End = b'\0',
        Number = b'0'..=b'9',
        OpenBracket = b'[',
        CloseBracket = b']',
        OpenParen = b'(',
        CloseParen = b')',
        OpenCurly = b'{',
        CloseCurly = b'}',
        Quote = b'"' | b'\'' | b'`',
        Underscore = b'_',
        Whitespace = b' ' | b'\t' | b'\n' | b'\r' | b'\x0C',
        _ => Other,
    }
}
