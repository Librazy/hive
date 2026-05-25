//! Iterator types for `Hive`: `Iter`, `IterMut`, `IntoIter`.

use core::alloc::Allocator;
use core::iter::FusedIterator;
use core::marker::PhantomData;
use crate::skipfield::Cursor;

macro_rules! iter_next {
    ($self:expr, $front:ident, $as_type:expr) => {{
        if $self.remaining == 0 { return None; }
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
        if $self.remaining == 0 { return None; }
        $self.remaining -= 1;
        $self.$back = unsafe { $self.$back.advance_backward() };
        Some($as_type)
    }};
}

pub struct Iter<'a, T: 'a, A: Allocator + 'a> {
    front: Cursor<T, A>, back: Cursor<T, A>, remaining: usize,
    _marker: PhantomData<&'a T>,
}

impl<'a, T, A: Allocator> Iter<'a, T, A> {
    pub(crate) unsafe fn new(begin: Cursor<T, A>, end: Cursor<T, A>, len: usize) -> Self {
        Self { front: begin, back: end, remaining: len, _marker: PhantomData }
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
    fn size_hint(&self) -> (usize, Option<usize>) { (self.remaining, Some(self.remaining)) }
    fn count(self) -> usize { self.remaining }
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

pub struct IterMut<'a, T: 'a, A: Allocator + 'a> {
    front: Cursor<T, A>, back: Cursor<T, A>, remaining: usize,
    _marker: PhantomData<&'a mut T>,
}

impl<'a, T, A: Allocator> IterMut<'a, T, A> {
    pub(crate) unsafe fn new(begin: Cursor<T, A>, end: Cursor<T, A>, len: usize) -> Self {
        Self { front: begin, back: end, remaining: len, _marker: PhantomData }
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
    fn size_hint(&self) -> (usize, Option<usize>) { (self.remaining, Some(self.remaining)) }
    fn count(self) -> usize { self.remaining }
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

pub struct IntoIter<T, A: Allocator> {
    front: Cursor<T, A>, back: Cursor<T, A>, remaining: usize,
    _marker: PhantomData<T>,
}

impl<T, A: Allocator> IntoIter<T, A> {
    pub(crate) unsafe fn new(begin: Cursor<T, A>, end: Cursor<T, A>, len: usize) -> Self {
        Self { front: begin, back: end, remaining: len, _marker: PhantomData }
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
    fn size_hint(&self) -> (usize, Option<usize>) { (self.remaining, Some(self.remaining)) }
    fn count(self) -> usize { self.remaining }
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
    fn drop(&mut self) { for _ in self {} }
}
