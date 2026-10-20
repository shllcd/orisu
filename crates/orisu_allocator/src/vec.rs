use crate::Allocator;
use std::fmt;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut, RangeBounds};

/// Arena vector that never runs destructors; its elements must not need dropping.
///
/// Shared references only reach the elements, never the arena, so a tree of these is `Sync` and can be read
/// from many threads once built. Copies go through [`Vec::clone_in`] into an arena the caller owns.
pub struct Vec<'a, T>(ManuallyDrop<bumpalo::collections::Vec<'a, T>>);

// SAFETY: `&Vec` exposes only `&[T]`; every path that touches the arena takes `&mut self` or `self`, and `Vec`
// stays `!Send`, so the arena is only ever used from the thread that owns it.
unsafe impl<T: Sync> Sync for Vec<'_, T> {}

impl<'a, T> Vec<'a, T> {
    const ASSERT_NO_DROP: () = assert!(!std::mem::needs_drop::<T>(), "arena Vec elements must not need Drop");

    #[inline]
    pub fn new_in(alloc: &'a Allocator) -> Self {
        let () = Self::ASSERT_NO_DROP;
        Self(ManuallyDrop::new(bumpalo::collections::Vec::new_in(alloc)))
    }

    #[inline]
    pub fn with_capacity_in(capacity: usize, alloc: &'a Allocator) -> Self {
        let () = Self::ASSERT_NO_DROP;
        Self(ManuallyDrop::new(bumpalo::collections::Vec::with_capacity_in(capacity, alloc)))
    }

    #[inline]
    pub fn from_iter_in(iter: impl IntoIterator<Item = T>, alloc: &'a Allocator) -> Self {
        let () = Self::ASSERT_NO_DROP;
        Self(ManuallyDrop::new(bumpalo::collections::Vec::from_iter_in(iter, alloc)))
    }

    #[inline]
    pub fn into_bump_slice(self) -> &'a [T] {
        ManuallyDrop::into_inner(self.0).into_bump_slice()
    }

    #[inline]
    pub fn into_bump_slice_mut(self) -> &'a mut [T] {
        ManuallyDrop::into_inner(self.0).into_bump_slice_mut()
    }

    #[inline]
    pub fn push(&mut self, value: T) {
        self.0.push(value);
    }

    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        self.0.pop()
    }

    #[inline]
    pub fn insert(&mut self, index: usize, value: T) {
        self.0.insert(index, value);
    }

    #[inline]
    pub fn remove(&mut self, index: usize) -> T {
        self.0.remove(index)
    }

    #[inline]
    pub fn swap_remove(&mut self, index: usize) -> T {
        self.0.swap_remove(index)
    }

    #[inline]
    pub fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    #[inline]
    pub fn clear(&mut self) {
        self.0.clear();
    }

    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }

    #[inline]
    pub fn reserve_exact(&mut self, additional: usize) {
        self.0.reserve_exact(additional);
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    #[inline]
    pub fn retain(&mut self, f: impl FnMut(&T) -> bool) {
        self.0.retain(f);
    }

    #[inline]
    pub fn retain_mut(&mut self, f: impl FnMut(&mut T) -> bool) {
        self.0.retain_mut(f);
    }

    #[inline]
    pub fn dedup_by(&mut self, same: impl FnMut(&mut T, &mut T) -> bool) {
        self.0.dedup_by(same);
    }

    #[inline]
    pub fn append(&mut self, other: &mut Self) {
        self.0.append(&mut other.0);
    }

    #[inline]
    pub fn drain(&mut self, range: impl RangeBounds<usize>) -> bumpalo::collections::vec::Drain<'_, 'a, T> {
        self.0.drain(range)
    }

    #[inline]
    pub fn splice<I: IntoIterator<Item = T>>(&mut self, range: impl RangeBounds<usize>, with: I) {
        self.0.splice(range, with);
    }

    #[inline]
    pub fn split_off(&mut self, at: usize) -> Self {
        Self(ManuallyDrop::new(self.0.split_off(at)))
    }

    #[inline]
    pub fn extend_from_slice(&mut self, items: &[T])
    where
        T: Clone,
    {
        self.0.extend_from_slice(items);
    }

    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.0
    }

    /// Copies the elements into `alloc`, which may be another arena than the one holding `self`.
    pub fn clone_in<'b>(&self, alloc: &'b Allocator) -> Vec<'b, T>
    where
        T: Clone,
    {
        let mut out = Vec::with_capacity_in(self.len(), alloc);
        out.extend_from_slice(self);
        out
    }
}

impl<T> Deref for Vec<'_, T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T> DerefMut for Vec<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

impl<'a, T> IntoIterator for Vec<'a, T> {
    type Item = T;
    type IntoIter = bumpalo::collections::vec::IntoIter<'a, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        ManuallyDrop::into_inner(self.0).into_iter()
    }
}

impl<'v, T> IntoIterator for &'v Vec<'_, T> {
    type Item = &'v T;
    type IntoIter = std::slice::Iter<'v, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'v, T> IntoIterator for &'v mut Vec<'_, T> {
    type Item = &'v mut T;
    type IntoIter = std::slice::IterMut<'v, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

impl<T: PartialEq> PartialEq for Vec<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Eq> Eq for Vec<'_, T> {}

impl<T: std::hash::Hash> std::hash::Hash for Vec<'_, T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl<T: fmt::Debug> fmt::Debug for Vec<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T> Extend<T> for Vec<'_, T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.0.extend(iter)
    }
}
