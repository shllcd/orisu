use crate::extractor::pre_processors::pre_processor::PreProcessor;
use bstr::ByteSlice;

#[derive(Debug, Default)]
pub struct Svelte;

impl PreProcessor for Svelte {
    fn process(&self, content: &[u8]) -> Vec<u8> {
        content.replace(" class:", " class ").replace("\tclass:", " class ").replace("\nclass:", " class ")
    }
}
