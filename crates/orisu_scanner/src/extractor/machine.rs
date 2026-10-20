use crate::cursor;

#[derive(Debug, Clone, Copy)]
pub struct Span {
    /// Inclusive start position of the span
    pub start: usize,

    /// Inclusive end position of the span
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    #[inline(always)]
    pub fn slice<'a>(&self, input: &'a [u8]) -> &'a [u8] {
        &input[self.start..=self.end]
    }
}

#[derive(Debug, Default)]
pub enum MachineState {
    /// Machine is not doing anything at the moment
    #[default]
    Idle,

    /// Machine is done parsing and has extracted a span
    Done(Span),
}

pub trait Machine: Sized + Default {
    fn reset(&mut self);
    fn next(&mut self, cursor: &mut cursor::Cursor<'_>) -> MachineState;

    /// Reset the state machine, and mark the machine as [MachineState::Idle].
    #[inline(always)]
    fn restart(&mut self) -> MachineState {
        self.reset();
        MachineState::Idle
    }

    /// Reset the state machine, and mark the machine as [MachineState::Done(…)].
    #[inline(always)]
    fn done(&mut self, start: usize, cursor: &cursor::Cursor<'_>) -> MachineState {
        self.reset();
        MachineState::Done(Span::new(start, cursor.pos))
    }
}
