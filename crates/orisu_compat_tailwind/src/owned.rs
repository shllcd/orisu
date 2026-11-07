use crate::Tailwind;
use orisu_allocator::Allocator;
use orisu_canonical::{CanonicalizeOptions, Canonicalizer};
use orisu_compiler::compiler::{self, CompileError, Compiler, LoadStylesheet, Options};

struct State<'a> {
    compiler: Compiler<'a>,
    canonicalizer: Option<Canonicalizer<'a>>,
}

self_cell::self_cell!(
    struct Arena {
        owner: Allocator,
        #[not_covariant]
        dependent: State,
    }
);

/// A Tailwind compiler together with the arena it lives in, and a canonicalizer created on first use.
pub struct OwnedCompiler(Arena);

impl OwnedCompiler {
    /// `polyfills` defaults to all of them.
    pub fn compile(
        css: &str,
        base: &str,
        polyfills: Option<u8>,
        load_stylesheet: Option<Box<LoadStylesheet<'_>>>,
    ) -> Result<Self, CompileError> {
        Arena::try_new(Allocator::new(), |alloc| {
            let mut options = Options { base, load_stylesheet, ..Options::new(&Tailwind) };
            if let Some(polyfills) = polyfills {
                options.polyfills = polyfills;
            }
            compiler::compile(alloc.alloc_str(css), alloc, options).map(|compiler| State { compiler, canonicalizer: None })
        })
        .map(Self)
    }

    pub fn with<R>(&self, f: impl for<'a> FnOnce(&Compiler<'a>) -> R) -> R {
        self.0.with_dependent(|_, state| f(&state.compiler))
    }

    pub fn with_mut<R>(&mut self, f: impl for<'a> FnOnce(&mut Compiler<'a>) -> R) -> R {
        self.0.with_dependent_mut(|_, state| f(&mut state.compiler))
    }

    pub fn canonicalize(&mut self, candidates: &[&str], options: &CanonicalizeOptions) -> Vec<String> {
        self.0.with_dependent_mut(|alloc, state| {
            state.canonicalizer.get_or_insert_with(|| Canonicalizer::new(alloc)).canonicalize(
                &state.compiler.design_system,
                candidates,
                options,
            )
        })
    }

    /// Bytes the arena holds, which only grow as candidates are compiled.
    pub fn allocated_bytes(&self) -> usize {
        self.0.borrow_owner().allocated_bytes()
    }
}
