#![cfg_attr(feature = "allocator_api", feature(allocator_api))]
use hive::allocator::{AllocError, Allocator, Global};
use hive::{BlockCapacityLimits, Hive, Pool};
use std::{
    alloc::Layout,
    panic::{catch_unwind, AssertUnwindSafe},
    ptr::NonNull,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct PanicsOnce<'a>(&'a AtomicUsize);
impl Drop for PanicsOnce<'_> {
    fn drop(&mut self) {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("first drop");
        }
    }
}
#[test]
fn clear_panic_does_not_drop_element_twice() {
    let count = AtomicUsize::new(0);
    let before = AtomicUsize::new(1);
    let after = AtomicUsize::new(1);
    let mut h = Hive::new();
    h.insert(PanicsOnce(&before));
    h.insert(PanicsOnce(&count));
    h.insert(PanicsOnce(&after));
    assert!(catch_unwind(AssertUnwindSafe(|| h.clear())).is_err());
    assert_eq!(h.len(), 1);
    drop(h);
    assert_eq!(before.load(Ordering::SeqCst), 2);
    assert_eq!(after.load(Ordering::SeqCst), 2);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
#[test]
fn pool_guard_panic_does_not_drop_element_twice() {
    let count = AtomicUsize::new(0);
    let pool = Pool::new();
    let guard = pool.insert(PanicsOnce(&count));
    assert!(catch_unwind(AssertUnwindSafe(|| drop(guard))).is_err());
    assert!(pool.is_empty());
    drop(pool);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
#[derive(Clone)]
struct Tracked {
    _owner: Arc<()>,
}
unsafe impl Allocator for Tracked {
    fn allocate(&self, l: Layout) -> Result<NonNull<[u8]>, AllocError> {
        Global.allocate(l)
    }
    unsafe fn deallocate(&self, p: NonNull<u8>, l: Layout) {
        unsafe { Global.deallocate(p, l) }
    }
}
#[test]
fn consuming_iterator_releases_allocator() {
    for n in [0, 10] {
        let owner = Arc::new(());
        let mut h = Hive::new_in(Tracked {
            _owner: owner.clone(),
        });
        h.extend(0..n);
        let mut iter = h.into_iter();
        iter.next();
        drop(iter);
        assert_eq!(Arc::strong_count(&owner), 1);
    }
}
#[test]
fn reshape_releases_old_allocator() {
    let owner = Arc::new(());
    let mut h = Hive::try_new_in(
        Tracked {
            _owner: owner.clone(),
        },
        BlockCapacityLimits::new(3, 3),
    )
    .unwrap();
    h.extend(0..10);
    h.reshape(BlockCapacityLimits::new(4, 4)).unwrap();
    assert_eq!(
        h.iter().copied().collect::<Vec<_>>(),
        (0..10).collect::<Vec<_>>()
    );
    drop(h);
    assert_eq!(Arc::strong_count(&owner), 1);
}
