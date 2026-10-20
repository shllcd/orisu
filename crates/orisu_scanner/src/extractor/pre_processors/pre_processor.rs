pub trait PreProcessor: Sized + Default {
    fn process(&self, content: &[u8]) -> Vec<u8>;
}
