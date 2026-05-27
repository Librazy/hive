//! Inline free list stored in erased skipblock head slots.

use crate::allocator::Allocator;
use crate::group::Group;
use core::ptr::NonNull;

unsafe fn slot_addr(element_base: *mut u8, slot_size: usize, index: u16) -> *mut u8 {
    unsafe { element_base.add(index as usize * slot_size) }
}

pub(crate) unsafe fn head_next<T, A: Allocator>(group: NonNull<Group<T, A>>) -> u16 {
    let gp = group.as_ptr();
    let index = (*gp).free_list_head;
    debug_assert_ne!(index, Group::<T, A>::none_index());
    read_next(group, index)
}

unsafe fn link_addr<T, A: Allocator>(
    group: NonNull<Group<T, A>>,
    index: u16,
    slot: usize,
) -> *mut u8 {
    let gp = group.as_ptr();
    slot_addr((*gp).elements_base(), (*gp).slot_size, index).add(slot * Group::<T, A>::index_size())
}

unsafe fn read_prev<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) -> u16 {
    (*group.as_ptr()).read_index_at(link_addr(group, index, 0))
}

unsafe fn read_next<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) -> u16 {
    (*group.as_ptr()).read_index_at(link_addr(group, index, 1))
}

unsafe fn write_prev<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16, prev: u16) {
    (*group.as_ptr()).write_index_at(link_addr(group, index, 0), prev);
}

unsafe fn write_next<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16, next: u16) {
    (*group.as_ptr()).write_index_at(link_addr(group, index, 1), next);
}

unsafe fn write_links<T, A: Allocator>(
    group: NonNull<Group<T, A>>,
    index: u16,
    prev: u16,
    next: u16,
) {
    write_prev(group, index, prev);
    write_next(group, index, next);
}

unsafe fn push_node<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) -> bool {
    let gp = group.as_ptr();
    let none = Group::<T, A>::none_index();
    let old = (*gp).free_list_head;
    write_links(group, index, none, old);
    if old != none {
        write_prev(group, old, index);
    }
    (*gp).free_list_head = index;
    old == none
}

unsafe fn remove_node<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) {
    let gp = group.as_ptr();
    let none = Group::<T, A>::none_index();
    let prev = read_prev(group, index);
    let next = read_next(group, index);

    if prev != none {
        write_next(group, prev, next);
    } else {
        debug_assert_eq!((*gp).free_list_head, index);
        (*gp).free_list_head = next;
    }

    if next != none {
        write_prev(group, next, prev);
    }
}

unsafe fn move_node<T, A: Allocator>(group: NonNull<Group<T, A>>, old_index: u16, new_index: u16) {
    let gp = group.as_ptr();
    let none = Group::<T, A>::none_index();
    let prev = read_prev(group, old_index);
    let next = read_next(group, old_index);

    write_links(group, new_index, prev, next);
    if prev != none {
        write_next(group, prev, new_index);
    } else {
        debug_assert_eq!((*gp).free_list_head, old_index);
        (*gp).free_list_head = new_index;
    }
    if next != none {
        write_prev(group, next, new_index);
    }
}

pub(crate) unsafe fn mark_erased<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) -> bool {
    let gp = group.as_ptr();
    let idx = index as usize;
    let prev_erased = index > 0 && (*gp).skipfield_at(idx - 1) != 0;
    let next_erased = (*gp).skipfield_at(idx + 1) != 0;

    if !prev_erased && !next_erased {
        (*gp).write_skipfield_at(idx, 1);
        push_node(group, index)
    } else if prev_erased && !next_erased {
        let left_len = (*gp).skipfield_at(idx - 1);
        let new_len = left_len + 1;
        (*gp).write_skipfield_at(idx - left_len as usize, new_len);
        (*gp).write_skipfield_at(idx, new_len);
        false
    } else if !prev_erased && next_erased {
        let right_len = (*gp).skipfield_at(idx + 1);
        let new_len = right_len + 1;
        let end = idx + new_len as usize - 1;
        (*gp).write_skipfield_at(idx, new_len);
        (*gp).write_skipfield_at(end, new_len);
        move_node(group, index + 1, index);
        false
    } else {
        let left_len = (*gp).skipfield_at(idx - 1);
        let right_len = (*gp).skipfield_at(idx + 1);
        let new_len = left_len + right_len + 1;
        let left_start = idx - left_len as usize;
        let right_start = index + 1;
        let end = idx + right_len as usize;

        (*gp).write_skipfield_at(idx, 1);
        (*gp).write_skipfield_at(left_start, new_len);
        (*gp).write_skipfield_at(end, new_len);
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

    let idx = index as usize;
    let old_len = (*gp).skipfield_at(idx);
    debug_assert_ne!(old_len, 0, "consume_head_skipblock on active slot");

    let none = Group::<T, A>::none_index();
    if old_len == 1 {
        (*gp).free_list_head = next;
        if next != none {
            write_prev(group, next, none);
        }
        (*gp).write_skipfield_at(idx, 0);
    } else {
        let new_len = old_len - 1;
        let new_index = index + 1;
        let end = idx + old_len as usize - 1;

        (*gp).write_skipfield_at(idx, 0);
        (*gp).write_skipfield_at(idx + 1, new_len);
        (*gp).write_skipfield_at(end, new_len);
        (*gp).free_list_head = new_index;
        write_links(group, new_index, none, next);
        if next != none {
            write_prev(group, next, new_index);
        }
    }

    (*gp).free_list_head == none
}
