use crate::classify::classify;
use crate::cursor;
use crate::extractor::CssVariableMachine;
use crate::extractor::bracket_stack::BracketStack;
use crate::extractor::machine::{Machine, MachineState};
use crate::extractor::string_machine::StringMachine;
use std::marker::PhantomData;

#[derive(Debug, Default)]
pub struct IdleState;

/// Parsing the property, e.g.:
///
/// ```text
/// [color:red]
///  ^^^^^
///
/// [--my-color:red]
///  ^^^^^^^^^^
/// ```
#[derive(Debug, Default)]
pub struct ParsingPropertyState;

/// Parsing the value, e.g.:
///
/// ```text
/// [color:red]
///        ^^^
/// ```
#[derive(Debug, Default)]
pub struct ParsingValueState;

/// Extracts arbitrary properties from the input, including the brackets.
///
/// E.g.:
///
/// ```text
/// [color:red]
/// ^^^^^^^^^^^
///
/// [--my-color:red]
/// ^^^^^^^^^^^^^^^^
/// ```
#[derive(Debug, Default)]
pub struct ArbitraryPropertyMachine<State = IdleState> {
    /// Start position of the arbitrary value
    start_pos: usize,

    /// Track brackets to ensure they are balanced
    bracket_stack: BracketStack,

    css_variable_machine: CssVariableMachine,
    string_machine: StringMachine,

    _state: PhantomData<State>,
}

impl<State> ArbitraryPropertyMachine<State> {
    #[inline(always)]
    fn transition<NextState>(&self) -> ArbitraryPropertyMachine<NextState> {
        ArbitraryPropertyMachine {
            start_pos: self.start_pos,
            bracket_stack: Default::default(),
            css_variable_machine: Default::default(),
            string_machine: Default::default(),
            _state: PhantomData,
        }
    }
}

impl Machine for ArbitraryPropertyMachine<IdleState> {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match cursor.curr().into() {
            Class::OpenBracket => {
                self.start_pos = cursor.pos;
                cursor.advance();
                self.transition::<ParsingPropertyState>().next(cursor)
            }

            _ => MachineState::Idle,
        }
    }
}

impl Machine for ArbitraryPropertyMachine<ParsingPropertyState> {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        let len = cursor.input.len();

        while cursor.pos < len {
            match cursor.curr().into() {
                Class::Dash => match cursor.next().into() {
                    Class::Dash => return self.parse_property_variable(cursor),

                    _ => cursor.advance(),
                },

                Class::AlphaLower => cursor.advance(),

                Class::Colon if cursor.pos > self.start_pos + 1 => {
                    cursor.advance();
                    return self.transition::<ParsingValueState>().next(cursor);
                }

                _ => return self.restart(),
            }
        }

        self.restart()
    }
}

impl ArbitraryPropertyMachine<ParsingPropertyState> {
    fn parse_property_variable(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match self.css_variable_machine.next(cursor) {
            MachineState::Idle => self.restart(),
            MachineState::Done(_) => match cursor.next().into() {
                Class::Colon => {
                    cursor.advance_twice();
                    self.transition::<ParsingValueState>().next(cursor)
                }

                _ => self.restart(),
            },
        }
    }
}

impl Machine for ArbitraryPropertyMachine<ParsingValueState> {
    #[inline(always)]
    fn reset(&mut self) {
        self.bracket_stack.reset();
    }

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        let len = cursor.input.len();
        let start_of_value_pos = cursor.pos;
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

                Class::CloseBracket if self.start_pos + 1 != cursor.pos && self.bracket_stack.is_empty() => {
                    return self.done(self.start_pos, cursor);
                }

                Class::Quote => return self.parse_string(cursor),

                Class::Colon if self.bracket_stack.is_empty() => return self.restart(),

                Class::Whitespace => return self.restart(),

                Class::Slash if start_of_value_pos == cursor.pos => return self.restart(),

                Class::Dollar if matches!(cursor.next().into(), Class::OpenCurly) => {
                    return self.restart();
                }

                Class::Exclamation if self.bracket_stack.is_empty() => {
                    return self.restart();
                }

                _ => cursor.advance(),
            };
        }

        self.restart()
    }
}

impl ArbitraryPropertyMachine<ParsingValueState> {
    fn parse_string(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match self.string_machine.next(cursor) {
            MachineState::Idle => self.restart(),
            MachineState::Done(_) => {
                cursor.advance();
                self.next(cursor)
            }
        }
    }
}

classify! {
    enum Class {
        OpenParen = b'(',
        OpenBracket = b'[',
        OpenCurly = b'{',
        CloseParen = b')',
        CloseBracket = b']',
        CloseCurly = b'}',
        Escape = b'\\',
        Quote = b'"' | b'\'' | b'`',
        Dash = b'-',
        Dollar = b'$',
        AlphaLower = b'a'..=b'z',
        Colon = b':',
        Slash = b'/',
        Exclamation = b'!',
        Whitespace = b' ' | b'\t' | b'\n' | b'\r' | b'\x0C',
        End = b'\0',
        _ => Other,
    }
}
