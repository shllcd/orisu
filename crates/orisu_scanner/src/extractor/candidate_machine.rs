use crate::cursor;
use crate::extractor::Span;
use crate::extractor::boundary::{has_valid_boundaries, is_valid_before_boundary};
use crate::extractor::machine::{Machine, MachineState};
use crate::extractor::utility_machine::UtilityMachine;
use crate::extractor::variant_machine::VariantMachine;

/// Extract full candidates including variants and utilities.
#[derive(Debug, Default)]
pub struct CandidateMachine {
    /// Start position of the candidate
    start_pos: usize,

    /// End position of the last variant (if any)
    last_variant_end_pos: Option<usize>,

    utility_machine: UtilityMachine,
    variant_machine: VariantMachine,
}

impl Machine for CandidateMachine {
    #[inline(always)]
    fn reset(&mut self) {
        self.start_pos = 0;
        self.last_variant_end_pos = None;
    }

    #[inline]
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState {
        let len = cursor.input.len();

        while cursor.pos < len {
            if cursor.curr().is_ascii_whitespace() {
                self.reset();
                cursor.advance();
                continue;
            }

            if matches!(cursor.curr(), b':' | b'"' | b'\'' | b'`') {
                self.reset();
                cursor.advance();
                continue;
            }

            if matches!(cursor.curr(), b'<' | b'A'..=b'Z') {
                if let Some(offset) = cursor.input[cursor.pos..].iter().position(|&c| is_valid_before_boundary(&c)) {
                    self.reset();
                    cursor.advance_by(offset + 1);
                } else {
                    return self.restart();
                }

                continue;
            }

            let mut variant_cursor = *cursor;
            let variant_machine_state = self.variant_machine.next(&mut variant_cursor);

            let mut utility_cursor = *cursor;
            let utility_machine_state = self.utility_machine.next(&mut utility_cursor);

            match (variant_machine_state, utility_machine_state) {
                (MachineState::Idle, MachineState::Done(utility_span)) => {
                    cursor.move_to(utility_cursor.pos + 1);

                    let span = match self.last_variant_end_pos {
                        Some(end_pos) => {
                            if end_pos + 1 != utility_span.start {
                                return self.restart();
                            }

                            Span::new(self.start_pos, utility_span.end)
                        }
                        None => utility_span,
                    };

                    if !has_valid_boundaries(&span, cursor.input) {
                        return self.restart();
                    }

                    return self.done_span(span);
                }

                (MachineState::Done(variant_span), MachineState::Done(utility_span)) => {
                    cursor.move_to(variant_cursor.pos + 1);

                    if let Some(end_pos) = self.last_variant_end_pos {
                        if end_pos + 1 != variant_span.start {
                            return self.restart();
                        }
                    } else {
                        let after = cursor.input.get(utility_span.end + 2).unwrap_or(&b'\0');
                        if after.is_ascii_whitespace() || *after == b'\0' {
                            cursor.move_to(utility_cursor.pos + 2);
                            return self.done_span(utility_span);
                        }

                        self.start_pos = variant_span.start;
                    }

                    self.last_variant_end_pos = Some(variant_cursor.pos);
                }

                (MachineState::Done(variant_span), MachineState::Idle) => {
                    cursor.move_to(variant_cursor.pos + 1);

                    if let Some(end_pos) = self.last_variant_end_pos {
                        if end_pos + 1 > variant_span.start {
                            self.reset();
                            return MachineState::Idle;
                        }
                    } else {
                        self.start_pos = variant_span.start;
                    }

                    self.last_variant_end_pos = Some(variant_cursor.pos);
                }

                (MachineState::Idle, MachineState::Idle) => {
                    if variant_cursor.pos > cursor.pos || utility_cursor.pos > cursor.pos {
                        cursor.move_to(variant_cursor.pos.max(utility_cursor.pos));
                    }

                    self.reset();
                    cursor.advance();
                }
            }
        }

        MachineState::Idle
    }
}

impl CandidateMachine {
    #[inline(always)]
    fn done_span(&mut self, span: Span) -> MachineState {
        self.reset();
        MachineState::Done(span)
    }
}
