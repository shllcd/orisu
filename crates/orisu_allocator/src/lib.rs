//! Two arenas back every orisu tree. `'perm` data (theme, registry, compiled rules) lives as long as the
//! design system; `'frame` data lives for one build. Both are bump arenas; the lifetimes keep them apart.

mod intern;
mod vec;

pub use bumpalo::Bump as Allocator;
pub use bumpalo::collections::String as ArenaString;
pub use intern::{Id, Interner};
pub use vec::Vec;

pub trait AllocExt {
    fn str<'a>(&'a self, s: &str) -> &'a str;
    fn concat<'a>(&'a self, parts: &[&str]) -> &'a str;
}

impl AllocExt for Allocator {
    #[inline]
    fn str<'a>(&'a self, s: &str) -> &'a str {
        if s.is_empty() { "" } else { self.alloc_str(s) }
    }

    fn concat<'a>(&'a self, parts: &[&str]) -> &'a str {
        let len = parts.iter().map(|p| p.len()).sum();
        let mut out = ArenaString::with_capacity_in(len, self);
        for p in parts {
            out.push_str(p);
        }
        out.into_bump_str()
    }
}

#[macro_export]
macro_rules! afmt {
    ($alloc:expr, $($arg:tt)*) => {{
        use std::fmt::Write as _;
        let mut s = $crate::ArenaString::new_in($alloc);
        let _ = write!(s, $($arg)*);
        s.into_bump_str()
    }};
}
