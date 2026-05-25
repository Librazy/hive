//! Jump-counting skipfield pattern and cursor types.

use core::alloc::Allocator;
use core::marker::PhantomData;
use core::ptr::NonNull;
use crate::group::Group;

pub(crate) struct Cursor<T, A: Allocator> {
    pub group: Option<NonNull<Group<T, A>>>,
    pub element: *const u8,
    pub skipfield: *const u16,
    pub _marker: PhantomData<T>,
}

// Manual Clone+Copy impl without bounds on T/A
impl<T, A: Allocator> Clone for Cursor<T, A> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, A: Allocator> Copy for Cursor<T, A> {}

impl<T, A: Allocator> Cursor<T, A> {
    #[allow(dead_code)]
    pub fn null() -> Self {
        Self { group: None, element: core::ptr::null(), skipfield: core::ptr::null(), _marker: PhantomData }
    }
    #[allow(dead_code)]
    pub fn is_null(&self) -> bool { self.group.is_none() }

    pub unsafe fn advance_forward(&self) -> Cursor<T, A> {
        let g = self.group.expect("null cursor").as_ref();
        let sf_base = g.skipfield_ptr();
        let elem_base = g.elements_base();
        let slot_size = g.slot_size;
        let cap = g.capacity as usize;
        let sf_idx = self.skipfield.offset_from(sf_base) as usize;

        if sf_idx + 1 < cap {
            let nv = *self.skipfield.add(1);
            let ni = if nv == 0 { sf_idx + 1 } else { sf_idx + 1 + nv as usize };
            if ni < cap {
                Cursor {
                    group: self.group,
                    element: elem_base.add(ni * slot_size),
                    skipfield: sf_base.add(ni),
                    _marker: PhantomData,
                }
            } else {
                // Overflowed past the group — move to next group
                let next = g.next.expect("advanced past end");
                let ng = next.as_ref();
                let sf0 = *ng.skipfield_ptr();
                let (elem, sf_ptr) = if sf0 == 0 {
                    (ng.elements_base(), ng.skipfield_ptr())
                } else {
                    (ng.elements_base().add(sf0 as usize * ng.slot_size), ng.skipfield_ptr().add(sf0 as usize))
                };
                Cursor {
                    group: Some(next),
                    element: elem,
                    skipfield: sf_ptr,
                    _marker: PhantomData,
                }
            }
        } else {
            let next = g.next.expect("advanced past end");
            let ng = next.as_ref();
            // Find first active element in the new group
            let sf0 = *ng.skipfield_ptr();
            if sf0 == 0 {
                Cursor {
                    group: Some(next),
                    element: ng.elements_base(),
                    skipfield: ng.skipfield_ptr(),
                    _marker: PhantomData,
                }
            } else {
                // First slot is erased, jump over it
                let ni = sf0 as usize;
                Cursor {
                    group: Some(next),
                    element: ng.elements_base().add(ni * ng.slot_size),
                    skipfield: ng.skipfield_ptr().add(ni),
                    _marker: PhantomData,
                }
            }
        }
    }

    pub unsafe fn advance_backward(&self) -> Cursor<T, A> {
        let g = self.group.expect("null cursor").as_ref();
        let sf_base = g.skipfield_ptr();
        let elem_base = g.elements_base();
        let slot_size = g.slot_size;
        let sf_idx = self.skipfield.offset_from(sf_base) as usize;

        let (group, elem, sf) = if sf_idx > 0 {
            let prev_val = *self.skipfield.sub(1);
            if prev_val == 0 {
                let ni = sf_idx - 1;
                (self.group, elem_base.add(ni * slot_size), sf_base.add(ni))
            } else {
                let ni = sf_idx - 1 - prev_val as usize;
                (self.group, elem_base.add(ni * slot_size), sf_base.add(ni))
            }
        } else {
            let prev = g.prev.expect("retreated past begin");
            let pg = prev.as_ref();
            // Find last active element in the previous group
            let (sf_ptr, idx) = find_last_active(pg);
            (Some(prev), pg.elements_base().add(idx * pg.slot_size), sf_ptr)
        };

        Cursor { group, element: elem, skipfield: sf, _marker: PhantomData }
    }
}

unsafe fn find_last_active<T, A: Allocator>(g: &Group<T, A>) -> (*const u16, usize) {
    let sf = g.skipfield_ptr();
    let cap = g.capacity as usize;
    let mut idx = cap;
    loop {
        debug_assert!(idx > 0, "no active elements in group");
        idx -= 1;
        let v = *sf.add(idx);
        if v != 0 {
            idx -= v as usize;
        } else {
            if idx + 1 < cap && *sf.add(idx + 1) != 0 {
                let run_len = *sf.add(idx + 1);
                let run_start = (idx + 1).saturating_sub(run_len as usize);
                if idx > run_start { idx = run_start; continue; }
            }
            return (sf.add(idx), idx);
        }
    }
}

pub(crate) unsafe fn mark_erased<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) {
    let g = group.as_ref();
    let sf = g.skipfield_mut();
    let cap = g.capacity;
    let prev_e = index > 0 && *sf.add(index as usize - 1) != 0;
    let next_e = (index as usize) + 1 < cap as usize && *sf.add(index as usize + 1) != 0;

    if !prev_e && !next_e {
        *sf.add(index as usize) = 1;
    } else if prev_e && !next_e {
        let lv = *sf.add(index as usize - 1);
        let nv = lv + 1;
        *sf.add(index as usize - lv as usize) = nv;
        *sf.add(index as usize) = nv;
    } else if !prev_e && next_e {
        let rv = *sf.add(index as usize + 1);
        let nv = rv + 1;
        *sf.add(index as usize) = nv;
        *sf.add(index as usize + nv as usize - 1) = nv;
    } else {
        let lv = *sf.add(index as usize - 1);
        let rv = *sf.add(index as usize + 1);
        let m = lv + rv + 1;
        *sf.add(index as usize) = 1;
        *sf.add(index as usize - lv as usize) = m;
        *sf.add(index as usize + rv as usize) = m;
    }
}

pub(crate) unsafe fn mark_constructed<T, A: Allocator>(group: NonNull<Group<T, A>>, index: u16) {
    let g = group.as_ref();
    let sf = g.skipfield_mut();
    let skip_val = *sf.add(index as usize);
    debug_assert_ne!(skip_val, 0, "mark_constructed on non-erased slot");

    if skip_val == 1 {
        *sf.add(index as usize) = 0;
        return;
    }

    let capacity = g.capacity as usize;
    let idx = index as usize;
    let n = skip_val as usize;

    // Check if this slot is the START of the run (has matching end boundary at idx+n-1)
    let is_start = n > 1
        && idx + n <= capacity + 1  // idx+n-1 < capacity
        && idx + n >= 1             // idx+n-1 >= 0
        && idx + n - 1 < capacity
        && *sf.add(idx + n - 1) == skip_val;

    // Check if this slot is the END of the run (has matching start boundary at idx+1-n)
    let is_end = n > 1
        && idx + 1 >= n  // idx - n + 1 >= 0
        && *sf.add(idx + 1 - n) == skip_val;

    if is_start && !is_end {
        // Remove from start: shrink run rightward
        *sf.add(idx) = 0;
        let new_val = skip_val - 1;
        if new_val > 0 {
            let end_idx = idx + n - 1;
            *sf.add(end_idx) = new_val;
            if n > 2 {
                *sf.add(idx + 1) = new_val;
            }
        }
    } else if is_end && !is_start {
        // Remove from end: shrink run leftward
        *sf.add(idx) = 0;
        let new_val = skip_val - 1;
        if new_val > 0 {
            let start_idx = idx - n + 1;
            *sf.add(start_idx) = new_val;
            if n > 2 {
                *sf.add(idx - 1) = new_val;
            }
        }
    } else if is_start && is_end {
        // n == 2 run, both slots are boundaries — became a single slot, but skip_val != 1
        // shouldn't reach here normally, but handle gracefully
        *sf.add(idx) = 0;
    } else {
        // Interior — remove from middle, merge remaining left+right runs
        let mut left_idx = idx - 1;
        while left_idx > 0 && *sf.add(left_idx) == 0 {
            left_idx -= 1;
        }
        let left_start_val = *sf.add(left_idx);
        let mut right_idx = idx + 1;
        while right_idx < capacity && *sf.add(right_idx) == 0 {
            right_idx += 1;
        }
        let right_end_val = *sf.add(right_idx);

        *sf.add(idx) = 0;
        let merged_len = left_start_val + right_end_val;
        let left_start = left_idx - left_start_val as usize + 1;
        let right_end = right_idx;

        *sf.add(left_start) = merged_len;
        *sf.add(right_end) = merged_len;
    }
}
