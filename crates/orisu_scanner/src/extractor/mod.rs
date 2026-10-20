use crate::cursor;
use crate::extractor::machine::Span;
use candidate_machine::CandidateMachine;
use css_variable_machine::CssVariableMachine;
use machine::{Machine, MachineState};
use std::fmt;

pub mod arbitrary_property_machine;
pub mod arbitrary_value_machine;
pub mod arbitrary_variable_machine;
mod boundary;
pub mod bracket_stack;
pub mod candidate_machine;
pub mod css_variable_machine;
pub mod machine;
pub mod modifier_machine;
pub mod named_utility_machine;
pub mod named_variant_machine;
pub mod pre_processors;
pub mod string_machine;
pub mod utility_machine;
pub mod variant_machine;

#[derive(Debug)]
pub enum Extracted<'a> {
    /// Extracted a valid looking candidate
    ///
    /// E.g.: `flex`
    ///
    Candidate(&'a [u8]),

    /// Extracted a valid looking CSS variable
    ///
    /// E.g.: `--my-variable`
    ///
    CssVariable(&'a [u8]),
}

impl fmt::Display for Extracted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Extracted::Candidate(candidate) => {
                write!(f, "Candidate({})", std::str::from_utf8(candidate).unwrap())
            }
            Extracted::CssVariable(candidate) => {
                write!(f, "CssVariable({})", std::str::from_utf8(candidate).unwrap())
            }
        }
    }
}

#[derive(Debug)]
pub struct Extractor<'a> {
    cursor: cursor::Cursor<'a>,

    css_variable_machine: CssVariableMachine,
    candidate_machine: CandidateMachine,
}

impl<'a> Extractor<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self { cursor: cursor::Cursor::new(input), css_variable_machine: Default::default(), candidate_machine: Default::default() }
    }

    pub fn extract(&mut self) -> Vec<Extracted<'a>> {
        let mut in_flight_spans: Vec<Span> = Vec::with_capacity(15);

        let mut extracted = Vec::with_capacity(100);

        let len = self.cursor.input.len();

        {
            let cursor = &mut self.cursor.clone();
            while cursor.pos < len {
                if cursor.curr().is_ascii_whitespace() {
                    cursor.advance();
                    continue;
                }

                if let MachineState::Done(span) = self.css_variable_machine.next(cursor) {
                    extracted.push(Extracted::CssVariable(span.slice(self.cursor.input)));
                }

                cursor.advance();
            }
        }

        {
            let cursor = &mut self.cursor.clone();

            while cursor.pos < len {
                if cursor.curr().is_ascii_whitespace() {
                    cursor.advance();
                    continue;
                }

                let before = cursor.pos;
                match self.candidate_machine.next(cursor) {
                    MachineState::Done(span) => {
                        in_flight_spans.push(span);
                        extract_sub_candidates(before..span.start, cursor, &mut in_flight_spans);
                    }
                    MachineState::Idle => {
                        extract_sub_candidates(before..cursor.pos.min(cursor.input.len()), cursor, &mut in_flight_spans);
                    }
                }

                cursor.advance();
            }

            if !in_flight_spans.is_empty() {
                extracted
                    .extend(drop_covered_spans(in_flight_spans).iter().map(|span| Extracted::Candidate(span.slice(self.cursor.input))));
            }
        }

        extracted
    }

    pub fn extract_variables_from_css(&mut self) -> Vec<Extracted<'a>> {
        let mut extracted = Vec::with_capacity(100);

        let len = self.cursor.input.len();

        let cursor = &mut self.cursor.clone();
        while cursor.pos < len {
            if cursor.curr().is_ascii_whitespace() {
                cursor.advance();
                continue;
            }

            if let MachineState::Done(span) = self.css_variable_machine.next(cursor) {
                if span.start < 4 {
                    cursor.advance();
                    continue;
                }

                let slice_before = Span::new(span.start - 4, span.start - 1);
                if !slice_before.slice(self.cursor.input).starts_with(b"var(") {
                    cursor.advance();
                    continue;
                }

                extracted.push(Extracted::CssVariable(span.slice(self.cursor.input)));
            }

            cursor.advance();
        }

        extracted
    }
}

#[inline(always)]
fn extract_sub_candidates(range: std::ops::Range<usize>, cursor: &cursor::Cursor<'_>, in_flight_spans: &mut Vec<Span>) {
    let end = range.end;
    for i in range {
        if cursor.input[i] == b'[' {
            let mut cursor = *cursor;
            cursor.move_to(i + 1);

            let mut machine = CandidateMachine::default();

            while cursor.pos < end {
                if let MachineState::Done(span) = machine.next(&mut cursor) {
                    in_flight_spans.push(span);
                }

                cursor.advance();
            }
        }
    }
}

fn drop_covered_spans(mut spans: Vec<Span>) -> Vec<Span> {
    if spans.len() <= 1 {
        return spans;
    }

    spans.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));

    let mut result = Vec::with_capacity(spans.len());
    let mut max_end = None;

    for span in spans {
        if max_end.is_none_or(|end| span.end > end) {
            result.push(span);
            max_end = Some(span.end);
        }
    }

    result
}
