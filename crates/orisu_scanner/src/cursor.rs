#[derive(Debug, Clone, Copy)]
pub struct Cursor<'a> {
    pub input: &'a [u8],

    pub pos: usize,
}

impl<'a> Cursor<'a> {
    #[inline(always)]
    pub fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    /// The current byte at `pos`, or 0x00 if past the end.
    #[inline(always)]
    pub fn curr(&self) -> u8 {
        if self.pos < self.input.len() { unsafe { *self.input.get_unchecked(self.pos) } } else { 0x00 }
    }

    /// The next byte at `pos + 1`, or 0x00 if past the end.
    #[inline(always)]
    pub fn next(&self) -> u8 {
        let next_pos = self.pos + 1;
        if next_pos < self.input.len() { unsafe { *self.input.get_unchecked(next_pos) } } else { 0x00 }
    }

    /// The previous byte at `pos - 1`, or 0x00 if at the start.
    #[inline(always)]
    pub fn prev(&self) -> u8 {
        if self.pos > 0 { unsafe { *self.input.get_unchecked(self.pos - 1) } } else { 0x00 }
    }

    pub fn advance_by(&mut self, amount: usize) {
        self.move_to(self.pos.saturating_add(amount));
    }

    #[inline(always)]
    pub fn advance(&mut self) {
        self.pos += 1;
    }

    #[inline(always)]
    pub fn advance_twice(&mut self) {
        self.pos += 2;
    }

    pub fn move_to(&mut self, pos: usize) {
        self.pos = pos.min(self.input.len());
    }
}
