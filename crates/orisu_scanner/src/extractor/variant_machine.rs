use crate::classify::classify;
use crate::cursor;
use crate::extractor::arbitrary_value_machine::ArbitraryValueMachine;
use crate::extractor::machine::{Machine, MachineState};
use crate::extractor::named_variant_machine::NamedVariantMachine;

#[derive(Debug, Default)]
pub struct VariantMachine {
    arbitrary_value_machine: ArbitraryValueMachine,
    named_variant_machine: NamedVariantMachine,
}

impl Machine for VariantMachine {
    #[inline(always)]
    fn reset(&mut self) {}

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match cursor.curr().into() {
            Class::OpenBracket => {
                let start_pos = cursor.pos;
                match self.arbitrary_value_machine.next(cursor) {
                    MachineState::Idle => self.restart(),
                    MachineState::Done(_) => self.parse_arbitrary_end(start_pos, cursor),
                }
            }

            _ => {
                let start_pos = cursor.pos;
                match self.named_variant_machine.next(cursor) {
                    MachineState::Idle => self.restart(),
                    MachineState::Done(_) => self.done(start_pos, cursor),
                }
            }
        }
    }
}

impl VariantMachine {
    #[inline(always)]
    fn parse_arbitrary_end(&mut self, start_pos: usize, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        match cursor.next().into() {
            Class::Colon => {
                cursor.advance();
                self.done(start_pos, cursor)
            }

            _ => self.restart(),
        }
    }
}

classify! {
    #[derive(Debug)]
    enum Class {
        OpenBracket = b'[',
        Colon = b':',
        _ => Other,
    }
}
