//! A bucket-based, unordered container with stable references and O(1) insertion/erasure.
//!
//! This module provides [`Hive<T>`], the core data structure of this crate, along
//! with the supporting types [`BlockCapacityLimits`] and
//! [`InvalidBlockCapacityLimits`].
//!
//! # Architecture
//!
//! `Hive` stores elements across multiple memory blocks (called *groups*). Each
//! block holds a contiguous array of element slots, a per-group skipfield (for
//! efficient O(1) forward/backward traversal over erased slots), and a
//! free-list for erased-slot reuse.
//!
//! When an element is erased, its slot is added to the free-list of its group
//! and the skipfield is updated so that iteration skips over it. When a new
//! element is inserted, the hive first checks for an erased slot; if one is
//! found it is reused immediately. Otherwise, the element is appended to the
//! tail group (allocating a new group if the tail is full).
//!
//! # Stable pointers
//!
//! Pointers returned by insertion methods remain valid until the element is
//! erased. Erasing or inserting other elements never moves existing ones. This
//! is the primary motivation for `Hive` over `Vec` and `VecDeque`.

use crate::allocator::{Allocator, Global};
use core::cell::Cell;
use core::marker::PhantomData;
use core::mem::{needs_drop, ManuallyDrop, MaybeUninit};
use core::ptr::NonNull;

#[cfg(feature = "pin-init")]
use pin_init::{Init, PinUninit};

use crate::free_list;
use crate::group::Group;
use crate::iter::{IntoIter, Iter, IterMut};
use crate::skipfield::Cursor;

const DEFAULT_MAX_BLOCK_CAPACITY: u16 = 8192;
const HARD_MIN_BLOCK_CAPACITY: u16 = 3;

/// Constraints on the size of internal memory blocks (groups).
///
/// Every group allocated by a [`Hive`] will have a capacity between `min` and
/// `max` (inclusive). The limits affect memory usage and allocation granularity.
///
/// Default limits are type-dependent. The hard lower bound is 3, and the upper
/// bound is determined by the skipfield index width for `T`.
///
/// Pass custom limits to [`Hive::try_new`] or [`Hive::try_new_in`].
///
/// # Examples
///
/// ```
/// use hive::{BlockCapacityLimits, Hive};
///
/// let mut hive: Hive<i32> = Hive::try_new(BlockCapacityLimits::new(4, 8192)).unwrap();
/// assert_eq!(hive.block_capacity_limits(), BlockCapacityLimits::new(4, 8192));
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockCapacityLimits {
    /// Minimum elements per block.
    pub min: u16,
    /// Maximum elements per block.
    pub max: u16,
}

/// Error returned when block capacity limits are invalid.
///
/// Limits must satisfy `hard_min <= min <= max <= hard_max` where
/// `hard_min = 3` and `hard_max` is type-dependent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidBlockCapacityLimits;

/// Error returned when [`Hive::splice`] cannot transfer source groups without
/// violating the destination hive's block capacity limits.
///
/// On error, both hives are left unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IncompatibleSplice;

impl BlockCapacityLimits {
    /// Creates new block capacity limits.
    ///
    /// This is a `const fn` and does **not** validate the values. Use
    /// [`Hive::try_new`] or [`Hive::try_new_in`] to check validity.
    pub const fn new(min: u16, max: u16) -> Self {
        Self { min, max }
    }
}

/// A bucket-based, unordered container with stable element addresses.
///
/// `Hive<T>` supports O(1) amortized insertion and erasure, immediate reuse of
/// erased slots, and bidirectional iteration. It is a Rust port of the C++
/// `plf::hive` container proposed as `std::hive` in [P0447](https://wg21.link/p0447).
///
/// # Type parameters
///
/// - `T` — the element type.
/// - `A` — the allocator type; defaults to [`Global`].
///
/// # Pointer stability
///
/// Raw pointers from [`insert`](Hive::insert) and
/// [`insert_mut`](Hive::insert_mut) remain valid until the element is erased or
/// an operation explicitly compacts/reallocates groups, such as
/// [`reshape`](Hive::reshape) or some [`shrink_to_fit`](Hive::shrink_to_fit)
/// calls. Inserting other elements never moves existing ones.
///
/// # Examples
///
/// ```
/// use hive::Hive;
///
/// let mut hive = Hive::new();
/// let a = hive.insert(10);
/// let b = hive.insert(20);
///
/// unsafe { hive.erase(b); }
/// let reused = hive.insert(30);
/// assert_eq!(reused, b); // erased slot reused immediately
/// ```
pub struct Hive<T, A: Allocator = Global> {
    head: Option<NonNull<Group<T, A>>>,
    tail: Option<NonNull<Group<T, A>>>,
    begin: Cursor<T, A>,
    end: Cursor<T, A>,
    erasure_groups_head: Option<NonNull<Group<T, A>>>,
    reserved_groups: Option<NonNull<Group<T, A>>>,
    lookup_hint: Cell<Option<NonNull<Group<T, A>>>>,
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
    skipfield: *const u8,
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
            g.skipfield_ptr_at(constructed as usize),
        )
    }
}

// ── Construction ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    /// Creates an empty hive using the given allocator.
    ///
    /// Default block capacity limits are used. To customize limits,
    /// use [`try_new_in`](Hive::try_new_in).
    pub fn new_in(allocator: A) -> Self {
        Self {
            head: None,
            tail: None,
            begin: null_cursor(),
            end: null_cursor(),
            erasure_groups_head: None,
            reserved_groups: None,
            lookup_hint: Cell::new(None),
            len: 0,
            capacity: 0,
            min_block_capacity: Self::default_min_block_capacity(),
            max_block_capacity: Self::default_max_block_capacity(),
            allocator,
        }
    }

    /// Creates an empty hive using the given allocator and custom block
    /// capacity limits.
    ///
    /// Returns [`InvalidBlockCapacityLimits`] if the limits are out of bounds
    /// (see [`BlockCapacityLimits`]).
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
    /// Creates an empty hive with the global allocator and default block
    /// capacity limits.
    pub fn new() -> Self {
        Self::new_in(Global)
    }

    /// Creates an empty hive with space pre-allocated for at least `cap`
    /// elements.
    ///
    /// Equivalent to constructing a new hive and calling
    /// [`reserve(cap)`](Hive::reserve).
    pub fn with_capacity(cap: usize) -> Self {
        let mut hive = Self::new();
        hive.reserve(cap);
        hive
    }

    /// Creates an empty hive with custom block capacity limits.
    ///
    /// Shortcut for `Self::try_new_in(Global, limits)`.
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
    /// Returns the number of live elements.
    pub fn len(&self) -> usize {
        self.len
    }
    /// Returns `true` if the hive contains no elements.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Returns the total number of element slots across all allocated and
    /// reserved groups.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    /// Returns the maximum possible element count for this hive.
    ///
    /// This is `usize::MAX / slot_size` and is effectively unbounded for
    /// practical element sizes.
    pub fn max_size(&self) -> usize {
        usize::MAX / Group::<T, A>::compute_slot_size()
    }
    /// Returns the current block capacity limits.
    pub fn block_capacity_limits(&self) -> BlockCapacityLimits {
        BlockCapacityLimits::new(self.min_block_capacity, self.max_block_capacity)
    }

    /// Returns the default block capacity limits.
    pub fn block_capacity_default_limits() -> BlockCapacityLimits {
        BlockCapacityLimits::new(
            Self::default_min_block_capacity(),
            Self::default_max_block_capacity(),
        )
    }

    /// Returns the hard bounds for block capacity limits.
    ///
    /// Minimum is 3 (required for the skipfield algorithm). Maximum depends on
    /// the skipfield index width selected for `T`.
    pub fn block_capacity_hard_limits() -> BlockCapacityLimits {
        BlockCapacityLimits::new(HARD_MIN_BLOCK_CAPACITY, Self::hard_max_block_capacity())
    }

    fn hard_max_block_capacity() -> u16 {
        Group::<T, A>::none_index()
    }

    fn default_max_block_capacity() -> u16 {
        DEFAULT_MAX_BLOCK_CAPACITY.min(Self::hard_max_block_capacity())
    }

    fn default_min_block_capacity() -> u16 {
        let slot_size = Group::<T, A>::compute_slot_size().max(1);
        let adaptive =
            ((core::mem::size_of::<Self>() + core::mem::size_of::<Group<T, A>>()) * 2) / slot_size;
        adaptive
            .max(8)
            .min(Self::default_max_block_capacity() as usize) as u16
    }

    /// Returns a reference to the allocator.
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
        if let Some(group) = self.lookup_hint.get() {
            let gp = group.as_ptr();
            let base = (*gp).elements_base();
            let end = base.add((*gp).capacity as usize * (*gp).slot_size);
            if element >= base && element < end {
                return Some(group);
            }

            let mut forward = (*gp).next;
            while let Some(group) = forward {
                let gp = group.as_ptr();
                let base = (*gp).elements_base();
                let end = base.add((*gp).capacity as usize * (*gp).slot_size);
                if element >= base && element < end {
                    self.lookup_hint.set(Some(group));
                    return Some(group);
                }
                forward = (*gp).next;
            }

            let mut backward = (*gp).prev;
            while let Some(group) = backward {
                let gp = group.as_ptr();
                let base = (*gp).elements_base();
                let end = base.add((*gp).capacity as usize * (*gp).slot_size);
                if element >= base && element < end {
                    self.lookup_hint.set(Some(group));
                    return Some(group);
                }
                backward = (*gp).prev;
            }
        }

        let mut g = self.head;
        while let Some(group) = g {
            if self.lookup_hint.get() == Some(group) {
                g = (*group.as_ptr()).next;
                continue;
            }

            let gp = group.as_ptr();
            let base = (*gp).elements_base();
            let end = base.add((*gp).capacity as usize * (*gp).slot_size);
            if element >= base && element < end {
                self.lookup_hint.set(Some(group));
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
        if element != byte_ptr || (*gp).skipfield_at(index as usize) != 0 {
            return None;
        }

        Some(make_cursor(
            group,
            element,
            (*gp).skipfield_ptr_at(index as usize),
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
    /// Returns an iterator over shared references to all live elements, in
    /// insertion order.
    pub fn iter(&self) -> Iter<'_, T, A> {
        unsafe { Iter::new(self.begin, self.end, self.len) }
    }
    /// Returns an iterator over mutable references to all live elements, in
    /// insertion order.
    pub fn iter_mut(&mut self) -> IterMut<'_, T, A> {
        unsafe { IterMut::new(self.begin, self.end, self.len) }
    }

    /// Returns a shared reference for a pointer previously returned by this
    /// hive.
    ///
    /// Returns `None` if the pointer points to an erased element, is outside
    /// any allocated group, or was never originated by this hive.
    ///
    /// # Safety
    ///
    /// `ptr` must be either null, a valid pointer returned by this hive for an
    /// element that has not been erased, or a pointer previously returned by
    /// this hive that has since been erased (which returns `None`). Passing an
    /// arbitrary foreign pointer may produce a garbage `Option` and may be
    /// undefined behavior due to pointer provenance and bounds checking limits.
    pub unsafe fn get(&self, ptr: *const T) -> Option<&T> {
        let cursor = unsafe { self.cursor_from_ptr(ptr)? };
        unsafe { Some(&*(cursor.element as *const T)) }
    }

    /// Returns a mutable reference for a pointer previously returned by this
    /// hive.
    ///
    /// Returns `None` under the same conditions as [`get`](Hive::get).
    ///
    /// # Safety
    ///
    /// See [`get`](Hive::get). The caller must also ensure there are no other
    /// `&T` or `&mut T` references to the same element for the duration of the
    /// returned mutable borrow.
    pub unsafe fn get_mut(&mut self, ptr: *const T) -> Option<&mut T> {
        let cursor = unsafe { self.cursor_from_ptr(ptr)? };
        unsafe { Some(&mut *(cursor.element as *mut T)) }
    }

    /// Returns an iterator over shared references beginning at `ptr`.
    ///
    /// Returns `None` if `ptr` is not a valid live element.
    ///
    /// # Safety
    ///
    /// See [`get`](Hive::get).
    pub unsafe fn iter_from(&self, ptr: *const T) -> Option<Iter<'_, T, A>> {
        let cursor = unsafe { self.cursor_from_ptr(ptr)? };
        let remaining = unsafe { self.count_from_cursor(cursor) };
        unsafe { Some(Iter::new(cursor, self.end, remaining)) }
    }

    /// Returns a mutable iterator beginning at `ptr`.
    ///
    /// Returns `None` if `ptr` is not a valid live element.
    ///
    /// # Safety
    ///
    /// See [`get_mut`](Hive::get_mut).
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

    unsafe fn append_erasures_list(&mut self, head: Option<NonNull<Group<T, A>>>) {
        let Some(source_head) = head else {
            return;
        };

        if let Some(dest_head) = self.erasure_groups_head {
            let mut tail = dest_head;
            while let Some(next) = (*tail.as_ptr()).erasures_next {
                tail = next;
            }
            (*tail.as_ptr()).erasures_next = Some(source_head);
            (*source_head.as_ptr()).erasures_prev = Some(tail);
        } else {
            self.erasure_groups_head = Some(source_head);
            (*source_head.as_ptr()).erasures_prev = None;
        }
    }

    unsafe fn mark_tail_unused_as_erased(&mut self) {
        let Some(tail) = self.tail else {
            return;
        };
        let gp = tail.as_ptr();
        let end_index = (*gp).index_from_element_ptr(self.end.element);
        let distance = (*gp).capacity - end_index;
        if distance == 0 {
            return;
        }

        let idx = end_index as usize;
        let previous_erased = end_index > 0 && (*gp).skipfield_at(idx - 1) != 0;
        if previous_erased {
            let previous_len = (*gp).skipfield_at(idx - 1);
            let new_len = previous_len + distance;
            let start = idx - previous_len as usize;
            let end = (*gp).capacity as usize - 1;
            (*gp).write_skipfield_at(start, new_len);
            (*gp).write_skipfield_at(end, new_len);
            if distance > 1 {
                for i in idx..end {
                    (*gp).write_skipfield_at(i, 1);
                }
            }
        } else {
            let mut added_to_erasures = false;
            for index in end_index..(*gp).capacity {
                added_to_erasures |= free_list::mark_erased(tail, index);
            }
            if added_to_erasures {
                self.add_to_erasures_list(tail);
            }
        }
    }

    fn active_capacity(&self) -> usize {
        let mut capacity = 0;
        let mut g = self.head;
        while let Some(group) = g {
            unsafe {
                let gp = group.as_ptr();
                capacity += (*gp).capacity as usize;
                g = (*gp).next;
            }
        }
        capacity
    }

    unsafe fn move_to_reserved_list(&mut self, group: NonNull<Group<T, A>>) {
        if self.lookup_hint.get() == Some(group) {
            self.lookup_hint.set(None);
        }
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
        (*gp).free_list_head = Group::<T, A>::none_index();
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
        self.lookup_hint.set(None);
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
    /// Inserts an element and returns a stable raw pointer to it.
    ///
    /// The pointer remains valid until the element is erased via
    /// [`erase`](Hive::erase), [`retain`](Hive::retain), or until an operation
    /// explicitly compacts/reallocates groups, such as [`reshape`](Hive::reshape)
    /// or some [`shrink_to_fit`](Hive::shrink_to_fit) calls. Other insertions
    /// and erasures never move this element.
    ///
    /// # Examples
    ///
    /// ```
    /// use hive::Hive;
    ///
    /// let mut hive = Hive::new();
    /// let p = hive.insert(42);
    /// unsafe { assert_eq!(*p, 42); }
    /// ```
    pub fn insert(&mut self, value: T) -> *const T {
        self.insert_raw(value)
    }

    /// Inserts an element and returns a stable mutable raw pointer to it.
    ///
    /// See [`insert`](Hive::insert).
    pub fn insert_mut(&mut self, value: T) -> *mut T {
        self.insert_raw_mut(value)
    }

    /// Constructs an element from a closure and inserts it.
    ///
    /// This is the safe counterpart to [`insert_with_uninit`](Hive::insert_with_uninit).
    /// The closure returns a fully initialized `T`, so there is no
    /// `MaybeUninit<T>` safety contract. The closure is evaluated before a
    /// hive slot is reserved — if it panics, the hive is unchanged.
    pub fn emplace<F>(&mut self, f: F) -> *const T
    where
        F: FnOnce() -> T,
    {
        self.insert(f())
    }

    /// Constructs an element from a closure, inserts it, and returns a mutable
    /// raw pointer.
    ///
    /// See [`emplace`](Hive::emplace).
    pub fn emplace_mut<F>(&mut self, f: F) -> *mut T
    where
        F: FnOnce() -> T,
    {
        self.insert_mut(f())
    }

    /// Inserts `T::default()`, then calls `f` on the initialized element before
    /// returning a stable pointer.
    ///
    /// Unlike [`insert_with_uninit`](Hive::insert_with_uninit), this method is
    /// safe: `f` receives an initialized `&mut T`. If `f` panics, the default
    /// value remains in the hive and will be dropped when the hive is.
    pub fn insert_with<F>(&mut self, f: F) -> *const T
    where
        T: Default,
        F: FnOnce(&mut T),
    {
        self.insert_with_mut(f)
    }

    /// Inserts `T::default()`, then calls `f` on the initialized element before
    /// returning a stable mutable pointer.
    ///
    /// See [`insert_with`](Hive::insert_with).
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
    /// The closure receives a `&mut MaybeUninit<T>` pointing to uninitialized
    /// memory inside a hive slot. It must initialize the slot exactly once and
    /// must not unwind. The returned pointer is stable until the element is
    /// erased.
    ///
    /// If you need panic/error-safe in-place initialization, enable the
    /// `pin-init` feature and use [`insert_pin_init`](Hive::insert_pin_init) or
    /// [`insert_pin_init_mut`](Hive::insert_pin_init_mut) instead.
    ///
    /// # Safety
    ///
    /// The closure must:
    /// - Initialize the supplied `MaybeUninit<T>` exactly once (via
    ///   `MaybeUninit::write` or equivalent).
    /// - **Not** read from the slot before initialization.
    /// - **Not** unwind (panic), before or after initializing the slot. The
    ///   hive may reserve or relink internal storage before invoking the
    ///   closure, so unwinding from the closure may leave internal bookkeeping
    ///   inconsistent.
    pub unsafe fn insert_with_uninit<F>(&mut self, f: F) -> *const T
    where
        F: FnOnce(&mut MaybeUninit<T>),
    {
        self.insert_raw_mut_with(f)
    }

    /// Constructs an element in-place and returns a mutable raw pointer.
    ///
    /// # Safety
    ///
    /// See [`insert_with_uninit`](Hive::insert_with_uninit).
    pub unsafe fn insert_with_uninit_mut<F>(&mut self, f: F) -> *mut T
    where
        F: FnOnce(&mut MaybeUninit<T>),
    {
        self.insert_raw_mut_with(f)
    }

    /// Pin-initializes an element in-place in a hive slot.
    ///
    /// The initializer receives a pinned, uninitialized slot through
    /// [`pin_init::PinUninit`]. The element is considered inserted only if the
    /// initializer succeeds; on error, the hive remains unchanged apart from any
    /// allocation that may be retained for later reuse.
    ///
    /// The returned pointer is stable until the element is erased.
    #[cfg(feature = "pin-init")]
    pub fn insert_pin_init<I, E>(&mut self, init: I) -> Result<*const T, E>
    where
        I: Init<T, E>,
    {
        self.insert_pin_init_mut(init).map(|ptr| ptr as *const T)
    }

    /// Pin-initializes an element in-place and returns a mutable raw pointer.
    ///
    /// See [`insert_pin_init`](Hive::insert_pin_init).
    #[cfg(feature = "pin-init")]
    pub fn insert_pin_init_mut<I, E>(&mut self, init: I) -> Result<*mut T, E>
    where
        I: Init<T, E>,
    {
        unsafe { self.insert_pin_init_raw_mut(init) }
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

    #[cfg(feature = "pin-init")]
    unsafe fn insert_pin_init_raw_mut<I, E>(&mut self, init: I) -> Result<*mut T, E>
    where
        I: Init<T, E>,
    {
        if let Some(eg) = self.erasure_groups_head {
            self.insert_pin_init_reuse_erased(init, eg)
        } else if let Some(tail) = self.tail {
            if !(*tail.as_ptr()).is_full() {
                self.insert_pin_init_append_tail(init)
            } else {
                self.insert_pin_init_new_group(init)
            }
        } else {
            self.insert_pin_init_first(init)
        }
    }

    #[cfg(feature = "pin-init")]
    unsafe fn pin_init_slot<I, E>(ptr: *mut T, init: I) -> Result<(), E>
    where
        I: Init<T, E>,
    {
        let slot = &mut *(ptr as *mut MaybeUninit<T>);
        match init.__init(PinUninit::new(slot)) {
            Ok(_) => Ok(()),
            Err(err) => Err(err.into_inner()),
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
        end_cursor.skipfield = end_cursor.skipfield.add(Group::<T, A>::index_size());
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
        let index = (*gp).free_list_head;
        debug_assert_ne!(index, Group::<T, A>::none_index());
        let next_free = free_list::head_next::<T, A>(erasure_group);
        let new_elem_byte = elements_base.add(index as usize * slot_size);
        let ptr = new_elem_byte as *mut T;
        let new_sf = (*gp).skipfield_ptr_at(index as usize);

        let begin = self.begin;
        let update_begin = begin
            .group
            .is_some_and(|bg| erasure_group == bg && (new_elem_byte as *const u8) < begin.element);

        ptr.write(T::default());
        if free_list::consume_head_skipblock_with_next::<T, A>(erasure_group, index, next_free) {
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
            (*gp).free_list_head = Group::<T, A>::none_index();
            (*gp).active_count = 0;
            (*gp).group_number = gn;
            core::ptr::write_bytes(
                (*gp).skipfield_mut(),
                0,
                (*gp).capacity as usize * Group::<T, A>::index_size(),
            );
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

    #[cfg(feature = "pin-init")]
    unsafe fn insert_pin_init_first<I, E>(&mut self, init: I) -> Result<*mut T, E>
    where
        I: Init<T, E>,
    {
        let pending = self.prepare_unlinked_group(None);
        let group = pending.as_ptr();
        let mut guard = PendingGroupInsertion::new(self, pending, None);
        let ptr = (*group.as_ptr()).element_ptr_mut(0);
        Self::pin_init_slot(ptr, init)?;
        guard.commit_first();
        Ok(ptr)
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
        end_cursor.skipfield = end_cursor.skipfield.add(Group::<T, A>::index_size());
        self.end = end_cursor;
        (*gp).active_count += 1;
        self.len += 1;
        ptr
    }

    #[cfg(feature = "pin-init")]
    unsafe fn insert_pin_init_append_tail<I, E>(&mut self, init: I) -> Result<*mut T, E>
    where
        I: Init<T, E>,
    {
        let mut end_cursor = self.end;
        let end_group = end_cursor.group.unwrap();
        let gp = end_group.as_ptr();
        let elem_byte = end_cursor.element as *mut u8;
        let ptr = elem_byte as *mut T;

        Self::pin_init_slot(ptr, init)?;
        end_cursor.element = elem_byte.add((*gp).slot_size);
        end_cursor.skipfield = end_cursor.skipfield.add(Group::<T, A>::index_size());
        self.end = end_cursor;
        (*gp).active_count += 1;
        self.len += 1;
        Ok(ptr)
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
        let index = (*gp).free_list_head;
        debug_assert_ne!(index, Group::<T, A>::none_index());
        let new_elem_byte = elements_base.add(index as usize * slot_size);
        let ptr = new_elem_byte as *mut T;
        let new_sf = (*gp).skipfield_ptr_at(index as usize);

        let begin = self.begin;
        let update_begin = begin
            .group
            .is_some_and(|bg| erasure_group == bg && (new_elem_byte as *const u8) < begin.element);

        if free_list::consume_head_skipblock::<T, A>(erasure_group, index) {
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

    #[cfg(feature = "pin-init")]
    unsafe fn insert_pin_init_reuse_erased<I, E>(
        &mut self,
        init: I,
        erasure_group: NonNull<Group<T, A>>,
    ) -> Result<*mut T, E>
    where
        I: Init<T, E>,
    {
        let gp = erasure_group.as_ptr();
        let slot_size = (*gp).slot_size;
        let elements_base = (*gp).elements_base();
        let index = (*gp).free_list_head;
        debug_assert_ne!(index, Group::<T, A>::none_index());
        let next_free = free_list::head_next::<T, A>(erasure_group);
        let new_elem_byte = elements_base.add(index as usize * slot_size);
        let ptr = new_elem_byte as *mut T;
        let new_sf = (*gp).skipfield_ptr_at(index as usize);

        Self::pin_init_slot(ptr, init)?;

        let begin = self.begin;
        let update_begin = begin
            .group
            .is_some_and(|bg| erasure_group == bg && (new_elem_byte as *const u8) < begin.element);

        if free_list::consume_head_skipblock_with_next::<T, A>(erasure_group, index, next_free) {
            self.remove_from_erasures_list(erasure_group);
        }
        (*gp).active_count += 1;
        self.len += 1;

        if update_begin {
            self.begin = make_cursor(erasure_group, new_elem_byte, new_sf);
        }

        Ok(ptr)
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

    #[cfg(feature = "pin-init")]
    unsafe fn insert_pin_init_new_group<I, E>(&mut self, init: I) -> Result<*mut T, E>
    where
        I: Init<T, E>,
    {
        let prev = self.tail;
        let pending = self.prepare_unlinked_group(prev);
        let group = pending.as_ptr();
        let mut guard = PendingGroupInsertion::new(self, pending, prev);
        let ptr = (*group.as_ptr()).element_ptr_mut(0);
        Self::pin_init_slot(ptr, init)?;
        guard.commit_new_group();
        Ok(ptr)
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
                    (*gp).free_list_head = Group::<T, A>::none_index();
                    (*gp).active_count = 0;
                    self.hive.reserved_groups = Some(group);
                }
            }
        }
    }
}

// ── Erase ──

impl<T, A: Allocator + Clone> Hive<T, A> {
    /// Erases an element identified by the raw pointer returned from a prior
    /// insertion.
    ///
    /// The element is dropped and its slot is made available for reuse by the
    /// next insertion. The skipfield is updated so that iteration skips the
    /// erased slot.
    ///
    /// The function takes `*const T` rather than `&T` or `&mut T`. This is
    /// intentional: erasure destroys the element and overwrites slot memory for
    /// the per-group free-list. If a Rust reference were passed, the aliasing
    /// rules (Stacked/Tree Borrows) would consider the argument's provenance
    /// still-active after the call, violating the model when the slot is reused
    /// through a different provenance path.
    ///
    /// # Safety
    ///
    /// `element_ptr` must be a valid pointer previously returned by this hive
    /// (via [`insert`](Hive::insert), [`insert_mut`](Hive::insert_mut), an
    /// iterator, [`get`](Hive::get)/[`get_mut`](Hive::get_mut), etc.) to an
    /// element that has not already been erased. The caller must drop or
    /// [`core::mem::forget`] any outstanding `&T` or `&mut T` references to the
    /// element before calling `erase`.
    ///
    /// # Panics (debug)
    ///
    /// In debug builds, panics if the hive is empty, the pointer does not
    /// belong to any group, or the element is already erased.
    ///
    /// # Panic safety
    ///
    /// If `T::drop` panics while erasing the element, the hive may still mark
    /// that slot as live. Do not rely on recovering and continuing to use the
    /// hive after an element destructor panics.
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
            (*gp).skipfield_at(index as usize),
            0,
            "element already erased"
        );

        element_ptr.drop_in_place();
        self.len -= 1;
        let new_active = (*gp).active_count - 1;
        (*gp).active_count = new_active;

        if new_active > 0 {
            if free_list::mark_erased::<T, A>(group, index) {
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
                    while unsafe { (*b.group.unwrap().as_ptr()).read_index_at(b.skipfield) != 0 }
                        && self.len > 0
                    {
                        b = b.advance_forward();
                    }
                    self.begin = b;
                }
            }
        }
    }

    /// Removes all elements from the hive.
    ///
    /// All elements are dropped. Memory blocks are moved to the reserved pool
    /// and may be reused on subsequent insertions. To release the reserved
    /// blocks as well, call [`trim_capacity`](Hive::trim_capacity) or
    /// [`shrink_to_fit`](Hive::shrink_to_fit) afterwards.
    ///
    /// # Panic safety
    ///
    /// If an element destructor panics, the hive may still mark already-dropped
    /// slots as live. Do not rely on recovering and continuing to use the hive
    /// after `T::drop` panics during `clear`.
    ///
    /// # Examples
    ///
    /// ```
    /// use hive::Hive;
    ///
    /// let mut hive = Hive::new();
    /// hive.insert(1);
    /// hive.insert(2);
    /// hive.clear();
    /// assert!(hive.is_empty());
    /// assert_eq!(hive.len(), 0);
    /// ```
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
                (*gp).free_list_head = Group::<T, A>::none_index();
                (*gp).erasures_next = None;
                (*gp).erasures_prev = None;
                (*gp).next = self.reserved_groups;
                (*gp).prev = None;
                let cap = (*gp).capacity as usize;
                core::ptr::write_bytes((*gp).skipfield_mut(), 0, cap * Group::<T, A>::index_size());
                self.reserved_groups = Some(group);
                g = next;
            }
        }
        self.head = None;
        self.tail = None;
        self.lookup_hint.set(None);
        self.begin = null_cursor();
        self.end = null_cursor();
        self.erasure_groups_head = None;
        self.len = 0;
    }

    /// Keeps only the elements for which `f` returns `true`.
    ///
    /// All other elements are erased. This is an O(n) operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use hive::Hive;
    ///
    /// let mut hive = Hive::new();
    /// hive.insert(1);
    /// hive.insert(2);
    /// hive.insert(3);
    /// hive.retain(|&x| x % 2 == 0);
    /// assert_eq!(hive.len(), 1);
    /// ```
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
    /// This intentionally follows Rust collection semantics (`Vec::reserve`), not
    /// the C++ total-capacity semantics of `std::hive::reserve(n)`.
    ///
    /// If `additional` elements cannot fit within existing capacity, new groups
    /// are allocated up to `max_block_capacity` per group and placed on the
    /// reserved list.
    ///
    /// # Examples
    ///
    /// ```
    /// use hive::Hive;
    ///
    /// let mut hive: Hive<i32> = Hive::new();
    /// hive.reserve(1000);
    /// assert!(hive.capacity() >= 1000);
    /// ```
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
            self.lookup_hint.set(None);
            self.begin = unsafe { Self::begin_cursor_of(head) };
            self.end = unsafe { Self::end_cursor_of(head, 0) };
        }
    }

    /// Shrinks the hive to use only the minimum capacity needed for its current
    /// elements.
    ///
    /// Reserved groups are deallocated first. If all live groups already fit
    /// the current block capacity limits, live elements are not moved and
    /// existing element pointers remain valid. Otherwise, elements are compacted
    /// into new groups that match the current block capacity limits, which
    /// invalidates all existing element pointers. This is an O(n) operation.
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
                self.lookup_hint.set(None);
                self.begin = null_cursor();
                self.end = null_cursor();
            }
            return;
        }

        let limits = self.block_capacity_limits();
        if self.groups_fit_limits(limits) {
            self.trim_capacity();
        } else {
            self.compact_to_limits(limits);
        }
    }

    /// Updates block capacity limits, compacting existing elements if
    /// necessary.
    ///
    /// If the new limits exclude some existing groups, elements are moved into
    /// new groups that satisfy the limits. Reserved groups outside the new
    /// limits are deallocated. Element stability **is not** preserved during
    /// compaction — all existing pointers are invalidated.
    ///
    /// Returns [`InvalidBlockCapacityLimits`] if the limits are invalid.
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

    /// Deallocates all reserved (empty) groups, reducing capacity to the
    /// total capacity of groups currently holding live elements.
    ///
    /// This does not affect live elements or compact them. The hive can still
    /// grow afterward.
    pub fn trim_capacity(&mut self) {
        self.lookup_hint.set(None);
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

    /// Deallocates reserved groups until total capacity drops to
    /// `retain_capacity` or can no longer be reduced without affecting live
    /// elements.
    ///
    /// Live element capacity is never touched. If there are enough reserved
    /// groups, total capacity is reduced to the given target; otherwise, all
    /// reserved groups are freed and capacity is left at the live-group total.
    pub fn trim_capacity_to(&mut self, retain_capacity: usize) {
        self.lookup_hint.set(None);
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
    /// Clears the hive and fills it with `len` copies of `value`.
    ///
    /// This is an O(len) operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use hive::Hive;
    ///
    /// let mut hive = Hive::new();
    /// hive.assign(3, 7);
    /// assert_eq!(hive.len(), 3);
    /// ```
    pub fn assign(&mut self, len: usize, value: T) {
        self.clear();
        self.reserve(len);
        for _ in 0..len {
            self.insert_raw(value.clone());
        }
    }
}

impl<T, A: Allocator + Clone> Hive<T, A> {
    /// Clears the hive and replaces its contents with elements from an iterator.
    ///
    /// Equivalent to `clear()` followed by `extend(iter)`, but avoids
    /// double-buffering.
    pub fn assign_from_iter<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.clear();
        self.extend(iter);
    }

    /// Inserts all elements from an iterator without clearing existing
    /// contents.
    ///
    /// Unlike [`extend`](Hive::extend), this does not pre-reserve capacity based
    /// on the iterator's size hint. It is equivalent to repeated
    /// [`insert`](Hive::insert) calls.
    pub fn insert_many<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for item in iter {
            self.insert_raw(item);
        }
    }

    /// Splices all elements from `source` into `self` by transferring whole
    /// groups.
    ///
    /// On success, existing element pointers into both hives remain valid and
    /// now refer to elements in `self`. `source` is left empty, except for any
    /// reserved capacity it already held.
    ///
    /// Self-splicing (passing `self` as `source`) is a no-op and returns
    /// `Ok(())`.
    ///
    /// Returns [`IncompatibleSplice`] if any active source group is outside the
    /// destination hive's current block capacity limits. On error, both hives
    /// are left unchanged.
    pub fn splice(&mut self, source: &mut Self) -> Result<(), IncompatibleSplice> {
        if core::ptr::eq(self, source) || source.is_empty() {
            return Ok(());
        }

        let limits = self.block_capacity_limits();
        let mut group = source.head;
        while let Some(g) = group {
            unsafe {
                let gp = g.as_ptr();
                if (*gp).capacity < limits.min || (*gp).capacity > limits.max {
                    return Err(IncompatibleSplice);
                }
                group = (*gp).next;
            }
        }

        unsafe {
            let source_head = source.head.unwrap();
            let source_tail = source.tail.unwrap();
            let source_begin = source.begin;
            let source_end = source.end;
            let source_erasure_groups_head = source.erasure_groups_head;
            let source_len = source.len;
            let source_reserved = source.reserved_groups;
            let source_active_capacity = source.active_capacity();

            source.head = None;
            source.tail = None;
            source.begin = null_cursor();
            source.end = null_cursor();
            source.reserved_groups = None;
            source.lookup_hint.set(None);

            if self.len == 0 {
                if let Some(group) = self.head {
                    let gp = group.as_ptr();
                    self.capacity = self.capacity.saturating_sub((*gp).capacity as usize);
                    Group::deallocate_group(group);
                }
                self.head = Some(source_head);
                self.tail = Some(source_tail);
                self.begin = source_begin;
                self.end = source_end;
                self.erasure_groups_head = source_erasure_groups_head;
                self.len = source_len;
                self.capacity = self.capacity.saturating_add(source_active_capacity);
            } else {
                self.mark_tail_unused_as_erased();
                let tail = self.tail.unwrap();
                (*tail.as_ptr()).next = Some(source_head);
                (*source_head.as_ptr()).prev = Some(tail);
                self.append_erasures_list(source_erasure_groups_head);
                self.tail = Some(source_tail);
                self.end = source_end;
                self.len += source_len;
                self.capacity += source_active_capacity;
            }

            source.len = 0;
            source.erasure_groups_head = None;
            source.reserved_groups = source_reserved;
            source.capacity = 0;
            let mut reserved = source.reserved_groups;
            while let Some(group) = reserved {
                let gp = group.as_ptr();
                source.capacity += (*gp).capacity as usize;
                reserved = (*gp).next;
            }
            self.lookup_hint.set(None);
            source.lookup_hint.set(None);
        }

        Ok(())
    }
}

impl<T: PartialEq, A: Allocator + Clone> Hive<T, A> {
    /// Removes consecutive duplicate elements (as determined by `==`).
    ///
    /// Returns the number of elements erased. The hive **must** be sorted (see
    /// [`sort`](Hive::sort)) before calling `unique`, or behavior is
    /// unspecified.
    ///
    /// # Examples
    ///
    /// ```
    /// use hive::Hive;
    ///
    /// let mut hive = Hive::new();
    /// hive.extend([1, 1, 2, 3, 3]);
    /// hive.sort();
    /// let removed = hive.unique();
    /// assert_eq!(removed, 2);
    /// assert_eq!(hive.len(), 3);
    /// ```
    pub fn unique(&mut self) -> usize {
        self.unique_by(|a, b| a == b)
    }
}

impl<T, A: Allocator + Clone> Hive<T, A> {
    /// Removes consecutive duplicates using a custom equality predicate.
    ///
    /// Returns the number of elements erased. Like [`unique`](Hive::unique),
    /// this requires a sorted hive.
    ///
    /// # Examples
    ///
    /// ```
    /// use hive::Hive;
    ///
    /// let mut hive = Hive::new();
    /// hive.extend(["a", "A", "b"]);
    /// hive.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    /// let removed = hive.unique_by(|a, b| a.eq_ignore_ascii_case(b));
    /// assert_eq!(removed, 1);
    /// ```
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
    /// Sorts the elements in place using a custom comparator.
    ///
    /// Element addresses are preserved — elements are not moved in memory,
    /// only their values are rearranged. This is an O(n log n) operation.
    ///
    /// # Panic safety
    ///
    /// If the comparator panics, the hive remains valid and destructible, but
    /// the order of elements is unspecified.
    ///
    /// # Examples
    ///
    /// ```
    /// use hive::Hive;
    ///
    /// let mut hive = Hive::new();
    /// hive.extend([3, 1, 2]);
    /// hive.sort_by(|a, b| a.cmp(b));
    /// let v: Vec<_> = hive.iter().copied().collect();
    /// assert_eq!(v, vec![1, 2, 3]);
    /// ```
    pub fn sort_by<F: FnMut(&T, &T) -> core::cmp::Ordering>(&mut self, mut compare: F) {
        let count = self.len;
        if count <= 1 {
            return;
        }
        let mut v: alloc::vec::Vec<(*mut T, usize)> = alloc::vec::Vec::with_capacity(count);
        unsafe {
            let mut cur = self.begin;
            for i in 0..count {
                let gp = cur.group.unwrap().as_ptr();
                let idx = (*gp).index_from_element_ptr(cur.element);
                v.push(((*gp).element_ptr_mut(idx), i));
                if i + 1 < count {
                    cur = cur.advance_forward();
                }
            }
        }

        v.sort_by(|a, b| unsafe { compare(&*a.0, &*b.0) });

        for i in 0..count {
            let current = i;
            while v[current].1 != current {
                let source = v[current].1;
                unsafe {
                    core::ptr::swap(v[current].0, v[source].0);
                }
                v.swap(current, source);
            }
        }
    }
}

impl<T: Ord, A: Allocator + Clone> Hive<T, A> {
    /// Sorts the elements in place using the natural ordering of `T`.
    ///
    /// Element addresses are preserved. See [`sort_by`](Hive::sort_by).
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
