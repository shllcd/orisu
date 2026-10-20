use rustc_hash::FxHashMap;
use std::marker::PhantomData;

/// A dense `u32` handle handed out by an [`Interner`].
pub trait Id: Copy + Eq {
    fn from_index(index: u32) -> Self;
    fn index(self) -> u32;
}

/// Maps strings to dense ids in insertion order; ids index straight into side tables.
pub struct Interner<I> {
    ids: FxHashMap<Box<str>, u32>,
    strings: std::vec::Vec<*const str>,
    marker: PhantomData<I>,
}

// SAFETY: `strings` points into the boxed keys of `ids`, which are never removed or mutated while the interner lives.
unsafe impl<I> Send for Interner<I> {}
// SAFETY: shared access only reads the boxed keys; see `Send`.
unsafe impl<I> Sync for Interner<I> {}

impl<I> Default for Interner<I> {
    fn default() -> Self {
        Self { ids: FxHashMap::default(), strings: std::vec::Vec::new(), marker: PhantomData }
    }
}

impl<I: Id> Interner<I> {
    pub fn intern(&mut self, s: &str) -> I {
        if let Some(&id) = self.ids.get(s) {
            return I::from_index(id);
        }
        let id = self.strings.len() as u32;
        let boxed: Box<str> = s.into();
        self.strings.push(&*boxed as *const str);
        self.ids.insert(boxed, id);
        I::from_index(id)
    }

    #[inline]
    pub fn get(&self, s: &str) -> Option<I> {
        self.ids.get(s).map(|&id| I::from_index(id))
    }

    #[inline]
    pub fn resolve(&self, id: I) -> &str {
        // SAFETY: every pointer refers to a key box owned by `ids`, alive as long as `self`.
        unsafe { &*self.strings[id.index() as usize] }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

#[macro_export]
macro_rules! define_id {
    ($(#[$meta:meta])* $vis:vis struct $name:ident;) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis struct $name(pub u32);

        impl $crate::Id for $name {
            #[inline]
            fn from_index(index: u32) -> Self {
                Self(index)
            }

            #[inline]
            fn index(self) -> u32 {
                self.0
            }
        }
    };
}
