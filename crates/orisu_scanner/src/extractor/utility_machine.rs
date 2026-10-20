use crate::classify::classify;
use crate::cursor;
use crate::extractor::arbitrary_property_machine::ArbitraryPropertyMachine;
use crate::extractor::machine::{Machine, MachineState};
use crate::extractor::modifier_machine::ModifierMachine;
use crate::extractor::named_utility_machine::NamedUtilityMachine;

#[derive(Debug, Default)]
pub struct UtilityMachine {
    /// Start position of the utility
    start_pos: usize,

    /// Whether the legacy important marker `!` was used
    legacy_important: bool,

    arbitrary_property_machine: ArbitraryPropertyMachine,
    named_utility_machine: NamedUtilityMachine,
    modifier_machine: ModifierMachine,
}

impl Machine for UtilityMachine {
    #[inline(always)]
    fn reset(&mut self) {
        self.start_pos = 0;
        self.legacy_important = false;
    }

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match cursor.curr().into() {
            Class::Exclamation => {
                self.legacy_important = true;

                match cursor.next().into() {
                    Class::OpenBracket => {
                        self.start_pos = cursor.pos;
                        cursor.advance();
                        self.parse_arbitrary_property(cursor)
                    }

                    _ => {
                        self.start_pos = cursor.pos;
                        cursor.advance();
                        self.parse_named_utility(cursor)
                    }
                }
            }

            Class::OpenBracket => {
                self.start_pos = cursor.pos;
                self.parse_arbitrary_property(cursor)
            }

            _ => {
                self.start_pos = cursor.pos;
                self.parse_named_utility(cursor)
            }
        }
    }
}

impl UtilityMachine {
    fn parse_arbitrary_property(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match self.arbitrary_property_machine.next(cursor) {
            MachineState::Idle => self.restart(),
            MachineState::Done(_) => match cursor.next().into() {
                Class::Slash => {
                    cursor.advance();
                    self.parse_modifier(cursor)
                }

                Class::Exclamation => {
                    cursor.advance();
                    self.parse_important(cursor)
                }

                _ => self.done(self.start_pos, cursor),
            },
        }
    }

    fn parse_named_utility(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match self.named_utility_machine.next(cursor) {
            MachineState::Idle => self.restart(),
            MachineState::Done(_) => match cursor.next().into() {
                Class::Slash => {
                    cursor.advance();
                    self.parse_modifier(cursor)
                }

                Class::Exclamation => {
                    cursor.advance();
                    self.parse_important(cursor)
                }

                _ => self.done(self.start_pos, cursor),
            },
        }
    }

    fn parse_modifier(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match self.modifier_machine.next(cursor) {
            MachineState::Idle => self.restart(),
            MachineState::Done(_) => match cursor.next().into() {
                Class::Slash => self.restart(),

                Class::Exclamation => {
                    cursor.advance();
                    self.parse_important(cursor)
                }

                _ => self.done(self.start_pos, cursor),
            },
        }
    }

    fn parse_important(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        if self.legacy_important {
            return self.restart();
        }

        self.done(self.start_pos, cursor)
    }
}

classify! {
    #[derive(Debug)]
    enum Class {
        Exclamation = b'!',
        OpenBracket = b'[',
        Slash = b'/',
        _ => Other,
    }
}
