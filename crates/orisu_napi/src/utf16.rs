/// The `IndexConverter` is used to convert UTF-8 *BYTE* indexes to UTF-16
/// *character* indexes
#[derive(Clone)]
pub struct IndexConverter<'a> {
    input: &'a str,
    curr_utf8: usize,
    curr_utf16: usize,
}

impl<'a> IndexConverter<'a> {
    pub fn new(input: &'a str) -> Self {
        Self { input, curr_utf8: 0, curr_utf16: 0 }
    }

    pub fn get(&mut self, pos: usize) -> i64 {
        #[cfg(debug_assertions)]
        if self.curr_utf8 > self.input.len() {
            panic!("curr_utf8 points past the end of the input string");
        }

        if pos < self.curr_utf8 {
            self.curr_utf8 = 0;
            self.curr_utf16 = 0;
        }

        // SAFETY: `curr_utf8` only ever advances by whole chars and never past the end of `input`, so it stays on a char
        // boundary whatever `pos` is.
        let slice = unsafe { self.input.get_unchecked(self.curr_utf8..) };

        for c in slice.chars() {
            if self.curr_utf8 >= pos {
                break;
            }

            self.curr_utf8 += c.len_utf8();
            self.curr_utf16 += c.len_utf16();
        }

        self.curr_utf16 as i64
    }
}
