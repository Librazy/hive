//! Inline free list stored in erased element memory slots.

use crate::allocator::Allocator;
use crate::group::Group;
use core::ptr::NonNull;

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
    unsafe {
        *(slot_addr(element_base, slot_size, index) as *mut u16) = prev;
    }
}

#[allow(dead_code)]
unsafe fn write_next(element_base: *mut u8, slot_size: usize, index: u16, next: u16) {
    unsafe {
        *((slot_addr(element_base, slot_size, index) as *mut u16).add(1)) = next;
    }
}

unsafe fn write_links(element_base: *mut u8, slot_size: usize, index: u16, prev: u16, next: u16) {
    let ptr = slot_addr(element_base, slot_size, index) as *mut u16;
    unsafe {
        *ptr = prev;
        *ptr.add(1) = next;
    }
}

pub(crate) unsafe fn push_free_slot<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) {
    let gp = group.as_ptr();
    let base = (*gp).elements_base();
    let ss = (*gp).slot_size;
    let old = (*gp).free_list_head;
    write_links(base, ss, index, u16::MAX, old);
    if old != u16::MAX {
        write_prev(base, ss, old, index);
    }
    (*gp).free_list_head = index;
}

pub(crate) unsafe fn pop_free_slot<T, A: Allocator>(group: NonNull<Group<T, A>>) -> u16 {
    let gp = group.as_ptr();
    let base = (*gp).elements_base();
    let ss = (*gp).slot_size;
    let idx = (*gp).free_list_head;
    debug_assert_ne!(idx, u16::MAX);
    let next_idx = read_next(base, ss, idx);
    if next_idx != u16::MAX {
        write_prev(base, ss, next_idx, u16::MAX);
    }
    (*gp).free_list_head = next_idx;
    idx
}

#[allow(dead_code)]
pub(crate) unsafe fn remove_from_free_list<T, A: Allocator>(
    group: NonNull<Group<T, A>>,
    index: u16,
    prev_idx: u16,
    next_idx: u16,
) {
    let gp = group.as_ptr();
    let base = (*gp).elements_base();
    let ss = (*gp).slot_size;
    if prev_idx != u16::MAX {
        write_next(base, ss, prev_idx, next_idx);
    }
    if next_idx != u16::MAX {
        write_prev(base, ss, next_idx, prev_idx);
    } else if prev_idx != u16::MAX {
        (*gp).free_list_head = prev_idx;
        write_next(base, ss, prev_idx, u16::MAX);
    } else {
        (*gp).free_list_head = u16::MAX;
    }
    write_links(base, ss, index, u16::MAX, u16::MAX);
}

#[allow(dead_code)]
pub(crate) unsafe fn replace_in_free_list<T, A: Allocator>(
    group: NonNull<Group<T, A>>,
    old_index: u16,
    new_index: u16,
) {
    let gp = group.as_ptr();
    let base = (*gp).elements_base();
    let ss = (*gp).slot_size;
    let prev = read_next(base, ss, old_index);
    let next = read_next(base, ss, old_index);
    write_links(base, ss, new_index, prev, next);
    if prev != u16::MAX {
        write_next(base, ss, prev, new_index);
    } else {
        (*gp).free_list_head = new_index;
    }
    if next != u16::MAX {
        write_prev(base, ss, next, new_index);
    }
}
