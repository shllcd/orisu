use crate::extractor::pre_processors::pre_processor::PreProcessor;
use bstr::ByteSlice;

#[derive(Debug, Default)]
pub struct Razor;

impl PreProcessor for Razor {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        content.replace("@@", " @").replace(r#"@("@")"#, "     @")
    }
}
