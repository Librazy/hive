//! Jump-counting skipfield pattern and cursor types.

use crate::allocator::Allocator;
use crate::group::Group;
use core::marker::PhantomData;
use core::ptr::NonNull;

pub(crate) struct Cursor<T, A: Allocator> {
    pub group: Option<NonNull<Group<T, A>>>,
    pub element: *const u8,
    pub skipfield: *const u8,
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
        Self {
            group: None,
            element: core::ptr::null(),
            skipfield: core::ptr::null(),
            _marker: PhantomData,
        }
    }
    #[allow(dead_code)]
    pub fn is_null(&self) -> bool {
        self.group.is_none()
    }

    pub unsafe fn advance_forward(&self) -> Cursor<T, A> {
        let g = self.group.expect("null cursor").as_ref();
        let sf_base = g.skipfield_ptr();
        let elem_base = g.elements_base();
        let slot_size = g.slot_size;
        let cap = g.capacity as usize;
        let sf_idx = self.skipfield.offset_from(sf_base) as usize / Group::<T, A>::index_size();

        if sf_idx + 1 < cap {
            let nv = g.skipfield_at(sf_idx + 1);
            let ni = if nv == 0 {
                sf_idx + 1
            } else {
                sf_idx + 1 + nv as usize
            };
            if ni < cap {
                Cursor {
                    group: self.group,
                    element: elem_base.add(ni * slot_size),
                    skipfield: g.skipfield_ptr_at(ni),
                    _marker: PhantomData,
                }
            } else {
                // Overflowed past the group — move to next group
                let next = g.next.expect("advanced past end");
                let ng = next.as_ref();
                let sf0 = ng.skipfield_at(0);
                let (elem, sf_ptr) = if sf0 == 0 {
                    (ng.elements_base(), ng.skipfield_ptr())
                } else {
                    (
                        ng.elements_base().add(sf0 as usize * ng.slot_size),
                        ng.skipfield_ptr_at(sf0 as usize),
                    )
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
            let sf0 = ng.skipfield_at(0);
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
                    skipfield: ng.skipfield_ptr_at(ni),
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
        let sf_idx = self.skipfield.offset_from(sf_base) as usize / Group::<T, A>::index_size();

        let (group, elem, sf) = if sf_idx > 0 {
            let prev_val = g.skipfield_at(sf_idx - 1);
            if prev_val == 0 {
                let ni = sf_idx - 1;
                (
                    self.group,
                    elem_base.add(ni * slot_size),
                    g.skipfield_ptr_at(ni),
                )
            } else {
                let ni = sf_idx - 1 - prev_val as usize;
                (
                    self.group,
                    elem_base.add(ni * slot_size),
                    g.skipfield_ptr_at(ni),
                )
            }
        } else {
            let prev = g.prev.expect("retreated past begin");
            let pg = prev.as_ref();
            // Find last active element in the previous group
            let (sf_ptr, idx) = find_last_active(pg);
            (
                Some(prev),
                pg.elements_base().add(idx * pg.slot_size),
                sf_ptr,
            )
        };

        Cursor {
            group,
            element: elem,
            skipfield: sf,
            _marker: PhantomData,
        }
    }
}

unsafe fn find_last_active<T, A: Allocator>(g: &Group<T, A>) -> (*const u8, usize) {
    let cap = g.capacity as usize;
    let mut idx = cap;
    loop {
        debug_assert!(idx > 0, "no active elements in group");
        idx -= 1;
        let v = g.skipfield_at(idx);
        if v != 0 {
            idx -= v as usize;
        } else {
            if idx + 1 < cap && g.skipfield_at(idx + 1) != 0 {
                let run_len = g.skipfield_at(idx + 1);
                let run_start = (idx + 1).saturating_sub(run_len as usize);
                if idx > run_start {
                    idx = run_start;
                    continue;
                }
            }
            return (g.skipfield_ptr_at(idx), idx);
        }
    }
}
