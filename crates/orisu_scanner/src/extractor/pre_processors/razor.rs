use bstr::ByteSlice;

pub fn process(content: &[u8]) -> Vec<u8> {
    content.replace("@@", " @").replace(r#"@("@")"#, "     @")
}
