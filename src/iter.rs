//! Iterator types for [`Hive`](crate::Hive).
//!
//! This module provides three iterator types:
//!
//! | Iterator | Produces | Obtained via |
//! |---|---|---|
//! | [`Iter`] | `&T` | [`Hive::iter`], `for x in &hive` |
//! | [`IterMut`] | `&mut T` | [`Hive::iter_mut`], `for x in &mut hive` |
//! | [`IntoIter`] | `T` | `for x in hive` (consuming) |
//!
//! All three are double-ended, exact-size, and fused.

use crate::allocator::Allocator;
use crate::group::Group;
use crate::skipfield::Cursor;
use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::ptr::NonNull;

macro_rules! iter_next {
    ($self:expr, $front:ident, $as_type:expr) => {{
        if $self.remaining == 0 {
            return None;
        }
        $self.remaining -= 1;
        let result = $as_type;
        if $self.remaining > 0 {
            $self.$front = unsafe { $self.$front.advance_forward() };
        }
        Some(result)
    }};
}

macro_rules! iter_next_back {
    ($self:expr, $back:ident, $as_type:expr) => {{
        if $self.remaining == 0 {
            return None;
        }
        $self.remaining -= 1;
        let result = $as_type;
        if $self.remaining > 0 {
            $self.$back = unsafe { $self.$back.advance_backward() };
        }
        Some(result)
    }};
}

/// A shared iterator over the elements of a [`Hive`](crate::Hive).
///
/// Yields `&T` in insertion order (left-to-right across blocks). The iterator
/// has a fixed remaining length and must not be used after externally erasing
/// elements through raw-pointer APIs.
///
/// This iterator is double-ended, exact-size, and fused.
pub struct Iter<'a, T: 'a, A: Allocator + 'a> {
    front: Cursor<T, A>,
    back: Cursor<T, A>,
    remaining: usize,
    _marker: PhantomData<&'a T>,
}

impl<'a, T, A: Allocator> Iter<'a, T, A> {
    pub(crate) unsafe fn new(begin: Cursor<T, A>, end: Cursor<T, A>, len: usize) -> Self {
        let back = if len == 0 {
            end
        } else {
            end.advance_backward()
        };
        Self {
            front: begin,
            back,
            remaining: len,
            _marker: PhantomData,
        }
    }
}

impl<'a, T, A: Allocator> Iterator for Iter<'a, T, A> {
    type Item = &'a T;
    fn next(&mut self) -> Option<&'a T> {
        iter_next!(self, front, unsafe {
            let g = self.front.group.unwrap().as_ref();
            let idx = g.index_from_element_ptr(self.front.element);
            &*(g.element_ptr(idx))
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
    fn count(self) -> usize {
        self.remaining
    }
}

impl<'a, T, A: Allocator> DoubleEndedIterator for Iter<'a, T, A> {
    fn next_back(&mut self) -> Option<&'a T> {
        iter_next_back!(self, back, unsafe {
            let g = self.back.group.unwrap().as_ref();
            let idx = g.index_from_element_ptr(self.back.element);
            &*(g.element_ptr(idx))
        })
    }
}

impl<'a, T, A: Allocator> ExactSizeIterator for Iter<'a, T, A> {}
impl<'a, T, A: Allocator> FusedIterator for Iter<'a, T, A> {}

/// A mutable iterator over the elements of a [`Hive`](crate::Hive).
///
/// Yields `&mut T` in insertion order. Because each element is yielded at most
/// once, mutable iteration is sound even in the presence of erased-slot reuse.
///
/// This iterator is double-ended, exact-size, and fused.
pub struct IterMut<'a, T: 'a, A: Allocator + 'a> {
    front: Cursor<T, A>,
    back: Cursor<T, A>,
    remaining: usize,
    _marker: PhantomData<&'a mut T>,
}

impl<'a, T, A: Allocator> IterMut<'a, T, A> {
    pub(crate) unsafe fn new(begin: Cursor<T, A>, end: Cursor<T, A>, len: usize) -> Self {
        let back = if len == 0 {
            end
        } else {
            end.advance_backward()
        };
        Self {
            front: begin,
            back,
            remaining: len,
            _marker: PhantomData,
        }
    }
}

impl<'a, T, A: Allocator> Iterator for IterMut<'a, T, A> {
    type Item = &'a mut T;
    fn next(&mut self) -> Option<&'a mut T> {
        iter_next!(self, front, unsafe {
            let g = self.front.group.unwrap().as_mut();
            let idx = g.index_from_element_ptr(self.front.element);
            &mut *(g.element_ptr_mut(idx))
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
    fn count(self) -> usize {
        self.remaining
    }
}

impl<'a, T, A: Allocator> DoubleEndedIterator for IterMut<'a, T, A> {
    fn next_back(&mut self) -> Option<&'a mut T> {
        iter_next_back!(self, back, unsafe {
            let g = self.back.group.unwrap().as_mut();
            let idx = g.index_from_element_ptr(self.back.element);
            &mut *(g.element_ptr_mut(idx))
        })
    }
}

impl<'a, T, A: Allocator> ExactSizeIterator for IterMut<'a, T, A> {}
impl<'a, T, A: Allocator> FusedIterator for IterMut<'a, T, A> {}

/// A consuming iterator over the elements of a [`Hive`](crate::Hive).
///
/// Yields `T` by value in insertion order, consuming the hive. Elements that
/// have not been exhausted when the iterator is dropped are still dropped
/// normally, and all group allocations are freed.
///
/// This iterator is double-ended, exact-size, and fused.
pub struct IntoIter<T, A: Allocator> {
    front: Cursor<T, A>,
    back: Cursor<T, A>,
    remaining: usize,
    head: Option<NonNull<Group<T, A>>>,
    reserved_groups: Option<NonNull<Group<T, A>>>,
    _marker: PhantomData<T>,
}

impl<T, A: Allocator> IntoIter<T, A> {
    pub(crate) unsafe fn new(
        begin: Cursor<T, A>,
        end: Cursor<T, A>,
        len: usize,
        head: Option<NonNull<Group<T, A>>>,
        reserved_groups: Option<NonNull<Group<T, A>>>,
    ) -> Self {
        let back = if len == 0 {
            end
        } else {
            end.advance_backward()
        };
        Self {
            front: begin,
            back,
            remaining: len,
            head,
            reserved_groups,
            _marker: PhantomData,
        }
    }
}

impl<T, A: Allocator> Iterator for IntoIter<T, A> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        iter_next!(self, front, unsafe {
            let g = self.front.group.unwrap().as_mut();
            let idx = g.index_from_element_ptr(self.front.element);
            g.element_ptr(idx).read()
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
    fn count(self) -> usize {
        self.remaining
    }
}

impl<T, A: Allocator> DoubleEndedIterator for IntoIter<T, A> {
    fn next_back(&mut self) -> Option<T> {
        iter_next_back!(self, back, unsafe {
            let g = self.back.group.unwrap().as_mut();
            let idx = g.index_from_element_ptr(self.back.element);
            g.element_ptr(idx).read()
        })
    }
}

impl<T, A: Allocator> ExactSizeIterator for IntoIter<T, A> {}
impl<T, A: Allocator> FusedIterator for IntoIter<T, A> {}

impl<T, A: Allocator> Drop for IntoIter<T, A> {
    fn drop(&mut self) {
        while self.next().is_some() {}

        let mut g = self.head;
        while let Some(group) = g {
            unsafe {
                let next = group.as_ref().next;
                Group::deallocate_group(group);
                g = next;
            }
        }

        let mut g = self.reserved_groups;
        while let Some(group) = g {
            unsafe {
                let next = group.as_ref().next;
                Group::deallocate_group(group);
                g = next;
            }
        }
    }
}
