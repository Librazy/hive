//! Inline free list stored in erased skipblock head slots.

use crate::allocator::Allocator;
use crate::group::Group;
use core::ptr::NonNull;

const NONE: u16 = u16::MAX;

unsafe fn slot_addr(element_base: *mut u8, slot_size: usize, index: u16) -> *mut u8 {
    unsafe { element_base.add(index as usize * slot_size) }
}

unsafe fn slot_addr_const(element_base: *const u8, slot_size: usize, index: u16) -> *const u8 {
    unsafe { element_base.add(index as usize * slot_size) }
}

unsafe fn read_prev(element_base: *const u8, slot_size: usize, index: u16) -> u16 {
    unsafe { *(slot_addr_const(element_base, slot_size, index) as *const u16) }
}

unsafe fn read_next(element_base: *const u8, slot_size: usize, index: u16) -> u16 {
    unsafe { *((slot_addr_const(element_base, slot_size, index) as *const u16).add(1)) }
}

pub(crate) unsafe fn head_next<T, A: Allocator>(group: NonNull<Group<T, A>>) -> u16 {
    let gp = group.as_ptr();
    let index = (*gp).free_list_head;
    debug_assert_ne!(index, NONE);
    read_next((*gp).elements_base(), (*gp).slot_size, index)
}

unsafe fn write_prev(element_base: *mut u8, slot_size: usize, index: u16, prev: u16) {
    unsafe {
        *(slot_addr(element_base, slot_size, index) as *mut u16) = prev;
    }
}

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

unsafe fn push_node<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) -> bool {
    let gp = group.as_ptr();
    let base = (*gp).elements_base();
    let ss = (*gp).slot_size;
    let old = (*gp).free_list_head;
    write_links(base, ss, index, NONE, old);
    if old != NONE {
        write_prev(base, ss, old, index);
    }
    (*gp).free_list_head = index;
    old == NONE
}

unsafe fn remove_node<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) {
    let gp = group.as_ptr();
    let base = (*gp).elements_base();
    let ss = (*gp).slot_size;
    let prev = read_prev(base, ss, index);
    let next = read_next(base, ss, index);

    if prev != NONE {
        write_next(base, ss, prev, next);
    } else {
        debug_assert_eq!((*gp).free_list_head, index);
        (*gp).free_list_head = next;
    }

    if next != NONE {
        write_prev(base, ss, next, prev);
    }
}

unsafe fn move_node<T, A: Allocator>(group: NonNull<Group<T, A>>, old_index: u16, new_index: u16) {
    let gp = group.as_ptr();
    let base = (*gp).elements_base();
    let ss = (*gp).slot_size;
    let prev = read_prev(base, ss, old_index);
    let next = read_next(base, ss, old_index);

    write_links(base, ss, new_index, prev, next);
    if prev != NONE {
        write_next(base, ss, prev, new_index);
    } else {
        debug_assert_eq!((*gp).free_list_head, old_index);
        (*gp).free_list_head = new_index;
    }
    if next != NONE {
        write_prev(base, ss, next, new_index);
    }
}

pub(crate) unsafe fn mark_erased<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) -> bool {
    let gp = group.as_ptr();
    let sf = (*gp).skipfield_mut();
    let idx = index as usize;
    let prev_erased = index > 0 && *sf.add(idx - 1) != 0;
    let next_erased = *sf.add(idx + 1) != 0;

    if !prev_erased && !next_erased {
        *sf.add(idx) = 1;
        push_node(group, index)
    } else if prev_erased && !next_erased {
        let left_len = *sf.add(idx - 1);
        let new_len = left_len + 1;
        *sf.add(idx - left_len as usize) = new_len;
        *sf.add(idx) = new_len;
        false
    } else if !prev_erased && next_erased {
        let right_len = *sf.add(idx + 1);
        let new_len = right_len + 1;
        let end = idx + new_len as usize - 1;
        *sf.add(idx) = new_len;
        *sf.add(end) = new_len;
        move_node(group, index + 1, index);
        false
    } else {
        let left_len = *sf.add(idx - 1);
        let right_len = *sf.add(idx + 1);
        let new_len = left_len + right_len + 1;
        let left_start = idx - left_len as usize;
        let right_start = index + 1;
        let end = idx + right_len as usize;

        *sf.add(idx) = 1;
        *sf.add(left_start) = new_len;
        *sf.add(end) = new_len;
        remove_node(group, right_start);
        false
    }
}

pub(crate) unsafe fn consume_head_skipblock<T, A: Allocator>(
    group: NonNull<Group<T, A>>,
    index: u16,
) -> bool {
    consume_head_skipblock_with_next(group, index, head_next(group))
}

pub(crate) unsafe fn consume_head_skipblock_with_next<T, A: Allocator>(
    group: NonNull<Group<T, A>>,
    index: u16,
    next: u16,
) -> bool {
    let gp = group.as_ptr();
    debug_assert_eq!((*gp).free_list_head, index);

    let sf = (*gp).skipfield_mut();
    let idx = index as usize;
    let old_len = *sf.add(idx);
    debug_assert_ne!(old_len, 0, "consume_head_skipblock on active slot");

    let base = (*gp).elements_base();
    let ss = (*gp).slot_size;
    if old_len == 1 {
        (*gp).free_list_head = next;
        if next != NONE {
            write_prev(base, ss, next, NONE);
        }
        *sf.add(idx) = 0;
    } else {
        let new_len = old_len - 1;
        let new_index = index + 1;
        let end = idx + old_len as usize - 1;

        *sf.add(idx) = 0;
        *sf.add(idx + 1) = new_len;
        *sf.add(end) = new_len;
        (*gp).free_list_head = new_index;
        write_links(base, ss, new_index, NONE, next);
        if next != NONE {
            write_prev(base, ss, next, new_index);
        }
    }

    (*gp).free_list_head == NONE
}
