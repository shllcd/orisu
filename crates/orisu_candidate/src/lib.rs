pub mod group;
mod parse;
pub mod print;

pub use parse::*;

orisu_allocator::define_id! {
    /// A raw class string seen by a compiler, interned for the compiler's lifetime.
    pub struct CandidateId;
}

orisu_allocator::define_id! {
    /// A parsed variant, interned so variant order can be computed once and compared as integers.
    pub struct VariantId;
}
