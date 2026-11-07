use bstr::ByteSlice;

pub fn process(content: &[u8]) -> Vec<u8> {
    content.replace(" class:", " class ").replace("\tclass:", " class ").replace("\nclass:", " class ")
}
