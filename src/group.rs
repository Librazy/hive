//! Internal group (block) type for the hive.

use crate::allocator::Allocator;
use core::alloc::Layout;
use core::marker::PhantomData;
use core::mem::{align_of, size_of};
use core::ptr::NonNull;

pub(crate) struct Group<T, A: Allocator> {
    pub(crate) allocation: NonNull<u8>,
    pub(crate) allocation_size: usize,
    pub(crate) slot_size: usize,
    pub(crate) next: Option<NonNull<Group<T, A>>>,
    pub(crate) prev: Option<NonNull<Group<T, A>>>,
    pub(crate) erasures_next: Option<NonNull<Group<T, A>>>,
    pub(crate) erasures_prev: Option<NonNull<Group<T, A>>>,
    pub(crate) free_list_head: u16,
    pub(crate) capacity: u16,
    pub(crate) active_count: u16,
    pub(crate) group_number: usize,
    pub(crate) allocator: A,
    pub(crate) _marker: PhantomData<T>,
}

impl<T, A: Allocator> Group<T, A> {
    const fn uses_compact_index() -> bool {
        size_of::<T>() <= 2 && align_of::<T>() <= 2
    }

    pub(crate) const fn index_size() -> usize {
        if Self::uses_compact_index() {
            size_of::<u8>()
        } else {
            size_of::<u16>()
        }
    }

    pub(crate) const fn index_align() -> usize {
        if Self::uses_compact_index() {
            align_of::<u8>()
        } else {
            align_of::<u16>()
        }
    }

    pub(crate) const fn none_index() -> u16 {
        if Self::uses_compact_index() {
            u8::MAX as u16
        } else {
            u16::MAX
        }
    }

    pub(crate) fn compute_slot_size() -> usize {
        let min_size = core::cmp::max(size_of::<T>(), 2 * Self::index_size());
        let align = Self::allocation_align();
        min_size.div_ceil(align) * align
    }

    pub(crate) fn allocation_align() -> usize {
        core::cmp::max(align_of::<T>(), Self::index_align())
    }

    pub(crate) fn compute_allocation_size(capacity: u16, slot_size: usize) -> usize {
        slot_size * capacity as usize + Self::index_size() * (capacity as usize + 1)
    }

    pub(crate) fn allocate(
        capacity: u16,
        prev: Option<NonNull<Group<T, A>>>,
        allocator: A,
    ) -> NonNull<Group<T, A>>
    where
        A: Clone,
    {
        let slot_size = Self::compute_slot_size();
        let alloc_size = Self::compute_allocation_size(capacity, slot_size);

        let group_layout = Layout::new::<Group<T, A>>();
        let group_ptr = allocator
            .allocate(group_layout)
            .expect("hive: group allocation failed")
            .cast::<Group<T, A>>();

        let elem_layout = Layout::from_size_align(alloc_size, Self::allocation_align())
            .expect("hive: invalid element layout");
        let alloc_block = allocator
            .allocate(elem_layout)
            .expect("hive: element block allocation failed");

        let allocation: NonNull<u8> = alloc_block.cast::<u8>();

        let skipfield_base = unsafe { allocation.as_ptr().add(slot_size * capacity as usize) };
        unsafe {
            core::ptr::write_bytes(
                skipfield_base,
                0,
                (capacity as usize + 1) * Self::index_size(),
            );
        }

        let group_number = prev.map_or(0, |p| unsafe { p.as_ref().group_number + 1 });

        let group = Group {
            allocation,
            allocation_size: alloc_size,
            slot_size,
            next: None,
            prev,
            erasures_next: None,
            erasures_prev: None,
            free_list_head: Self::none_index(),
            capacity,
            active_count: 0,
            group_number,
            allocator,
            _marker: PhantomData,
        };

        unsafe {
            core::ptr::write(group_ptr.as_ptr(), group);
            group_ptr
        }
    }

    pub(crate) unsafe fn deallocate_data(&self) {
        let elem_layout = Layout::from_size_align(self.allocation_size, Self::allocation_align())
            .expect("hive: invalid element layout");
        self.allocator
            .deallocate(self.allocation.cast::<u8>(), elem_layout);
    }

    pub(crate) unsafe fn deallocate_group(this: NonNull<Group<T, A>>) {
        let g = this.as_ref();
        g.deallocate_data();
        let group_layout = Layout::new::<Group<T, A>>();
        g.allocator.deallocate(this.cast::<u8>(), group_layout);
    }

    pub(crate) unsafe fn reset(
        this: NonNull<Group<T, A>>,
        next: Option<NonNull<Group<T, A>>>,
        prev: Option<NonNull<Group<T, A>>>,
        group_number: usize,
    ) {
        // Use raw-pointer access so we do not create overlapping `&mut Group`
        // borrows with callers that already hold a raw pointer to the group.
        let gp = this.as_ptr();
        (*gp).next = next;
        (*gp).prev = prev;
        (*gp).erasures_next = None;
        (*gp).erasures_prev = None;
        (*gp).free_list_head = Self::none_index();
        (*gp).active_count = 1;
        (*gp).group_number = group_number;
        let cap = (*gp).capacity as usize;
        let sf = (*gp).skipfield_mut();
        core::ptr::write_bytes(sf, 0, cap * Self::index_size());
    }

    // ── Accessors ──

    pub(crate) unsafe fn elements_base(&self) -> *mut u8 {
        self.allocation.as_ptr()
    }

    pub(crate) unsafe fn element_ptr_mut(&self, index: u16) -> *mut T {
        self.elements_base().add(index as usize * self.slot_size) as *mut T
    }

    pub(crate) unsafe fn element_ptr(&self, index: u16) -> *const T {
        self.elements_base().add(index as usize * self.slot_size) as *const T
    }

    pub(crate) unsafe fn skipfield_mut(&self) -> *mut u8 {
        self.allocation
            .as_ptr()
            .add(self.slot_size * self.capacity as usize)
    }

    pub(crate) unsafe fn skipfield_ptr(&self) -> *const u8 {
        self.allocation
            .as_ptr()
            .add(self.slot_size * self.capacity as usize)
    }

    pub(crate) unsafe fn skipfield_at(&self, index: usize) -> u16 {
        let ptr = self.skipfield_ptr().add(index * Self::index_size());
        if Self::index_size() == 1 {
            *ptr as u16
        } else {
            *(ptr as *const u16)
        }
    }

    pub(crate) unsafe fn write_skipfield_at(&self, index: usize, value: u16) {
        debug_assert!(value <= Self::none_index());
        let ptr = self.skipfield_mut().add(index * Self::index_size());
        if Self::index_size() == 1 {
            *ptr = value as u8;
        } else {
            *(ptr as *mut u16) = value;
        }
    }

    pub(crate) unsafe fn skipfield_ptr_at(&self, index: usize) -> *const u8 {
        self.skipfield_ptr().add(index * Self::index_size())
    }

    pub(crate) unsafe fn read_index_at(&self, ptr: *const u8) -> u16 {
        if Self::index_size() == 1 {
            *ptr as u16
        } else {
            *(ptr as *const u16)
        }
    }

    pub(crate) unsafe fn write_index_at(&self, ptr: *mut u8, value: u16) {
        debug_assert!(value <= Self::none_index());
        if Self::index_size() == 1 {
            *ptr = value as u8;
        } else {
            *(ptr as *mut u16) = value;
        }
    }

    pub(crate) fn is_full(&self) -> bool {
        self.active_count == self.capacity
    }

    pub(crate) unsafe fn index_from_element_ptr(&self, ptr: *const u8) -> u16 {
        let offset_bytes = ptr.offset_from(self.elements_base());
        debug_assert!(offset_bytes >= 0);
        (offset_bytes as usize / self.slot_size) as u16
    }
}
