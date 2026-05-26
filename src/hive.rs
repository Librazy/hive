//! A bucket-based, unordered container with stable references and O(1) insertion/erasure.

use crate::allocator::{Allocator, Global};
use core::marker::PhantomData;
use core::mem::{needs_drop, ManuallyDrop, MaybeUninit};
use core::ptr::NonNull;

use crate::free_list;
use crate::group::Group;
use crate::iter::{IntoIter, Iter, IterMut};
use crate::skipfield::{self, Cursor};

const DEFAULT_MIN_BLOCK_CAPACITY: u16 = 8;
const DEFAULT_MAX_BLOCK_CAPACITY: u16 = 8192;
const HARD_MIN_BLOCK_CAPACITY: u16 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockCapacityLimits {
    pub min: u16,
    pub max: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidBlockCapacityLimits;

impl BlockCapacityLimits {
    pub const fn new(min: u16, max: u16) -> Self {
        Self { min, max }
    }
}

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

// SAFETY: `Hive` owns all elements and internal groups. Moving it to another
// thread is sound when elements and the allocator can be sent there. Shared
// concurrent access still requires external synchronization.
unsafe impl<T: Send, A: Allocator + Send> Send for Hive<T, A> {}

// ── Cursor helpers ──

fn null_cursor<T, A: Allocator>() -> Cursor<T, A> {
    Cursor {
        group: None,
        element: core::ptr::null(),
        skipfield: core::ptr::null(),
        _marker: PhantomData,
    }
}

fn make_cursor<T, A: Allocator>(
    group: NonNull<Group<T, A>>,
    element: *const u8,
    skipfield: *const u16,
) -> Cursor<T, A> {
    Cursor {
        group: Some(group),
        element,
        skipfield,
        _marker: PhantomData,
    }
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
            head: None,
            tail: None,
            begin: null_cursor(),
            end: null_cursor(),
            erasure_groups_head: None,
            reserved_groups: None,
            len: 0,
            capacity: 0,
            min_block_capacity: DEFAULT_MIN_BLOCK_CAPACITY,
            max_block_capacity: DEFAULT_MAX_BLOCK_CAPACITY,
            allocator,
        }
    }

    pub fn try_new_in(
        allocator: A,
        limits: BlockCapacityLimits,
    ) -> Result<Self, InvalidBlockCapacityLimits> {
        Self::check_block_capacity_limits(limits)?;
        let mut hive = Self::new_in(allocator);
        hive.min_block_capacity = limits.min;
        hive.max_block_capacity = limits.max;
        Ok(hive)
    }
}

impl<T> Hive<T, Global> {
    pub fn new() -> Self {
        Self::new_in(Global)
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut hive = Self::new();
        hive.reserve(cap);
        hive
    }

    pub fn try_new(limits: BlockCapacityLimits) -> Result<Self, InvalidBlockCapacityLimits> {
        Self::try_new_in(Global, limits)
    }
}

impl<T> Default for Hive<T, Global> {
    fn default() -> Self {
        Self::new()
    }
}

// ── Size ──

impl<T, A: Allocator> Hive<T, A> {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn max_size(&self) -> usize {
        usize::MAX / Group::<T, A>::compute_slot_size()
    }
    pub fn block_capacity_limits(&self) -> BlockCapacityLimits {
        BlockCapacityLimits::new(self.min_block_capacity, self.max_block_capacity)
    }

    pub const fn block_capacity_default_limits() -> BlockCapacityLimits {
        BlockCapacityLimits::new(DEFAULT_MIN_BLOCK_CAPACITY, DEFAULT_MAX_BLOCK_CAPACITY)
    }

    pub const fn block_capacity_hard_limits() -> BlockCapacityLimits {
        BlockCapacityLimits::new(HARD_MIN_BLOCK_CAPACITY, u16::MAX - 1)
    }

    pub fn get_allocator(&self) -> &A {
        &self.allocator
    }

    fn check_block_capacity_limits(
        limits: BlockCapacityLimits,
    ) -> Result<(), InvalidBlockCapacityLimits> {
        let hard = Self::block_capacity_hard_limits();
        if limits.min < hard.min || limits.min > limits.max || limits.max > hard.max {
            Err(InvalidBlockCapacityLimits)
        } else {
            Ok(())
        }
    }

    unsafe fn find_group_for(&self, element: *const u8) -> Option<NonNull<Group<T, A>>> {
        let mut g = self.head;
        while let Some(group) = g {
            let gp = group.as_ptr();
            let base = (*gp).elements_base();
            let end = base.add((*gp).capacity as usize * (*gp).slot_size);
            if element >= base && element < end {
                return Some(group);
            }
            g = (*gp).next;
        }
        None
    }

    unsafe fn cursor_from_ptr(&self, ptr: *const T) -> Option<Cursor<T, A>> {
        let byte_ptr = ptr as *const u8;
        let group = self.find_group_for(byte_ptr)?;
        let gp = group.as_ptr();
        let index = (*gp).index_from_element_ptr(byte_ptr);

        if index >= (*gp).capacity {
            return None;
        }

        let element = (*gp).element_ptr(index) as *const u8;
        if element != byte_ptr || *(*gp).skipfield_ptr().add(index as usize) != 0 {
            return None;
        }

        Some(make_cursor(
            group,
            element,
            (*gp).skipfield_ptr().add(index as usize),
        ))
    }

    unsafe fn count_from_cursor(&self, cursor: Cursor<T, A>) -> usize {
        let mut cur = cursor;
        let mut count = 0;
        let last = self.end.advance_backward();

        loop {
            count += 1;
            if cur.group == last.group && cur.element == last.element {
                break;
            }
            cur = cur.advance_forward();
        }

        count
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

    /// Returns a shared reference for a pointer previously returned by this hive.
    ///
    /// # Safety
    /// `ptr` must be either null/foreign/erased, or a valid pointer returned by
    /// this hive for an element that has not been erased. Passing arbitrary
    /// pointers may be undefined behavior because pointer provenance and bounds
    /// cannot be validated completely.
    pub unsafe fn get(&self, ptr: *const T) -> Option<&T> {
        let cursor = unsafe { self.cursor_from_ptr(ptr)? };
        unsafe { Some(&*(cursor.element as *const T)) }
    }

    /// Returns a mutable reference for a pointer previously returned by this hive.
    ///
    /// # Safety
    /// See [`Hive::get`]. The caller must also ensure there are no aliases to
    /// the same element for the duration of the returned mutable borrow.
    pub unsafe fn get_mut(&mut self, ptr: *const T) -> Option<&mut T> {
        let cursor = unsafe { self.cursor_from_ptr(ptr)? };
        unsafe { Some(&mut *(cursor.element as *mut T)) }
    }

    /// Returns an iterator beginning at `ptr`.
    ///
    /// # Safety
    /// See [`Hive::get`].
    pub unsafe fn iter_from(&self, ptr: *const T) -> Option<Iter<'_, T, A>> {
        let cursor = unsafe { self.cursor_from_ptr(ptr)? };
        let remaining = unsafe { self.count_from_cursor(cursor) };
        unsafe { Some(Iter::new(cursor, self.end, remaining)) }
    }

    /// Returns a mutable iterator beginning at `ptr`.
    ///
    /// # Safety
    /// See [`Hive::get_mut`].
    pub unsafe fn iter_mut_from(&mut self, ptr: *const T) -> Option<IterMut<'_, T, A>> {
        let cursor = unsafe { self.cursor_from_ptr(ptr)? };
        let remaining = unsafe { self.count_from_cursor(cursor) };
        unsafe { Some(IterMut::new(cursor, self.end, remaining)) }
    }
}

impl<T, A: Allocator> IntoIterator for Hive<T, A> {
    type Item = T;
    type IntoIter = IntoIter<T, A>;
    fn into_iter(self) -> IntoIter<T, A> {
        let len = self.len;
        let begin = self.begin;
        let end = self.end;
        let head = self.head;
        let reserved_groups = self.reserved_groups;
        core::mem::forget(self);
        unsafe { IntoIter::new(begin, end, len, head, reserved_groups) }
    }
}

impl<'a, T, A: Allocator> IntoIterator for &'a Hive<T, A> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T, A>;
    fn into_iter(self) -> Iter<'a, T, A> {
        self.iter()
    }
}

impl<'a, T, A: Allocator> IntoIterator for &'a mut Hive<T, A> {
    type Item = &'a mut T;
    type IntoIter = IterMut<'a, T, A>;
    fn into_iter(self) -> IterMut<'a, T, A> {
        self.iter_mut()
    }
}

// ── Internal: group ops ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    fn new_group_capacity(&self) -> u16 {
        self.len
            .max(self.min_block_capacity as usize)
            .min(self.max_block_capacity as usize) as u16
    }

    unsafe fn allocate_new_group(
        &mut self,
        capacity: u16,
        prev: Option<NonNull<Group<T, A>>>,
    ) -> NonNull<Group<T, A>> {
        let group = Group::allocate(capacity, prev, self.allocator.clone());
        self.capacity += capacity as usize;
        if let Some(p) = prev {
            (*p.as_ptr()).next = Some(group);
        }
        if self.head.is_none() {
            self.head = Some(group);
        }
        self.tail = Some(group);
        group
    }

    unsafe fn reuse_reserved_group(
        &mut self,
        prev: Option<NonNull<Group<T, A>>>,
    ) -> NonNull<Group<T, A>> {
        let group = self.reserved_groups.expect("no reserved groups");
        let gp = group.as_ptr();
        self.reserved_groups = (*gp).next;
        let gn = prev.map_or(0, |p| (*p.as_ptr()).group_number + 1);
        Group::reset(group, None, prev, gn);
        if let Some(p) = prev {
            (*p.as_ptr()).next = Some(group);
        }
        if self.head.is_none() {
            self.head = Some(group);
        }
        self.tail = Some(group);
        group
    }

    unsafe fn add_to_erasures_list(&mut self, group: NonNull<Group<T, A>>) {
        // All field access goes through the raw `*mut Group` to avoid creating
        // overlapping `&mut Group` borrows when helpers also touch the same
        // group. This is required for soundness under Stacked/Tree Borrows.
        let gp = group.as_ptr();
        let old_head = self.erasure_groups_head;
        (*gp).erasures_prev = None;
        (*gp).erasures_next = old_head;
        if let Some(h) = old_head {
            (*h.as_ptr()).erasures_prev = Some(group);
        }
        self.erasure_groups_head = Some(group);
    }

    unsafe fn remove_from_erasures_list(&mut self, group: NonNull<Group<T, A>>) {
        let gp = group.as_ptr();
        let prev = (*gp).erasures_prev;
        let next = (*gp).erasures_next;
        if let Some(p) = prev {
            (*p.as_ptr()).erasures_next = next;
        } else {
            self.erasure_groups_head = next;
        }
        if let Some(n) = next {
            (*n.as_ptr()).erasures_prev = prev;
        }
        (*gp).erasures_prev = None;
        (*gp).erasures_next = None;
    }

    unsafe fn move_to_reserved_list(&mut self, group: NonNull<Group<T, A>>) {
        let gp = group.as_ptr();
        let prev = (*gp).prev;
        let next = (*gp).next;
        let has_erasures = (*gp).erasures_prev.is_some()
            || (*gp).erasures_next.is_some()
            || self.erasure_groups_head == Some(group);

        if let Some(p) = prev {
            (*p.as_ptr()).next = next;
        } else {
            self.head = next;
        }
        if let Some(n) = next {
            (*n.as_ptr()).prev = prev;
        } else {
            self.tail = prev;
        }
        if has_erasures {
            self.remove_from_erasures_list(group);
        }
        (*gp).next = self.reserved_groups;
        (*gp).prev = None;
        (*gp).free_list_head = u16::MAX;
        self.reserved_groups = Some(group);
    }

    fn groups_fit_limits(&self, limits: BlockCapacityLimits) -> bool {
        let mut g = self.head;
        while let Some(group) = g {
            unsafe {
                let gp = group.as_ptr();
                if (*gp).capacity < limits.min || (*gp).capacity > limits.max {
                    return false;
                }
                g = (*gp).next;
            }
        }
        true
    }

    fn deallocate_reserved_outside_limits(&mut self, limits: BlockCapacityLimits) {
        let mut current = self.reserved_groups;
        let mut previous: Option<NonNull<Group<T, A>>> = None;

        while let Some(group) = current {
            unsafe {
                let gp = group.as_ptr();
                let next = (*gp).next;
                if (*gp).capacity < limits.min || (*gp).capacity > limits.max {
                    if let Some(prev) = previous {
                        (*prev.as_ptr()).next = next;
                    } else {
                        self.reserved_groups = next;
                    }
                    self.capacity = self.capacity.saturating_sub((*gp).capacity as usize);
                    Group::deallocate_group(group);
                } else {
                    previous = Some(group);
                }
                current = next;
            }
        }
    }

    fn compact_to_limits(&mut self, limits: BlockCapacityLimits) {
        let mut temp = Self::new_in(self.allocator.clone());
        temp.min_block_capacity = limits.min;
        temp.max_block_capacity = limits.max;
        temp.reserve(self.len);

        unsafe {
            let mut cur = self.begin;
            let count = self.len;
            for i in 0..count {
                let gp = cur.group.unwrap().as_ptr();
                let idx = (*gp).index_from_element_ptr(cur.element);
                temp.insert_raw((*gp).element_ptr(idx).read());
                if i + 1 < count {
                    cur = cur.advance_forward();
                }
            }
        }

        self.len = 0;
        let old = core::mem::replace(self, temp);
        old.deallocate_without_dropping_elements();
    }

    fn deallocate_without_dropping_elements(self) {
        let old = ManuallyDrop::new(self);
        unsafe {
            let mut g = old.head;
            while let Some(group) = g {
                let next = (*group.as_ptr()).next;
                Group::deallocate_group(group);
                g = next;
            }

            let mut g = old.reserved_groups;
            while let Some(group) = g {
                let next = (*group.as_ptr()).next;
                Group::deallocate_group(group);
                g = next;
            }
        }
    }
}

// ── Insert ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    /// Insert an element. Returns a stable raw pointer to it.
    pub fn insert(&mut self, value: T) -> *const T {
        self.insert_raw(value)
    }

    /// Insert an element and return a mutable raw pointer.
    pub fn insert_mut(&mut self, value: T) -> *mut T {
        self.insert_raw_mut(value)
    }

    /// Constructs an element from a closure and inserts it.
    ///
    /// This is the safe counterpart to [`Hive::insert_with_uninit`]. The closure
    /// returns a fully initialized value, so there is no external
    /// `MaybeUninit<T>` safety contract. The closure is evaluated before the
    /// hive reserves a slot, which keeps the hive unchanged if the closure
    /// panics.
    pub fn emplace<F>(&mut self, f: F) -> *const T
    where
        F: FnOnce() -> T,
    {
        self.insert(f())
    }

    /// Constructs an element from a closure, inserts it, and returns a mutable
    /// raw pointer.
    ///
    /// See [`Hive::emplace`].
    pub fn emplace_mut<F>(&mut self, f: F) -> *mut T
    where
        F: FnOnce() -> T,
    {
        self.insert_mut(f())
    }

    /// Inserts `T::default()`, then lets `f` mutate the initialized element in
    /// place before returning a stable pointer to it.
    ///
    /// Unlike [`Hive::insert_with_uninit`], this is safe because the closure
    /// receives an initialized `&mut T`. If the closure panics, the default value
    /// remains in the hive and will be dropped normally.
    pub fn insert_with<F>(&mut self, f: F) -> *const T
    where
        T: Default,
        F: FnOnce(&mut T),
    {
        self.insert_with_mut(f)
    }

    /// Inserts `T::default()`, then lets `f` mutate the initialized element in
    /// place before returning a stable mutable pointer to it.
    ///
    /// See [`Hive::insert_with`].
    pub fn insert_with_mut<F>(&mut self, f: F) -> *mut T
    where
        T: Default,
        F: FnOnce(&mut T),
    {
        let ptr = self.insert_default_mut();
        // SAFETY: `ptr` was just returned by this hive and points to a live,
        // initialized element. The mutable borrow is limited to this call.
        f(unsafe { &mut *ptr });
        ptr
    }

    fn insert_default_mut(&mut self) -> *mut T
    where
        T: Default,
    {
        unsafe {
            if let Some(eg) = self.erasure_groups_head {
                self.insert_default_reuse_erased(eg)
            } else if let Some(tail) = self.tail {
                if !(*tail.as_ptr()).is_full() {
                    self.insert_default_append_tail()
                } else {
                    self.insert_default_new_group()
                }
            } else {
                self.insert_default_first()
            }
        }
    }

    /// Constructs an element in-place in a hive slot.
    ///
    /// # Safety
    /// The closure must initialize the supplied slot exactly once. It must not
    /// read from the slot before initialization and must not unwind after
    /// initializing it. If the closure returns without initializing the slot,
    /// subsequent use of the hive is undefined behavior.
    pub unsafe fn insert_with_uninit<F>(&mut self, f: F) -> *const T
    where
        F: FnOnce(&mut MaybeUninit<T>),
    {
        self.insert_raw_mut_with(f)
    }

    /// Constructs an element in-place and returns a mutable raw pointer.
    ///
    /// # Safety
    /// See [`Hive::insert_with_uninit`].
    pub unsafe fn insert_with_uninit_mut<F>(&mut self, f: F) -> *mut T
    where
        F: FnOnce(&mut MaybeUninit<T>),
    {
        self.insert_raw_mut_with(f)
    }

    fn insert_raw(&mut self, value: T) -> *const T {
        self.insert_raw_mut(value)
    }

    fn insert_raw_mut(&mut self, value: T) -> *mut T {
        unsafe {
            self.insert_raw_mut_with(|slot| {
                slot.write(value);
            })
        }
    }

    unsafe fn insert_raw_mut_with<F>(&mut self, f: F) -> *mut T
    where
        F: FnOnce(&mut MaybeUninit<T>),
    {
        if let Some(eg) = self.erasure_groups_head {
            self.insert_reuse_erased_with(f, eg)
        } else if let Some(tail) = self.tail {
            if !(*tail.as_ptr()).is_full() {
                self.insert_append_tail_with(f)
            } else {
                self.insert_new_group_with(f)
            }
        } else {
            self.insert_first_with(f)
        }
    }

    unsafe fn insert_default_first(&mut self) -> *mut T
    where
        T: Default,
    {
        let pending = self.prepare_unlinked_group(None);
        let group = pending.as_ptr();
        let mut guard = PendingGroupInsertion::new(self, pending, None);
        let gp = group.as_ptr();
        let ptr = (*gp).element_ptr_mut(0);
        ptr.write(T::default());
        guard.commit_first();
        ptr
    }

    unsafe fn insert_default_append_tail(&mut self) -> *mut T
    where
        T: Default,
    {
        let mut end_cursor = self.end;
        let end_group = end_cursor.group.unwrap();
        let gp = end_group.as_ptr();
        let elem_byte = end_cursor.element as *mut u8;
        let ptr = elem_byte as *mut T;
        ptr.write(T::default());
        end_cursor.element = elem_byte.add((*gp).slot_size);
        end_cursor.skipfield = end_cursor.skipfield.add(1);
        self.end = end_cursor;
        (*gp).active_count += 1;
        self.len += 1;
        ptr
    }

    unsafe fn insert_default_reuse_erased(&mut self, erasure_group: NonNull<Group<T, A>>) -> *mut T
    where
        T: Default,
    {
        let gp = erasure_group.as_ptr();
        let slot_size = (*gp).slot_size;
        let elements_base = (*gp).elements_base();
        let skipfield_base = (*gp).skipfield_ptr();
        let index = (*gp).free_list_head;
        debug_assert_ne!(index, u16::MAX);
        let next_free = free_list::head_next::<T, A>(erasure_group);
        let new_elem_byte = elements_base.add(index as usize * slot_size);
        let ptr = new_elem_byte as *mut T;
        let new_sf = skipfield_base.add(index as usize);

        let begin = self.begin;
        let update_begin = begin
            .group
            .is_some_and(|bg| erasure_group == bg && (new_elem_byte as *const u8) < begin.element);

        ptr.write(T::default());
        free_list::pop_known_head::<T, A>(erasure_group, index, next_free);
        skipfield::mark_constructed(erasure_group, index);
        if (*gp).free_list_head == u16::MAX {
            self.remove_from_erasures_list(erasure_group);
        }
        (*gp).active_count += 1;
        self.len += 1;

        if update_begin {
            self.begin = make_cursor(erasure_group, new_elem_byte, new_sf);
        }

        ptr
    }

    unsafe fn insert_default_new_group(&mut self) -> *mut T
    where
        T: Default,
    {
        let prev = self.tail;
        let pending = self.prepare_unlinked_group(prev);
        let group = pending.as_ptr();
        let mut guard = PendingGroupInsertion::new(self, pending, prev);
        let gp = group.as_ptr();
        let ptr = (*gp).element_ptr_mut(0);
        ptr.write(T::default());
        guard.commit_new_group();
        ptr
    }

    unsafe fn prepare_unlinked_group(
        &mut self,
        prev: Option<NonNull<Group<T, A>>>,
    ) -> PendingGroup<T, A> {
        if let Some(group) = self.reserved_groups {
            let gp = group.as_ptr();
            self.reserved_groups = (*gp).next;
            let gn = prev.map_or(0, |p| (*p.as_ptr()).group_number + 1);
            (*gp).next = None;
            (*gp).prev = prev;
            (*gp).erasures_next = None;
            (*gp).erasures_prev = None;
            (*gp).free_list_head = u16::MAX;
            (*gp).active_count = 0;
            (*gp).group_number = gn;
            core::ptr::write_bytes((*gp).skipfield_mut(), 0, (*gp).capacity as usize);
            PendingGroup::Reserved {
                group,
                capacity_counted: self.head.is_some() || self.len == 0,
            }
        } else {
            PendingGroup::Allocated(Group::allocate(
                self.new_group_capacity(),
                prev,
                self.allocator.clone(),
            ))
        }
    }

    unsafe fn insert_first_with<F>(&mut self, f: F) -> *mut T
    where
        F: FnOnce(&mut MaybeUninit<T>),
    {
        let group = if self.reserved_groups.is_some() {
            self.reuse_reserved_group(None)
        } else {
            self.allocate_new_group(self.new_group_capacity(), None)
        };
        let gp = group.as_ptr();
        let ptr = (*gp).element_ptr_mut(0);
        f(&mut *(ptr as *mut MaybeUninit<T>));
        (*gp).active_count = 1;
        self.len = 1;
        self.begin = Self::begin_cursor_of(group);
        self.end = Self::end_cursor_of(group, 1);
        ptr
    }

    unsafe fn insert_append_tail_with<F>(&mut self, f: F) -> *mut T
    where
        F: FnOnce(&mut MaybeUninit<T>),
    {
        let mut end_cursor = self.end;
        let end_group = end_cursor.group.unwrap();
        let gp = end_group.as_ptr();
        let elem_byte = end_cursor.element as *mut u8;
        let ptr = elem_byte as *mut T;
        f(&mut *(ptr as *mut MaybeUninit<T>));
        end_cursor.element = elem_byte.add((*gp).slot_size);
        end_cursor.skipfield = end_cursor.skipfield.add(1);
        self.end = end_cursor;
        (*gp).active_count += 1;
        self.len += 1;
        ptr
    }

    unsafe fn insert_reuse_erased_with<F>(
        &mut self,
        f: F,
        erasure_group: NonNull<Group<T, A>>,
    ) -> *mut T
    where
        F: FnOnce(&mut MaybeUninit<T>),
    {
        // Access fields strictly through the raw `*mut Group` so we never hold
        // a Rust `&Group` / `&mut Group` borrow across nested helper calls that
        // reborrow the same group (e.g. `remove_from_erasures_list`).
        let gp = erasure_group.as_ptr();
        let slot_size = (*gp).slot_size;
        let elements_base = (*gp).elements_base();
        let skipfield_base = (*gp).skipfield_ptr();

        let index = free_list::pop_free_slot::<T, A>(erasure_group);
        let new_elem_byte = elements_base.add(index as usize * slot_size);
        let ptr = new_elem_byte as *mut T;
        let new_sf = skipfield_base.add(index as usize);

        let begin = self.begin;
        let update_begin = begin
            .group
            .is_some_and(|bg| erasure_group == bg && (new_elem_byte as *const u8) < begin.element);

        skipfield::mark_constructed(erasure_group, index);
        if (*gp).free_list_head == u16::MAX {
            self.remove_from_erasures_list(erasure_group);
        }
        f(&mut *(ptr as *mut MaybeUninit<T>));
        (*gp).active_count += 1;
        self.len += 1;

        if update_begin {
            self.begin = make_cursor(erasure_group, new_elem_byte, new_sf);
        }

        ptr
    }

    unsafe fn insert_new_group_with<F>(&mut self, f: F) -> *mut T
    where
        F: FnOnce(&mut MaybeUninit<T>),
    {
        let prev = self.tail;
        let cap = self.new_group_capacity();
        let group = if self.reserved_groups.is_some() {
            self.reuse_reserved_group(prev)
        } else {
            self.allocate_new_group(cap, prev)
        };
        let gp = group.as_ptr();
        let ptr = (*gp).element_ptr_mut(0);
        f(&mut *(ptr as *mut MaybeUninit<T>));
        (*gp).active_count = 1;
        self.end = Self::end_cursor_of(group, 1);
        self.len += 1;
        ptr
    }
}

enum PendingGroup<T, A: Allocator> {
    Allocated(NonNull<Group<T, A>>),
    Reserved {
        group: NonNull<Group<T, A>>,
        capacity_counted: bool,
    },
}

impl<T, A: Allocator> PendingGroup<T, A> {
    fn as_ptr(&self) -> NonNull<Group<T, A>> {
        match *self {
            Self::Allocated(group) | Self::Reserved { group, .. } => group,
        }
    }

    fn capacity_is_counted(&self) -> bool {
        matches!(
            *self,
            Self::Reserved {
                capacity_counted: true,
                ..
            }
        )
    }
}

struct PendingGroupInsertion<'a, T, A: Allocator + Clone> {
    hive: &'a mut Hive<T, A>,
    group: PendingGroup<T, A>,
    prev: Option<NonNull<Group<T, A>>>,
    committed: bool,
}

impl<'a, T, A: Allocator + Clone> PendingGroupInsertion<'a, T, A> {
    fn new(
        hive: &'a mut Hive<T, A>,
        group: PendingGroup<T, A>,
        prev: Option<NonNull<Group<T, A>>>,
    ) -> Self {
        Self {
            hive,
            group,
            prev,
            committed: false,
        }
    }

    unsafe fn commit_first(&mut self) {
        let group = self.group.as_ptr();
        let gp = group.as_ptr();
        debug_assert!(self.prev.is_none());
        (*gp).active_count = 1;
        if !self.group.capacity_is_counted() {
            self.hive.capacity += (*gp).capacity as usize;
        }
        self.hive.head = Some(group);
        self.hive.tail = Some(group);
        self.hive.len = 1;
        self.hive.begin = Hive::begin_cursor_of(group);
        self.hive.end = Hive::end_cursor_of(group, 1);
        self.committed = true;
    }

    unsafe fn commit_new_group(&mut self) {
        let group = self.group.as_ptr();
        let gp = group.as_ptr();
        let prev = self.prev;
        (*gp).active_count = 1;
        if !self.group.capacity_is_counted() {
            self.hive.capacity += (*gp).capacity as usize;
        }
        if let Some(p) = prev {
            (*p.as_ptr()).next = Some(group);
        }
        if self.hive.head.is_none() {
            self.hive.head = Some(group);
        }
        self.hive.tail = Some(group);
        self.hive.end = Hive::end_cursor_of(group, 1);
        self.hive.len += 1;
        self.committed = true;
    }
}

impl<T, A: Allocator + Clone> Drop for PendingGroupInsertion<'_, T, A> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }

        unsafe {
            match self.group {
                PendingGroup::Allocated(group) => Group::deallocate_group(group),
                PendingGroup::Reserved { group, .. } => {
                    let gp = group.as_ptr();
                    (*gp).next = self.hive.reserved_groups;
                    (*gp).prev = None;
                    (*gp).erasures_next = None;
                    (*gp).erasures_prev = None;
                    (*gp).free_list_head = u16::MAX;
                    (*gp).active_count = 0;
                    self.hive.reserved_groups = Some(group);
                }
            }
        }
    }
}

// ── Erase ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    /// Erase an element by raw pointer.
    ///
    /// Takes the element as a `*const T` rather than a Rust reference because
    /// the function destroys the element and reuses the slot's bytes for the
    /// per-group free-list. Passing `&T` or `&mut T` would have the borrow
    /// outlive the call and is undefined behavior under Stacked/Tree Borrows
    /// (the function-argument protector aliases memory we then overwrite via
    /// a different provenance chain).
    ///
    /// # Safety
    /// `element_ptr` must be a valid pointer previously returned by this hive
    /// (via `insert`, `insert_mut`, an iterator, etc.) to an element that has
    /// not already been erased. If the caller is holding an outstanding
    /// `&T`/`&mut T` to this element, they must drop or `mem::forget` it
    /// before calling `erase`.
    pub unsafe fn erase(&mut self, element_ptr: *const T) {
        self.erase_raw(element_ptr as *mut T);
    }

    unsafe fn erase_raw(&mut self, element_ptr: *mut T) {
        if cfg!(debug_assertions) {
            assert!(self.len > 0, "erase on empty hive");
        }
        let byte_ptr = element_ptr as *const u8;
        let group = self
            .find_group_for(byte_ptr)
            .expect("element not in any group");
        let gp = group.as_ptr();
        let index = (*gp).index_from_element_ptr(byte_ptr);
        debug_assert_eq!(
            *(*gp).skipfield_ptr().add(index as usize),
            0,
            "element already erased"
        );

        element_ptr.drop_in_place();
        self.len -= 1;
        let new_active = (*gp).active_count - 1;
        (*gp).active_count = new_active;

        if new_active > 0 {
            skipfield::mark_erased(group, index);
            let was_empty = (*gp).free_list_head == u16::MAX;
            free_list::push_free_slot::<T, A>(group, index);
            if was_empty {
                self.add_to_erasures_list(group);
            }
            let begin = self.begin;
            if Some(group) == begin.group && byte_ptr == begin.element {
                self.begin = begin.advance_forward();
            }
        } else {
            let begin = self.begin;
            let was_begin_group = Some(group) == begin.group;
            self.move_to_reserved_list(group);
            if was_begin_group {
                if self.len == 0 {
                    self.begin = null_cursor();
                    self.end = null_cursor();
                } else {
                    self.begin = Self::begin_cursor_of(self.head.unwrap());
                    let mut b = self.begin;
                    while unsafe { *b.skipfield != 0 } && self.len > 0 {
                        b = b.advance_forward();
                    }
                    self.begin = b;
                }
            }
        }
    }

    pub fn clear(&mut self) {
        if self.len == 0 {
            return;
        }
        unsafe {
            let mut cur = self.begin;
            let count = self.len;
            for i in 0..count {
                let gp = cur.group.unwrap().as_ptr();
                let idx = (*gp).index_from_element_ptr(cur.element);
                (*gp).element_ptr_mut(idx).drop_in_place();
                if i + 1 < count {
                    cur = cur.advance_forward();
                }
            }
        }
        let mut g = self.head;
        while let Some(group) = g {
            unsafe {
                let gp = group.as_ptr();
                let next = (*gp).next;
                (*gp).active_count = 0;
                (*gp).free_list_head = u16::MAX;
                (*gp).erasures_next = None;
                (*gp).erasures_prev = None;
                (*gp).next = self.reserved_groups;
                (*gp).prev = None;
                let cap = (*gp).capacity as usize;
                core::ptr::write_bytes((*gp).skipfield_mut(), 0, cap);
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
                let gp = cur.group.unwrap().as_ptr();
                let idx = (*gp).index_from_element_ptr(cur.element);
                let elem_ptr = (*gp).element_ptr_mut(idx);
                if !f(&*elem_ptr) {
                    to_erase.push(elem_ptr);
                }
                if i + 1 < count {
                    cur = cur.advance_forward();
                }
            }
        }
        for ptr in to_erase {
            unsafe {
                self.erase_raw(ptr);
            }
        }
    }
}

// ── Reserve / shrink ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    /// Ensures capacity for at least `additional` more elements.
    ///
    /// This intentionally follows Rust collection semantics (`Vec::reserve`),
    /// not C++ `std::hive::reserve(n)` total-capacity semantics.
    pub fn reserve(&mut self, additional: usize) {
        let needed = self.len.saturating_add(additional);
        if needed <= self.capacity {
            return;
        }
        let mut remaining = needed - self.capacity;
        while remaining > 0 {
            let cap = remaining
                .min(self.max_block_capacity as usize)
                .max(self.min_block_capacity as usize) as u16;
            let group = Group::allocate(cap, None, self.allocator.clone());
            self.capacity += cap as usize;
            unsafe {
                let gp = group.as_ptr();
                (*gp).active_count = 0;
                (*gp).next = self.reserved_groups;
            }
            self.reserved_groups = Some(group);
            remaining = remaining.saturating_sub(cap as usize);
        }
        if self.len == 0 && self.head.is_none() {
            let head = self.reserved_groups.expect("reserve allocated nothing");
            unsafe {
                let hp = head.as_ptr();
                let next = (*hp).next;
                self.reserved_groups = next;
                (*hp).next = None;
            }
            self.head = Some(head);
            self.tail = Some(head);
            self.begin = unsafe { Self::begin_cursor_of(head) };
            self.end = unsafe { Self::end_cursor_of(head, 0) };
        }
    }

    pub fn shrink_to_fit(&mut self) {
        if self.capacity == self.len {
            return;
        }
        if self.len == 0 {
            self.trim_capacity();
            if let Some(group) = self.head {
                unsafe {
                    self.capacity = self
                        .capacity
                        .saturating_sub((*group.as_ptr()).capacity as usize);
                    Group::deallocate_group(group);
                }
                self.head = None;
                self.tail = None;
                self.begin = null_cursor();
                self.end = null_cursor();
            }
            return;
        }

        self.compact_to_limits(self.block_capacity_limits());
    }

    pub fn reshape(
        &mut self,
        limits: BlockCapacityLimits,
    ) -> Result<(), InvalidBlockCapacityLimits> {
        Self::check_block_capacity_limits(limits)?;

        if self.len != 0 && !self.groups_fit_limits(limits) {
            self.compact_to_limits(limits);
        } else {
            self.deallocate_reserved_outside_limits(limits);
            if self.len == 0 {
                self.trim_capacity();
            }
            self.min_block_capacity = limits.min;
            self.max_block_capacity = limits.max;
        }

        Ok(())
    }

    pub fn trim_capacity(&mut self) {
        unsafe {
            while let Some(group) = self.reserved_groups {
                let gp = group.as_ptr();
                let next = (*gp).next;
                self.capacity = self.capacity.saturating_sub((*gp).capacity as usize);
                Group::deallocate_group(group);
                self.reserved_groups = next;
            }
        }
    }

    pub fn trim_capacity_to(&mut self, retain_capacity: usize) {
        if self.capacity <= retain_capacity || self.len >= retain_capacity {
            return;
        }

        let mut current = self.reserved_groups;
        let mut previous: Option<NonNull<Group<T, A>>> = None;

        while let Some(group) = current {
            if self.capacity <= retain_capacity {
                break;
            }

            unsafe {
                let gp = group.as_ptr();
                let next = (*gp).next;
                let group_capacity = (*gp).capacity as usize;
                if self.capacity.saturating_sub(group_capacity) >= retain_capacity {
                    if let Some(prev) = previous {
                        (*prev.as_ptr()).next = next;
                    } else {
                        self.reserved_groups = next;
                    }
                    self.capacity -= group_capacity;
                    Group::deallocate_group(group);
                } else {
                    previous = Some(group);
                }
                current = next;
            }
        }
    }
}

impl<T: Clone, A: Allocator + Clone> Hive<T, A> {
    pub fn assign(&mut self, len: usize, value: T) {
        self.clear();
        self.reserve(len);
        for _ in 0..len {
            self.insert_raw(value.clone());
        }
    }
}

impl<T, A: Allocator + Clone> Hive<T, A> {
    pub fn assign_from_iter<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.clear();
        self.extend(iter);
    }

    pub fn insert_many<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for item in iter {
            self.insert_raw(item);
        }
    }

    pub fn splice(&mut self, source: &mut Self) {
        if core::ptr::eq(self, source) || source.is_empty() {
            return;
        }

        let mut values = alloc::vec::Vec::with_capacity(source.len());
        unsafe {
            let mut cur = source.begin;
            let count = source.len;
            for i in 0..count {
                let gp = cur.group.unwrap().as_ptr();
                let idx = (*gp).index_from_element_ptr(cur.element);
                values.push((*gp).element_ptr(idx).read());
                if i + 1 < count {
                    cur = cur.advance_forward();
                }
            }
        }
        source.len = 0;
        source.clear_without_dropping_elements();
        self.extend(values);
    }

    fn clear_without_dropping_elements(&mut self) {
        let mut g = self.head;
        while let Some(group) = g {
            unsafe {
                let gp = group.as_ptr();
                let next = (*gp).next;
                (*gp).active_count = 0;
                (*gp).free_list_head = u16::MAX;
                (*gp).erasures_next = None;
                (*gp).erasures_prev = None;
                (*gp).next = self.reserved_groups;
                (*gp).prev = None;
                let cap = (*gp).capacity as usize;
                core::ptr::write_bytes((*gp).skipfield_mut(), 0, cap);
                self.reserved_groups = Some(group);
                g = next;
            }
        }
        self.head = None;
        self.tail = None;
        self.begin = null_cursor();
        self.end = null_cursor();
        self.erasure_groups_head = None;
    }
}

impl<T: PartialEq, A: Allocator + Clone> Hive<T, A> {
    pub fn unique(&mut self) -> usize {
        self.unique_by(|a, b| a == b)
    }
}

impl<T, A: Allocator + Clone> Hive<T, A> {
    pub fn unique_by<F>(&mut self, mut same_bucket: F) -> usize
    where
        F: FnMut(&T, &T) -> bool,
    {
        if self.len < 2 {
            return 0;
        }

        let mut to_erase: alloc::vec::Vec<*mut T> = alloc::vec::Vec::new();
        unsafe {
            let mut prev = self.begin;
            let mut cur = prev.advance_forward();
            let original_len = self.len;
            for i in 1..original_len {
                let pgp = prev.group.unwrap().as_ptr();
                let pidx = (*pgp).index_from_element_ptr(prev.element);
                let cgp = cur.group.unwrap().as_ptr();
                let cidx = (*cgp).index_from_element_ptr(cur.element);
                let current_ptr = (*cgp).element_ptr_mut(cidx);

                if same_bucket(&*(*pgp).element_ptr(pidx), &*current_ptr) {
                    to_erase.push(current_ptr);
                } else {
                    prev = cur;
                }

                if i + 1 < original_len {
                    cur = cur.advance_forward();
                }
            }
        }

        let removed = to_erase.len();
        for ptr in to_erase {
            unsafe {
                self.erase_raw(ptr);
            }
        }
        removed
    }
}

// ── Sort ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    pub fn sort_by<F: FnMut(&T, &T) -> core::cmp::Ordering>(&mut self, mut compare: F) {
        let count = self.len;
        if count <= 1 {
            return;
        }
        let mut v: alloc::vec::Vec<*mut T> = alloc::vec::Vec::with_capacity(count);
        unsafe {
            let mut cur = self.begin;
            for i in 0..count {
                let gp = cur.group.unwrap().as_ptr();
                let idx = (*gp).index_from_element_ptr(cur.element);
                v.push((*gp).element_ptr_mut(idx));
                if i + 1 < count {
                    cur = cur.advance_forward();
                }
            }
        }

        v.sort_by(|a, b| unsafe { compare(&**a, &**b) });

        let mut values: alloc::vec::Vec<T> = alloc::vec::Vec::with_capacity(count);
        for ptr in &v {
            unsafe {
                values.push(ptr.read());
            }
        }

        unsafe {
            let mut cur = self.begin;
            for (i, value) in values.into_iter().enumerate() {
                let gp = cur.group.unwrap().as_ptr();
                let idx = (*gp).index_from_element_ptr(cur.element);
                (*gp).element_ptr_mut(idx).write(value);
                if i + 1 < count {
                    cur = cur.advance_forward();
                }
            }
        }
    }
}

impl<T: Ord, A: Allocator + Clone> Hive<T, A> {
    pub fn sort(&mut self) {
        self.sort_by(|a, b| a.cmp(b))
    }
}

// ── Clone ──

impl<T: Clone, A: Allocator + Clone> Clone for Hive<T, A> {
    fn clone(&self) -> Self {
        let mut new = Self::new_in(self.allocator.clone());
        new.min_block_capacity = self.min_block_capacity;
        new.max_block_capacity = self.max_block_capacity;
        new.reserve(self.len);
        for item in self.iter() {
            new.insert(item.clone());
        }
        new
    }
}

// ── Collect / Extend ──

impl<T> FromIterator<T> for Hive<T, Global> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();
        let mut hive = Self::with_capacity(lower);
        for item in iter {
            hive.insert(item);
        }
        hive
    }
}

impl<T, A: Allocator + Clone> Extend<T> for Hive<T, A> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();
        self.reserve(lower);
        for item in iter {
            let _ = self.insert_raw(item);
        }
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
        let count = self.len;
        if count > 0 && needs_drop::<T>() {
            unsafe {
                let mut cur = self.begin;
                for i in 0..count {
                    let gp = cur.group.unwrap().as_ptr();
                    let idx = (*gp).index_from_element_ptr(cur.element);
                    (*gp).element_ptr_mut(idx).drop_in_place();
                    if i + 1 < count {
                        cur = cur.advance_forward();
                    }
                }
            }
        }
        let mut g = self.head;
        while let Some(group) = g {
            unsafe {
                let next = (*group.as_ptr()).next;
                Group::deallocate_group(group);
                g = next;
            }
        }
        let mut g = self.reserved_groups;
        while let Some(group) = g {
            unsafe {
                let next = (*group.as_ptr()).next;
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
