//! A bucket-based, unordered container with stable references and O(1) insertion/erasure.

use core::alloc::Allocator;
use core::marker::PhantomData;
use core::ptr::NonNull;

use alloc::alloc::Global;

use crate::free_list;
use crate::group::Group;
use crate::iter::{IntoIter, Iter, IterMut};
use crate::skipfield::{self, Cursor};

const DEFAULT_MIN_BLOCK_CAPACITY: u16 = 8;
const DEFAULT_MAX_BLOCK_CAPACITY: u16 = 8192;

pub struct Hive<T, A: Allocator = Global> {
    head: Option<NonNull<Group<T, A>>>,
    tail: Option<NonNull<Group<T, A>>>,
    begin: Cursor<T, A>,
    end: Cursor<T, A>,
    erasure_groups_head: Option<NonNull<Group<T, A>>>,
    reserved_groups: Option<NonNull<Group<T, A>>>,
    len: usize,
    capacity: usize,
    min_block_capacity: u16,
    max_block_capacity: u16,
    allocator: A,
}

// ── Cursor helpers ──

fn null_cursor<T, A: Allocator>() -> Cursor<T, A> {
    Cursor { group: None, element: core::ptr::null(), skipfield: core::ptr::null(), _marker: PhantomData }
}

fn make_cursor<T, A: Allocator>(group: NonNull<Group<T, A>>, element: *const u8, skipfield: *const u16) -> Cursor<T, A> {
    Cursor { group: Some(group), element, skipfield, _marker: PhantomData }
}

impl<T, A: Allocator> Hive<T, A> {
    unsafe fn begin_cursor_of(group: NonNull<Group<T, A>>) -> Cursor<T, A> {
        let g = group.as_ref();
        make_cursor(group, g.elements_base(), g.skipfield_ptr())
    }

    unsafe fn end_cursor_of(group: NonNull<Group<T, A>>, constructed: u16) -> Cursor<T, A> {
        let g = group.as_ref();
        make_cursor(
            group,
            g.elements_base().add(constructed as usize * g.slot_size),
            g.skipfield_ptr().add(constructed as usize),
        )
    }
}

// ── Construction ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    pub fn new_in(allocator: A) -> Self {
        Self {
            head: None, tail: None,
            begin: null_cursor(), end: null_cursor(),
            erasure_groups_head: None, reserved_groups: None,
            len: 0, capacity: 0,
            min_block_capacity: DEFAULT_MIN_BLOCK_CAPACITY,
            max_block_capacity: DEFAULT_MAX_BLOCK_CAPACITY,
            allocator,
        }
    }
}

impl<T> Hive<T, Global> {
    pub fn new() -> Self { Self::new_in(Global) }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut hive = Self::new();
        hive.reserve(capacity);
        hive
    }
}

impl<T> Default for Hive<T, Global> {
    fn default() -> Self { Self::new() }
}

// ── Size ──

impl<T, A: Allocator> Hive<T, A> {
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub fn capacity(&self) -> usize { self.capacity }
    pub fn block_capacity_limits(&self) -> (u16, u16) {
        (self.min_block_capacity, self.max_block_capacity)
    }
}

// ── Iteration ──

impl<T, A: Allocator> Hive<T, A> {
    pub fn iter(&self) -> Iter<'_, T, A> {
        unsafe { Iter::new(self.begin, self.end, self.len) }
    }
    pub fn iter_mut(&mut self) -> IterMut<'_, T, A> {
        unsafe { IterMut::new(self.begin, self.end, self.len) }
    }
}

impl<T, A: Allocator> IntoIterator for Hive<T, A> {
    type Item = T;
    type IntoIter = IntoIter<T, A>;
    fn into_iter(self) -> IntoIter<T, A> {
        let len = self.len;
        let begin = self.begin;
        let end = self.end;
        core::mem::forget(self);
        unsafe { IntoIter::new(begin, end, len) }
    }
}

impl<'a, T, A: Allocator> IntoIterator for &'a Hive<T, A> {
    type Item = &'a T; type IntoIter = Iter<'a, T, A>;
    fn into_iter(self) -> Iter<'a, T, A> { self.iter() }
}

impl<'a, T, A: Allocator> IntoIterator for &'a mut Hive<T, A> {
    type Item = &'a mut T; type IntoIter = IterMut<'a, T, A>;
    fn into_iter(self) -> IterMut<'a, T, A> { self.iter_mut() }
}

// ── Internal: group ops ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    fn new_group_capacity(&self) -> u16 {
        let desired = (self.len as u16).max(self.min_block_capacity).min(self.max_block_capacity);
        desired.max(self.min_block_capacity)
    }

    unsafe fn allocate_new_group(&mut self, capacity: u16, prev: Option<NonNull<Group<T, A>>>) -> NonNull<Group<T, A>> {
        let group = Group::allocate(capacity, prev, self.allocator.clone());
        self.capacity += capacity as usize;
        if let Some(mut p) = prev { p.as_mut().next = Some(group); }
        if self.head.is_none() { self.head = Some(group); }
        self.tail = Some(group);
        group
    }

    unsafe fn reuse_reserved_group(&mut self, prev: Option<NonNull<Group<T, A>>>) -> NonNull<Group<T, A>> {
        let mut group = self.reserved_groups.expect("no reserved groups");
        let g = group.as_mut();
        self.reserved_groups = g.next;
        let gn = prev.map_or(0, |p| p.as_ref().group_number + 1);
        Group::reset(group, None, prev, gn);
        if let Some(mut p) = prev { p.as_mut().next = Some(group); }
        self.tail = Some(group);
        group
    }

    unsafe fn add_to_erasures_list(&mut self, mut group: NonNull<Group<T, A>>) {
        let g = group.as_mut();
        g.erasures_prev = None;
        g.erasures_next = self.erasure_groups_head;
        if let Some(mut h) = self.erasure_groups_head { h.as_mut().erasures_prev = Some(group); }
        self.erasure_groups_head = Some(group);
    }

    unsafe fn remove_from_erasures_list(&mut self, mut group: NonNull<Group<T, A>>) {
        let g = group.as_mut();
        if let Some(mut p) = g.erasures_prev { p.as_mut().erasures_next = g.erasures_next; }
        else { self.erasure_groups_head = g.erasures_next; }
        if let Some(mut n) = g.erasures_next { n.as_mut().erasures_prev = g.erasures_prev; }
        g.erasures_prev = None;
        g.erasures_next = None;
    }

    unsafe fn move_to_reserved_list(&mut self, mut group: NonNull<Group<T, A>>) {
        let g = group.as_mut();
        if let Some(mut p) = g.prev { p.as_mut().next = g.next; }
        else { self.head = g.next; }
        if let Some(mut n) = g.next { n.as_mut().prev = g.prev; }
        else { self.tail = g.prev; }
        if g.erasures_prev.is_some() || g.erasures_next.is_some() || self.erasure_groups_head == Some(group) {
            self.remove_from_erasures_list(group);
        }
        g.next = self.reserved_groups;
        g.prev = None;
        g.free_list_head = u16::MAX;
        self.reserved_groups = Some(group);
    }

    unsafe fn find_group_for(&self, element: *const u8) -> Option<NonNull<Group<T, A>>> {
        let mut g = self.head;
        while let Some(group) = g {
            let gg = group.as_ref();
            let base = gg.elements_base();
            let end = base.add(gg.capacity as usize * gg.slot_size);
            if element >= base && element < end { return Some(group); }
            g = gg.next;
        }
        None
    }
}

// ── Insert ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    pub fn insert(&mut self, value: T) -> *const T {
        let ptr = self.insert_inner(value);
        ptr as *const T
    }

    pub fn insert_mut(&mut self, value: T) -> *mut T {
        self.insert_inner(value)
    }

    fn insert_inner(&mut self, value: T) -> *mut T {
        if let Some(eg) = self.erasure_groups_head {
            unsafe { self.insert_reuse_erased(value, eg) }
        } else if let Some(tail) = self.tail {
            if unsafe { !tail.as_ref().is_full() } {
                unsafe { self.insert_append_tail(value) }
            } else {
                unsafe { self.insert_new_group(value) }
            }
        } else {
            unsafe { self.insert_first(value) }
        }
    }

    unsafe fn insert_first(&mut self, value: T) -> *mut T {
        let mut group = self.allocate_new_group(self.new_group_capacity(), None);
        let g = group.as_mut();
        let ptr = g.element_ptr_mut(0);
        ptr.write(value);
        g.active_count = 1;
        self.len = 1;
        self.begin = Self::begin_cursor_of(group);
        self.end = Self::end_cursor_of(group, 1);
        ptr
    }

    unsafe fn insert_append_tail(&mut self, value: T) -> *mut T {
        let mut end_group = self.end.group.unwrap();
        let g = end_group.as_mut();
        let elem_byte = self.end.element as *mut u8;
        let ptr = elem_byte as *mut T;
        ptr.write(value);
        self.end.element = elem_byte.add(g.slot_size);
        self.end.skipfield = self.end.skipfield.add(1);
        g.active_count += 1;
        self.len += 1;
        ptr
    }

    unsafe fn insert_reuse_erased(&mut self, value: T, mut erasure_group: NonNull<Group<T, A>>) -> *mut T {
        let g = erasure_group.as_mut();
        let index = free_list::pop_free_slot::<T, A>(erasure_group);
        let ptr = g.element_ptr_mut(index);
        
        // Check if this slot is before the current begin cursor (for begin update)
        let new_elem_byte = g.elements_base().add(index as usize * g.slot_size);
        let update_begin = self.begin.group.is_some_and(|bg| {
            erasure_group == bg && (new_elem_byte as *const u8) < self.begin.element
        });
        let new_sf = g.skipfield_ptr().add(index as usize);
        
        skipfield::mark_constructed(erasure_group, index);
        if g.free_list_head == u16::MAX {
            self.remove_from_erasures_list(erasure_group);
        }
        ptr.write(value);
        g.active_count += 1;
        self.len += 1;

        if update_begin {
            self.begin = make_cursor(erasure_group, new_elem_byte, new_sf);
        }

        ptr
    }

    unsafe fn insert_new_group(&mut self, value: T) -> *mut T {
        let prev = self.tail;
        let cap = self.new_group_capacity();
        let mut group = if self.reserved_groups.is_some() {
            self.reuse_reserved_group(prev)
        } else {
            self.allocate_new_group(cap, prev)
        };
        let g = group.as_mut();
        let ptr = g.element_ptr_mut(0);
        ptr.write(value);
        g.active_count = 1;
        self.end = Self::end_cursor_of(group, 1);
        self.len += 1;
        ptr
    }
}

// ── Erase ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    /// Erase an element by its pointer.
    ///
    /// # Safety
    /// `element_ref` must be a valid pointer to an element in this hive that
    /// has not already been erased.
    pub unsafe fn erase(&mut self, element_ref: &T) {
        self.erase_raw(element_ref as *const T as *mut T);
    }

    /// Erase an element by its mutable pointer.
    ///
    /// # Safety
    /// `element_ref` must be a valid mutable pointer to an element in this hive
    /// that has not already been erased.
    pub unsafe fn erase_mut(&mut self, element_ref: &mut T) {
        self.erase_raw(element_ref as *mut T);
    }

    unsafe fn erase_raw(&mut self, element_ptr: *mut T) {
        if cfg!(debug_assertions) {
            assert!(self.len > 0, "erase on empty hive");
        }
        let byte_ptr = element_ptr as *const u8;
        let mut group = self.find_group_for(byte_ptr).expect("element not in any group");
        let g = group.as_mut();
        let index = g.index_from_element_ptr(byte_ptr);
        debug_assert_eq!(*g.skipfield_ptr().add(index as usize), 0, "element already erased");

        element_ptr.drop_in_place();
        self.len -= 1;
        g.active_count -= 1;

        if g.active_count > 0 {
            skipfield::mark_erased(group, index);
            let was_empty = g.free_list_head == u16::MAX;
            free_list::push_free_slot::<T, A>(group, index);
            if was_empty {
                self.add_to_erasures_list(group);
            }
            if group == self.begin.group.unwrap() && byte_ptr == self.begin.element {
                self.begin = self.begin.advance_forward();
            }
        } else {
            let was_begin_group = group == self.begin.group.unwrap();
            self.move_to_reserved_list(group);
            if was_begin_group {
                if self.len == 0 {
                    self.begin = null_cursor();
                    self.end = null_cursor();
                } else {
                    self.begin = Self::begin_cursor_of(self.head.unwrap());
                    while unsafe { *self.begin.skipfield != 0 } && self.len > 0 {
                        self.begin = self.begin.advance_forward();
                    }
                }
            }
        }
    }

    pub fn clear(&mut self) {
        if self.len == 0 { return; }
        unsafe {
            let mut cur = self.begin;
            for i in 0..self.len {
                let mut g = cur.group.unwrap();
                let gg = g.as_mut();
                let idx = gg.index_from_element_ptr(cur.element);
                gg.element_ptr_mut(idx).drop_in_place();
                if i + 1 < self.len {
                    cur = cur.advance_forward();
                }
            }
        }
        let mut g = self.head;
        while let Some(mut group) = g {
            unsafe {
                let next = group.as_ref().next;
                let gm = group.as_mut();
                gm.active_count = 0;
                gm.free_list_head = u16::MAX;
                gm.erasures_next = None;
                gm.erasures_prev = None;
                gm.next = self.reserved_groups;
                gm.prev = None;
                core::ptr::write_bytes(gm.skipfield_mut(), 0, gm.capacity as usize);
                self.reserved_groups = Some(group);
                g = next;
            }
        }
        self.head = None;
        self.tail = None;
        self.begin = null_cursor();
        self.end = null_cursor();
        self.erasure_groups_head = None;
        self.len = 0;
    }

    pub fn retain<F: FnMut(&T) -> bool>(&mut self, mut f: F) {
        let mut to_erase: alloc::vec::Vec<*mut T> = alloc::vec::Vec::new();
        unsafe {
            let mut cur = self.begin;
            let count = self.len;
            for i in 0..count {
                let g = cur.group.unwrap().as_ref();
                let idx = g.index_from_element_ptr(cur.element);
                let elem_ptr = g.element_ptr_mut(idx);
                if !f(&*elem_ptr) {
                    to_erase.push(elem_ptr);
                }
                if i + 1 < count {
                    cur = cur.advance_forward();
                }
            }
        }
        for ptr in to_erase {
            unsafe { self.erase_raw(ptr); }
        }
    }
}

// ── Reserve / shrink ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    pub fn reserve(&mut self, additional: usize) {
        let needed = self.len.saturating_add(additional);
        if needed <= self.capacity { return; }
        let mut remaining = needed - self.capacity;
        while remaining > 0 {
            let cap = (remaining as u16).min(self.max_block_capacity).max(self.min_block_capacity);
            // Create a group and add it to the reserved list (not linked to tail)
            let mut group = Group::allocate(cap, None, self.allocator.clone());
            self.capacity += cap as usize;
            let g = unsafe { group.as_mut() };
            g.active_count = 0;
            g.next = self.reserved_groups;
            self.reserved_groups = Some(group);
            remaining = remaining.saturating_sub(cap as usize);
        }
        if self.len == 0 && self.head.is_none() {
            // Reserve on empty hive: need to pull one group from reserved
            let mut head = self.reserved_groups.expect("reserve allocated nothing");
            unsafe {
                let next = head.as_ref().next;
                self.reserved_groups = next;
                head.as_mut().next = None;
            }
            self.head = Some(head);
            self.tail = Some(head);
            self.begin = unsafe { Self::begin_cursor_of(head) };
            self.end = unsafe { Self::end_cursor_of(head, 0) };
        }
    }

    pub fn shrink_to_fit(&mut self) {}

    pub fn trim_capacity(&mut self) {
        unsafe {
            while let Some(group) = self.reserved_groups {
                let next = group.as_ref().next;
                self.capacity = self.capacity.saturating_sub(group.as_ref().capacity as usize);
                Group::deallocate_group(group);
                self.reserved_groups = next;
            }
        }
    }
}

// ── Sort ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    pub fn sort_by<F: FnMut(&T, &T) -> core::cmp::Ordering>(&mut self, mut compare: F) {
        if self.len <= 1 { return; }
        let count = self.len;
        let mut v: alloc::vec::Vec<T> = alloc::vec::Vec::with_capacity(count);
        unsafe {
            let mut cur = self.begin;
            for i in 0..count {
                let mut g = cur.group.unwrap();
                let gg = g.as_mut();
                let idx = gg.index_from_element_ptr(cur.element);
                v.push(gg.element_ptr(idx).read());
                *gg.skipfield_mut().add(idx as usize) = 1;
                if i + 1 < count {
                    cur = cur.advance_forward();
                }
            }
        }
        let mut g = self.head;
        while let Some(mut group) = g {
            unsafe {
                let next = group.as_ref().next;
                let gm = group.as_mut();
                gm.active_count = 0;
                gm.free_list_head = u16::MAX;
                gm.erasures_next = None;
                gm.erasures_prev = None;
                gm.next = self.reserved_groups;
                gm.prev = None;
                core::ptr::write_bytes(gm.skipfield_mut(), 0, gm.capacity as usize);
                self.reserved_groups = Some(group);
                g = next;
            }
        }
        self.head = None;
        self.tail = None;
        self.begin = null_cursor();
        self.end = null_cursor();
        self.erasure_groups_head = None;
        self.len = 0;
        v.sort_by(&mut compare);
        for item in v {
            self.insert(item);
        }
    }
}

impl<T: Ord, A: Allocator + Clone> Hive<T, A> {
    pub fn sort(&mut self) { self.sort_by(|a, b| a.cmp(b)) }
}

// ── Clone ──

impl<T: Clone, A: Allocator + Clone> Clone for Hive<T, A> {
    fn clone(&self) -> Self {
        let mut new = Self::new_in(self.allocator.clone());
        new.min_block_capacity = self.min_block_capacity;
        new.max_block_capacity = self.max_block_capacity;
        new.reserve(self.len);
        for item in self.iter() { new.insert(item.clone()); }
        new
    }
}

// ── Collect / Extend ──

impl<T> FromIterator<T> for Hive<T, Global> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();
        let mut hive = Self::with_capacity(lower);
        for item in iter { hive.insert(item); }
        hive
    }
}

impl<T, A: Allocator + Clone> Extend<T> for Hive<T, A> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();
        self.reserve(lower);
        for item in iter { self.insert(item); }
    }
}

impl<'a, T: Copy + 'a, A: Allocator + Clone> Extend<&'a T> for Hive<T, A> {
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, iter: I) {
        self.extend(iter.into_iter().copied());
    }
}

// ── Drop ──

impl<T, A: Allocator> Drop for Hive<T, A> {
    fn drop(&mut self) {
        if self.len > 0 {
            unsafe {
                let mut cur = self.begin;
                for i in 0..self.len {
                    let mut g = cur.group.unwrap();
                    let gg = g.as_mut();
                    let idx = gg.index_from_element_ptr(cur.element);
                    gg.element_ptr_mut(idx).drop_in_place();
                    if i + 1 < self.len {
                        cur = cur.advance_forward();
                    }
                }
            }
        }
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

// ── Debug ──

impl<T: core::fmt::Debug, A: Allocator> core::fmt::Debug for Hive<T, A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
