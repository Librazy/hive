//! Inline free list stored in erased element memory slots.

use core::alloc::Allocator;
use core::ptr::NonNull;
use crate::group::Group;

unsafe fn slot_addr(element_base: *mut u8, slot_size: usize, index: u16) -> *mut u8 {
    unsafe { element_base.add(index as usize * slot_size) }
}

unsafe fn slot_addr_const(element_base: *const u8, slot_size: usize, index: u16) -> *const u8 {
    unsafe { element_base.add(index as usize * slot_size) }
}

unsafe fn read_next(element_base: *const u8, slot_size: usize, index: u16) -> u16 {
    unsafe { *((slot_addr_const(element_base, slot_size, index) as *const u16).add(1)) }
}

unsafe fn write_prev(element_base: *mut u8, slot_size: usize, index: u16, prev: u16) {
    unsafe { *(slot_addr(element_base, slot_size, index) as *mut u16) = prev; }
}

unsafe fn write_next(element_base: *mut u8, slot_size: usize, index: u16, next: u16) {
    unsafe { *((slot_addr(element_base, slot_size, index) as *mut u16).add(1)) = next; }
}

unsafe fn write_links(element_base: *mut u8, slot_size: usize, index: u16, prev: u16, next: u16) {
    let ptr = slot_addr(element_base, slot_size, index) as *mut u16;
    unsafe { *ptr = prev; *ptr.add(1) = next; }
}

pub(crate) unsafe fn push_free_slot<T, A: Allocator>(mut group: NonNull<Group<T, A>>, index: u16) {
    let g = group.as_mut();
    let base = g.elements_base();
    let ss = g.slot_size;
    let old = g.free_list_head;
    write_links(base, ss, index, u16::MAX, old);
    if old != u16::MAX { write_prev(base, ss, old, index); }
    g.free_list_head = index;
}

pub(crate) unsafe fn pop_free_slot<T, A: Allocator>(mut group: NonNull<Group<T, A>>) -> u16 {
    let g = group.as_mut();
    let base = g.elements_base();
    let ss = g.slot_size;
    let idx = g.free_list_head;
    debug_assert_ne!(idx, u16::MAX);
    let next_idx = read_next(base, ss, idx);
    if next_idx != u16::MAX { write_prev(base, ss, next_idx, u16::MAX); }
    g.free_list_head = next_idx;
    idx
}

pub(crate) unsafe fn remove_from_free_list<T, A: Allocator>(
    mut group: NonNull<Group<T, A>>,
    index: u16,
    prev_idx: u16,
    next_idx: u16,
) {
    let g = group.as_mut();
    let base = g.elements_base();
    let ss = g.slot_size;
    if prev_idx != u16::MAX { write_next(base, ss, prev_idx, next_idx); }
    if next_idx != u16::MAX { write_prev(base, ss, next_idx, prev_idx); }
    else if prev_idx != u16::MAX { g.free_list_head = prev_idx; write_next(base, ss, prev_idx, u16::MAX); }
    else { g.free_list_head = u16::MAX; }
    write_links(base, ss, index, u16::MAX, u16::MAX);
}

pub(crate) unsafe fn replace_in_free_list<T, A: Allocator>(
    mut group: NonNull<Group<T, A>>,
    old_index: u16,
    new_index: u16,
) {
    let g = group.as_mut();
    let base = g.elements_base();
    let ss = g.slot_size;
    let prev = read_next(base, ss, old_index);
    let next = read_next(base, ss, old_index);
    write_links(base, ss, new_index, prev, next);
    if prev != u16::MAX { write_next(base, ss, prev, new_index); }
    else { g.free_list_head = new_index; }
    if next != u16::MAX { write_prev(base, ss, next, new_index); }
}
