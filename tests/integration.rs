#![cfg_attr(feature = "allocator_api", feature(allocator_api))]

use hive::allocator::Global;
#[cfg(feature = "std")]
use hive::SyncPool;
use hive::{BlockCapacityLimits, Hive, IncompatibleSplice, Pool};
use std::cell::Cell;
#[cfg(feature = "pin-init")]
use std::convert::Infallible;
#[cfg(feature = "pin-init")]
use std::marker::PhantomPinned;
#[cfg(feature = "pin-init")]
use std::pin::Pin;
#[cfg(feature = "pin-init")]
use std::ptr;
use std::rc::Rc;

#[cfg(feature = "pin-init")]
use pin_init::{init_from_closure, InitResult, PinUninit};

#[test]
fn test_new_empty() {
    let h: Hive<i32> = Hive::new();
    assert!(h.is_empty());
    assert_eq!(h.len(), 0);
}

#[test]
fn test_insert_one() {
    let mut h = Hive::new();
    h.insert(42);
    assert!(!h.is_empty());
    assert_eq!(h.len(), 1);
}

#[test]
fn test_insert_with_uninit() {
    let mut h = Hive::new();
    let ptr = unsafe {
        h.insert_with_uninit(|slot| {
            slot.write(String::from("hello"));
        })
    };

    assert_eq!(h.len(), 1);
    unsafe {
        assert_eq!(&*ptr, "hello");
    }
}

#[test]
fn test_insert_with_uninit_mut() {
    let mut h = Hive::new();
    let ptr = unsafe {
        h.insert_with_uninit_mut(|slot| {
            slot.write(41);
        })
    };

    unsafe {
        *ptr += 1;
    }
    assert_eq!(h.iter().next(), Some(&42));
}

#[cfg(feature = "pin-init")]
struct NeedPin {
    address: *const NeedPin,
    value: i32,
    _pinned: PhantomPinned,
}

#[cfg(feature = "pin-init")]
impl NeedPin {
    fn new(value: i32) -> impl pin_init::Init<Self, Infallible> {
        init_from_closure(
            move |mut this: PinUninit<'_, Self>| -> InitResult<'_, Self, Infallible> {
                let ptr = this.get_mut().as_mut_ptr();
                unsafe {
                    ptr::addr_of_mut!((*ptr).address).write(ptr);
                    ptr::addr_of_mut!((*ptr).value).write(value);
                    ptr::addr_of_mut!((*ptr)._pinned).write(PhantomPinned);
                    Ok(this.init_ok())
                }
            },
        )
    }

    fn verify(self: Pin<&Self>) {
        assert!(ptr::eq(&*self, self.address));
    }
}

#[cfg(feature = "pin-init")]
#[test]
fn test_insert_pin_init() {
    let mut h = Hive::new();
    let ptr = h.insert_pin_init(NeedPin::new(7)).unwrap();

    assert_eq!(h.len(), 1);
    unsafe {
        Pin::new_unchecked(&*ptr).verify();
        assert_eq!((*ptr).value, 7);
    }
}

#[cfg(feature = "pin-init")]
#[test]
fn test_insert_pin_init_mut() {
    let mut h = Hive::new();
    let ptr = h.insert_pin_init_mut::<_, Infallible>(41).unwrap();

    unsafe {
        *ptr += 1;
    }

    assert_eq!(h.iter().next(), Some(&42));
}

#[cfg(feature = "pin-init")]
#[test]
fn test_insert_pin_init_error_leaves_hive_unchanged() {
    let mut h = Hive::new();
    h.insert(1);

    let result = h.insert_pin_init(init_from_closure(
        |this: PinUninit<'_, i32>| -> InitResult<'_, i32, &'static str> {
            Err(this.init_err("failed"))
        },
    ));

    assert_eq!(result, Err("failed"));
    assert_eq!(h.len(), 1);
    assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![1]);
}

#[cfg(feature = "pin-init")]
#[test]
fn test_insert_pin_init_reuses_erased_slot() {
    let mut h = Hive::new();
    h.insert(1);
    let erased = h.insert(2);
    h.insert(3);
    unsafe {
        h.erase(erased);
    }

    let inserted = h.insert_pin_init::<_, Infallible>(4).unwrap();

    assert_eq!(inserted, erased);
    let mut vals: Vec<i32> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, vec![1, 3, 4]);
}

#[test]
fn test_emplace() {
    let mut h = Hive::new();
    let ptr = h.emplace(|| String::from("hello"));

    assert_eq!(h.len(), 1);
    assert_eq!(unsafe { h.get(ptr) }.unwrap(), "hello");
}

#[test]
fn test_emplace_mut() {
    let mut h = Hive::new();
    let ptr = h.emplace_mut(|| 41);

    unsafe {
        *ptr += 1;
    }

    assert_eq!(h.iter().next(), Some(&42));
}

#[test]
fn test_emplace_panic_leaves_hive_unchanged() {
    let mut h = Hive::new();
    h.insert(1);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.emplace(|| -> i32 { panic!("construction failed") });
    }));

    assert!(result.is_err());
    assert_eq!(h.len(), 1);
    assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![1]);
}

#[test]
fn test_insert_with_default_initialized_slot() {
    let mut h = Hive::new();
    let ptr = h.insert_with(|value: &mut String| {
        value.push_str("built in place");
    });

    assert_eq!(h.len(), 1);
    assert_eq!(unsafe { h.get(ptr) }.unwrap(), "built in place");
}

#[test]
fn test_insert_with_mut_default_initialized_slot() {
    let mut h = Hive::new();
    let ptr = h.insert_with_mut(|value: &mut Vec<i32>| {
        value.extend([1, 2, 3]);
    });

    unsafe {
        (*ptr).push(4);
    }

    assert_eq!(h.iter().next().unwrap(), &vec![1, 2, 3, 4]);
}

#[test]
fn test_insert_with_panic_leaves_default_value_inserted() {
    let mut h = Hive::new();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.insert_with(|value: &mut Vec<i32>| {
            value.push(1);
            panic!("initialization failed");
        });
    }));

    assert!(result.is_err());
    assert_eq!(h.len(), 1);
    assert_eq!(h.iter().next().unwrap(), &vec![1]);
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PanicDefault(i32);

impl Default for PanicDefault {
    fn default() -> Self {
        panic!("default failed")
    }
}

#[test]
fn test_insert_with_default_panic_leaves_empty_hive_unchanged() {
    let mut h = Hive::<PanicDefault>::new();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.insert_with(|_| {});
    }));

    assert!(result.is_err());
    assert_eq!(h.len(), 0);
    assert!(h.is_empty());
}

#[test]
fn test_insert_with_default_panic_leaves_tail_append_unchanged() {
    let mut h = Hive::new();
    h.insert(PanicDefault(1));

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.insert_with(|_| {});
    }));

    assert!(result.is_err());
    assert_eq!(h.len(), 1);
    assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![PanicDefault(1)]);
}

#[test]
fn test_insert_with_default_panic_leaves_erased_slot_reusable() {
    let mut h = Hive::new();
    let erased = h.insert(PanicDefault(1));
    h.insert(PanicDefault(2));

    unsafe {
        h.erase(erased);
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.insert_with(|_| {});
    }));

    assert!(result.is_err());
    assert_eq!(h.len(), 1);

    h.insert(PanicDefault(3));
    let mut values = h.iter().copied().collect::<Vec<_>>();
    values.sort();
    assert_eq!(values, vec![PanicDefault(2), PanicDefault(3)]);
}

#[test]
fn test_insert_with_default_panic_leaves_full_tail_unchanged() {
    let mut h = Hive::try_new(BlockCapacityLimits::new(3, 3)).unwrap();
    h.insert(PanicDefault(1));
    h.insert(PanicDefault(2));
    h.insert(PanicDefault(3));

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.insert_with(|_| {});
    }));

    assert!(result.is_err());
    assert_eq!(h.len(), 3);
    assert_eq!(
        h.iter().copied().collect::<Vec<_>>(),
        vec![PanicDefault(1), PanicDefault(2), PanicDefault(3),]
    );
}

#[test]
fn test_iter_single() {
    let mut h = Hive::new();
    h.insert(42);
    assert_eq!(h.iter().next(), Some(&42));
}

#[test]
fn test_iter_multiple() {
    let mut h = Hive::new();
    for i in 0..100 {
        h.insert(i);
    }
    let sum: i32 = h.iter().sum();
    assert_eq!(sum, 4950);
}

#[test]
fn test_double_ended_iter() {
    let mut h = Hive::new();
    for i in 0..10 {
        h.insert(i);
    }
    let fwd: Vec<i32> = h.iter().copied().collect();
    assert_eq!(fwd, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

    let rev: Vec<i32> = h.iter().rev().copied().collect();
    assert_eq!(rev, vec![9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);

    let mut it = h.iter();
    assert_eq!(it.next(), Some(&0));
    assert_eq!(it.next_back(), Some(&9));
    assert_eq!(it.next(), Some(&1));
    assert_eq!(it.next_back(), Some(&8));
    assert_eq!(it.next(), Some(&2));
    assert_eq!(it.next_back(), Some(&7));
}

#[test]
fn test_next_back_single_element() {
    let mut h = Hive::new();
    h.insert(42);
    assert_eq!(h.iter().next_back(), Some(&42));
}

#[test]
fn test_erase_basic() {
    let mut h = Hive::new();
    let r0 = h.insert(42);
    let _r1 = h.insert(99);
    assert_eq!(h.len(), 2);

    unsafe {
        h.erase(r0);
    }
    assert_eq!(h.len(), 1);

    let vals: Vec<i32> = h.iter().copied().collect();
    assert_eq!(vals, vec![99]);
}

#[test]
fn test_erase_from_middle() {
    let mut h = Hive::new();
    h.insert(1);
    let r2 = h.insert(2);
    h.insert(3);

    unsafe {
        h.erase(r2);
    }

    let mut sorted: Vec<i32> = h.iter().copied().collect();
    sorted.sort();
    assert_eq!(sorted, vec![1, 3]);
}

#[test]
fn test_insert_after_erase() {
    let mut h = Hive::new();
    h.insert(1);
    let r2 = h.insert(2);
    h.insert(3);

    unsafe {
        h.erase(r2);
    }
    h.insert(4);

    assert_eq!(h.len(), 3);
    let mut sorted: Vec<i32> = h.iter().copied().collect();
    sorted.sort();
    assert_eq!(sorted, vec![1, 3, 4]);
}

#[test]
fn test_insert_with_uninit_reuses_erased_slot() {
    let mut h = Hive::new();
    h.insert(1);
    let erased = h.insert(2);
    h.insert(3);
    unsafe {
        h.erase(erased);
    }

    let inserted = unsafe {
        h.insert_with_uninit(|slot| {
            slot.write(4);
        })
    };

    assert_eq!(inserted, erased);
    let mut vals: Vec<i32> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, vec![1, 3, 4]);
}

#[test]
fn test_erase_all_one_by_one() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..100).map(|i| h.insert(i)).collect();

    for r in &refs {
        unsafe {
            h.erase(*r);
        }
    }
    assert!(h.is_empty());
}

#[test]
fn test_insert_after_empty_reuses_reserved_capacity() {
    let mut h = Hive::new();
    let ptr = h.insert(1);
    let cap = h.capacity();
    unsafe {
        h.erase(ptr);
    }
    assert_eq!(h.len(), 0);
    h.insert(2);
    assert_eq!(h.capacity(), cap);
    assert_eq!(h.iter().next(), Some(&2));
}

#[test]
fn test_clear() {
    let mut h = Hive::new();
    for i in 0..50 {
        h.insert(i);
    }
    h.clear();
    assert!(h.is_empty());
    h.insert(42);
    assert_eq!(h.len(), 1);
}

#[test]
fn test_reserve() {
    let mut h: Hive<i32> = Hive::new();
    h.reserve(100);
    assert!(h.capacity() >= 100);
    for i in 0..100 {
        h.insert(i);
    }
    assert_eq!(h.len(), 100);
}

#[test]
fn test_with_capacity() {
    let mut h: Hive<i32> = Hive::with_capacity(200);
    assert!(h.capacity() >= 200);
    for i in 0..200 {
        h.insert(i);
    }
    assert_eq!(h.len(), 200);
}

#[test]
fn test_block_capacity_limit_apis() {
    let defaults = Hive::<i32>::block_capacity_default_limits();
    assert!(defaults.min >= 8);
    assert!(defaults.min <= defaults.max);
    assert_eq!(defaults.max, 255);
    assert_eq!(Hive::<i32>::block_capacity_hard_limits().min, 3);
    assert_eq!(Hive::<i32>::block_capacity_hard_limits().max, 255);

    let h = Hive::<i32>::try_new(BlockCapacityLimits::new(4, 16)).unwrap();
    assert_eq!(h.block_capacity_limits(), BlockCapacityLimits::new(4, 16));

    assert!(Hive::<i32>::try_new(BlockCapacityLimits::new(2, 16)).is_err());
    assert!(Hive::<i32>::try_new(BlockCapacityLimits::new(16, 4)).is_err());
    assert!(Hive::<i32>::try_new(BlockCapacityLimits::new(4, 256)).is_err());

    assert_eq!(Hive::<[u8; 16]>::block_capacity_hard_limits().max, u16::MAX);
    assert_eq!(Hive::<[u8; 16]>::block_capacity_default_limits().max, 8192);
    assert!(Hive::<[u8; 16]>::try_new(BlockCapacityLimits::new(4, 8192)).is_ok());
}

#[test]
fn test_max_size_and_allocator_access() {
    let h: Hive<i32> = Hive::new();
    assert!(h.max_size() > 0);
    let _ = h.get_allocator();
}

#[test]
fn test_reshape_updates_limits_and_preserves_values() {
    let mut h = Hive::<i32>::try_new(BlockCapacityLimits::new(4, 16)).unwrap();
    for i in 0..40 {
        h.insert(i);
    }

    h.reshape(BlockCapacityLimits::new(8, 32)).unwrap();
    assert_eq!(h.block_capacity_limits(), BlockCapacityLimits::new(8, 32));
    assert_eq!(h.len(), 40);

    let mut vals: Vec<i32> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, (0..40).collect::<Vec<_>>());
}

#[test]
fn test_reshape_rejects_invalid_limits() {
    let mut h = Hive::<i32>::new();
    assert!(h.reshape(BlockCapacityLimits::new(1, 8)).is_err());
    assert_eq!(
        h.block_capacity_limits(),
        Hive::<i32>::block_capacity_default_limits()
    );
}

#[test]
fn test_large_reserve_uses_max_capacity_blocks() {
    let mut h: Hive<i32> = Hive::new();
    h.reserve(70_000);
    assert!(h.capacity() >= 70_000);
    assert!(h.capacity() < 80_000);
}

#[test]
fn test_clone() {
    let mut h = Hive::new();
    for i in 0..50 {
        h.insert(i);
    }
    let h2 = h.clone();
    assert_eq!(h.len(), h2.len());
    let mut v1: Vec<i32> = h.iter().copied().collect();
    let mut v2: Vec<i32> = h2.iter().copied().collect();
    v1.sort();
    v2.sort();
    assert_eq!(v1, v2);
}

#[test]
fn test_from_iterator() {
    let h: Hive<i32> = (0..50).collect();
    assert_eq!(h.len(), 50);
    let mut v: Vec<i32> = h.iter().copied().collect();
    v.sort();
    assert_eq!(v, (0..50).collect::<Vec<_>>());
}

#[test]
fn test_extend() {
    let mut h = Hive::new();
    h.extend(0..30);
    assert_eq!(h.len(), 30);
    h.extend(30..60);
    assert_eq!(h.len(), 60);
}

#[test]
fn test_retain_even() {
    let mut h = Hive::new();
    for i in 0..20 {
        h.insert(i);
    }
    h.retain(|&x| x % 2 == 0);
    let vals: Vec<i32> = h.iter().copied().collect();
    for v in &vals {
        assert!(v % 2 == 0);
    }
}

#[test]
fn test_retain_all() {
    let mut h = Hive::new();
    for i in 0..50 {
        h.insert(i);
    }
    h.retain(|_| true);
    assert_eq!(h.len(), 50);
}

#[test]
fn test_retain_none() {
    let mut h = Hive::new();
    for i in 0..50 {
        h.insert(i);
    }
    h.retain(|_| false);
    assert!(h.is_empty());
}

#[test]
fn test_sort() {
    let mut h = Hive::new();
    for i in (0..100).rev() {
        h.insert(i);
    }
    h.sort();
    let vals: Vec<i32> = h.iter().copied().collect();
    for w in vals.windows(2) {
        assert!(w[0] <= w[1]);
    }
}

#[test]
fn test_sort_preserves_element_locations() {
    let mut h = Hive::new();
    let p1 = h.insert(3) as *mut i32;
    let p2 = h.insert(1) as *mut i32;
    let p3 = h.insert(2) as *mut i32;
    let ptrs = [p1, p2, p3];

    h.sort();

    let live_ptrs: Vec<*const i32> = h.iter().map(|v| v as *const i32).collect();
    for ptr in ptrs {
        assert!(live_ptrs.contains(&(ptr as *const i32)));
    }
    assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
}

#[test]
fn test_sort_by_desc() {
    let mut h = Hive::new();
    for i in 0..100 {
        h.insert(i);
    }
    h.sort_by(|a, b| b.cmp(a));
    let vals: Vec<i32> = h.iter().copied().collect();
    for w in vals.windows(2) {
        assert!(w[0] >= w[1]);
    }
}

#[test]
fn test_sort_by_panic_leaves_hive_valid() {
    let drops = Rc::new(Cell::new(0));

    #[derive(Debug)]
    struct Tracked {
        value: i32,
        drops: Rc<Cell<usize>>,
    }

    impl Drop for Tracked {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    let mut h = Hive::new();
    let ptrs: Vec<*const Tracked> = (0..16)
        .rev()
        .map(|value| {
            h.insert(Tracked {
                value,
                drops: drops.clone(),
            })
        })
        .collect();

    let mut comparisons = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.sort_by(|a, b| {
            comparisons += 1;
            if comparisons == 5 {
                panic!("comparator failed");
            }
            a.value.cmp(&b.value)
        });
    }));

    assert!(result.is_err());
    assert_eq!(h.len(), 16);
    assert_eq!(drops.get(), 0);
    for ptr in ptrs {
        assert!(unsafe { h.get(ptr) }.is_some());
    }
    let mut vals: Vec<i32> = h.iter().map(|v| v.value).collect();
    vals.sort();
    assert_eq!(vals, (0..16).collect::<Vec<_>>());

    drop(h);
    assert_eq!(drops.get(), 16);
}

#[test]
fn test_stable_references() {
    let mut h = Hive::new();
    let r1 = h.insert(10);
    let r2 = h.insert(20);
    let r3 = h.insert(30);

    for i in 0..1000 {
        h.insert(i);
    }

    unsafe {
        assert_eq!(*r1, 10);
        assert_eq!(*r2, 20);
        assert_eq!(*r3, 30);
    }
}

#[test]
fn test_iter_mut() {
    let mut h = Hive::new();
    for i in 0..10 {
        h.insert(i);
    }
    for val in h.iter_mut() {
        *val *= 2;
    }
    let mut vals: Vec<i32> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, vec![0, 2, 4, 6, 8, 10, 12, 14, 16, 18]);
}

#[test]
fn test_into_iter() {
    let mut h = Hive::new();
    for i in 0..50 {
        h.insert(i);
    }
    let vals: Vec<i32> = h.into_iter().collect();
    assert_eq!(vals.len(), 50);
}

#[test]
fn test_into_iter_drops_remaining_elements() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct DropCounter;
    impl Drop for DropCounter {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    DROP_COUNT.store(0, Ordering::SeqCst);
    {
        let mut h = Hive::new();
        for _ in 0..10 {
            h.insert(DropCounter);
        }
        let mut iter = h.into_iter();
        let _ = iter.next();
    }
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 10);
}

#[test]
fn test_exact_size() {
    let mut h = Hive::new();
    for i in 0..30 {
        h.insert(i);
    }
    assert_eq!(h.iter().len(), 30);
}

#[test]
fn test_drop_non_trivial() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    struct DropCounter {
        _val: i32,
    }
    impl Drop for DropCounter {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    DROP_COUNT.store(0, Ordering::SeqCst);
    {
        let mut h: Hive<DropCounter> = Hive::new();
        for i in 0..100 {
            h.insert(DropCounter { _val: i });
        }
    }
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 100);
}

#[test]
fn test_stress_insert_erase() {
    for _ in 0..10 {
        let mut h = Hive::new();
        let mut refs = Vec::new();

        for i in 0..2000 {
            let r = h.insert(i);
            refs.push(r);
        }

        for r in &refs {
            unsafe {
                h.erase(*r);
            }
        }
        assert!(h.is_empty());
    }
}

#[test]
fn test_alternating_erase() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..500).map(|i| h.insert(i)).collect();

    for i in (0..500).step_by(2) {
        unsafe {
            h.erase(refs[i]);
        }
    }

    assert_eq!(h.len(), 250);
    let mut vals: Vec<i32> = h.iter().copied().collect();
    vals.sort();
    let expected: Vec<i32> = (1..500).step_by(2).collect();
    assert_eq!(vals, expected);
}

#[test]
fn test_erase_and_reinsert_reuse() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..200).map(|i| h.insert(i)).collect();

    // Erase every 3rd
    for i in (0..200).step_by(3) {
        unsafe {
            h.erase(refs[i]);
        }
    }

    // Re-insert new values (should reuse erased slots)
    for i in 1000..1100 {
        h.insert(i);
    }

    use std::collections::HashSet;
    let vals: HashSet<i32> = h.iter().copied().collect();
    for i in 0..200 {
        if i % 3 != 0 {
            assert!(vals.contains(&i), "missing {i}");
        }
    }
    for i in 1000..1100 {
        assert!(vals.contains(&i), "missing {i}");
    }
}

#[test]
fn test_reinsert_inside_merged_skipblock() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..8).map(|i| h.insert(i)).collect();

    unsafe {
        h.erase(refs[2]);
    }
    unsafe {
        h.erase(refs[4]);
    }
    unsafe {
        h.erase(refs[3]);
    }

    h.insert(100);
    h.insert(101);

    let mut vals: Vec<i32> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, vec![0, 1, 5, 6, 7, 100, 101]);
}

#[test]
fn test_small_type_u8() {
    let mut h = Hive::new();
    for i in 0u8..100 {
        h.insert(i);
    }
    assert_eq!(h.len(), 100);
    let mut vals: Vec<u8> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, (0u8..100).collect::<Vec<_>>());
}

#[test]
fn test_small_type_reuse_near_u8_capacity_limit() {
    let mut h = Hive::try_new(BlockCapacityLimits::new(255, 255)).unwrap();
    let refs: Vec<*const u8> = (0..255).map(|i| h.insert(i as u8)).collect();

    for idx in 100..255 {
        unsafe {
            h.erase(refs[idx]);
        }
    }

    let reused: Vec<*const u8> = (0..155).map(|i| h.insert(i as u8)).collect();

    assert_eq!(reused.first().copied(), Some(refs[100]));
    assert_eq!(reused.last().copied(), Some(refs[254]));
    assert_eq!(h.len(), 255);
}

#[test]
fn test_rev_iter() {
    let mut h = Hive::new();
    for i in 0..5 {
        h.insert(i);
    }
    let rev: Vec<i32> = h.iter().rev().copied().collect();
    assert_eq!(rev, vec![4, 3, 2, 1, 0]);
}

#[test]
fn test_nth() {
    let mut h = Hive::new();
    for i in 0..100 {
        h.insert(i);
    }
    assert_eq!(h.iter().copied().nth(50), Some(50));
    assert_eq!(h.iter().copied().nth(99), Some(99));
    assert_eq!(h.iter().copied().nth(100), None);
}

#[test]
fn test_fused() {
    let mut h = Hive::new();
    h.insert(1);
    let mut it = h.iter();
    assert_eq!(it.next(), Some(&1));
    for _ in 0..5 {
        assert_eq!(it.next(), None);
    }
}

#[test]
fn test_last() {
    let mut h = Hive::new();
    h.insert(1);
    h.insert(2);
    h.insert(3);
    assert_eq!(h.iter().next_back(), Some(&3));
}

#[test]
fn test_default() {
    let h: Hive<i32> = Default::default();
    assert!(h.is_empty());
}

#[test]
fn test_debug() {
    let mut h = Hive::new();
    h.insert(1);
    h.insert(2);
    let s = format!("{h:?}");
    assert!(s.contains('1'));
    assert!(s.contains('2'));
}

#[test]
fn test_trim_capacity() {
    let mut h: Hive<i32> = Hive::new();
    h.reserve(500);
    let cap_before = h.capacity();
    h.trim_capacity();
    assert!(h.capacity() <= cap_before);
}

#[test]
fn test_trim_capacity_to() {
    let mut h: Hive<i32> = Hive::new();
    h.reserve(500);
    let cap_before = h.capacity();
    h.trim_capacity_to(100);
    assert!(h.capacity() <= cap_before);
    assert!(h.capacity() >= 100 || h.capacity() == 0);
}

#[test]
fn test_get_and_iter_from_pointer() {
    let mut h = Hive::new();
    let p1 = h.insert(1);
    let p2 = h.insert(2);
    h.insert(3);

    assert_eq!(unsafe { h.get(p2) }, Some(&2));
    assert_eq!(
        unsafe { h.iter_from(p2) }
            .unwrap()
            .copied()
            .collect::<Vec<_>>(),
        vec![2, 3]
    );

    unsafe {
        h.erase(p1);
    }
    assert_eq!(unsafe { h.get(p1) }, None);
}

#[test]
fn test_get_mut_and_iter_mut_from_pointer() {
    let mut h = Hive::new();
    h.insert(1);
    let p2 = h.insert(2);
    h.insert(3);

    *unsafe { h.get_mut(p2) }.unwrap() = 20;
    for value in unsafe { h.iter_mut_from(p2) }.unwrap() {
        *value += 1;
    }

    assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![1, 21, 4]);
}

#[test]
fn test_assign_and_assign_from_iter() {
    let mut h = Hive::new();
    h.extend(0..10);
    h.assign(3, 7);
    assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![7, 7, 7]);

    h.assign_from_iter([1, 2, 3, 4]);
    assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3, 4]);
}

#[test]
fn test_insert_many() {
    let mut h = Hive::new();
    h.insert(1);
    h.insert_many([2, 3, 4]);
    assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3, 4]);
}

#[test]
fn test_unique() {
    let mut h = Hive::new();
    h.extend([1, 1, 2, 2, 2, 3, 1, 1]);
    assert_eq!(h.unique(), 4);
    assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3, 1]);
}

#[test]
fn test_unique_by() {
    let mut h = Hive::new();
    h.extend([1, 3, 4, 6, 7]);
    assert_eq!(h.unique_by(|a, b| a % 2 == b % 2), 2);
    assert_eq!(h.iter().copied().collect::<Vec<_>>(), vec![1, 4, 7]);
}

#[test]
fn test_pool_insert_returns_mutable_guard() {
    let pool = Pool::new();
    let mut value = pool.insert(41);

    *value += 1;

    assert_eq!(*value, 42);
    assert_eq!(pool.len(), 1);
    assert!(!pool.is_empty());
}

#[test]
fn test_pool_emplace_returns_mutable_guard() {
    let pool = Pool::new();
    let mut value = pool.emplace(|| String::from("hello"));

    value.push_str(" pool");

    assert_eq!(&*value, "hello pool");
    assert_eq!(pool.len(), 1);
}

#[test]
fn test_pool_emplace_panic_leaves_pool_unchanged() {
    let pool = Pool::new();
    let value = pool.insert(1);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pool.emplace(|| -> i32 { panic!("construction failed") });
    }));

    assert!(result.is_err());
    assert_eq!(*value, 1);
    assert_eq!(pool.len(), 1);
}

#[test]
fn test_pool_insert_with_default_initialized_slot() {
    let pool = Pool::new();
    let value = pool.insert_with(|value: &mut String| {
        value.push_str("pool value");
    });

    assert_eq!(&*value, "pool value");
    assert_eq!(pool.len(), 1);
}

#[test]
fn test_pool_insert_with_panic_erases_default_value() {
    let pool = Pool::new();
    let value = pool.insert(1);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pool.insert_with(|candidate: &mut i32| {
            *candidate = 2;
            panic!("initialization failed");
        });
    }));

    assert!(result.is_err());
    assert_eq!(*value, 1);
    assert_eq!(pool.len(), 1);
}

#[test]
fn test_pool_allows_multiple_live_guards_and_insertions() {
    let pool = Pool::new();
    let mut first = pool.insert(String::from("first"));
    let mut second = pool.insert(String::from("second"));

    first.push_str(" value");
    second.push_str(" value");
    let third = pool.insert(String::from("third"));

    assert_eq!(&*first, "first value");
    assert_eq!(&*second, "second value");
    assert_eq!(&*third, "third");
    assert_eq!(pool.len(), 3);
}

#[test]
fn test_pool_guard_erases_on_drop() {
    let pool = Pool::new();
    let first = pool.insert(1);
    let second = pool.insert(2);
    assert_eq!(pool.len(), 2);

    drop(first);
    assert_eq!(pool.len(), 1);

    drop(second);
    assert!(pool.is_empty());
}

#[test]
fn test_pool_reuses_erased_slot() {
    let pool = Pool::with_capacity(1);
    let first = pool.insert(1);
    let first_addr = (&*first) as *const i32 as usize;

    drop(first);
    let second = pool.insert(2);
    let second_addr = (&*second) as *const i32 as usize;

    assert_eq!(*second, 2);
    assert_eq!(first_addr, second_addr);
    assert_eq!(pool.len(), 1);
}

#[test]
fn test_pool_new_in_uses_allocator() {
    let pool = Pool::<i32, Global>::new_in(Global);
    let value = pool.insert(7);

    assert_eq!(*value, 7);
    assert_eq!(pool.len(), 1);
}

#[test]
fn test_pool_with_capacity_in_uses_allocator() {
    let pool = Pool::<i32, Global>::with_capacity_in(10, Global);

    assert!(pool.capacity() >= 10);
}

#[test]
fn test_pool_try_new_in_validates_limits() {
    let pool = Pool::<i32, Global>::try_new_in(Global, BlockCapacityLimits::new(4, 16)).unwrap();
    let value = pool.insert(11);

    assert_eq!(*value, 11);
    assert!(Pool::<i32, Global>::try_new_in(Global, BlockCapacityLimits::new(2, 16)).is_err());
}

#[cfg(feature = "std")]
#[test]
fn test_sync_pool_guard_can_move_to_thread() {
    let pool = SyncPool::new();
    let value = pool.insert(41);

    let value = std::thread::spawn(move || {
        let mut value = value;
        *value += 1;
        assert_eq!(*value, 42);
        value
    })
    .join()
    .unwrap();

    assert_eq!(*value, 42);
    assert_eq!(pool.len(), 1);
    drop(value);
    assert!(pool.is_empty());
}

#[cfg(feature = "std")]
#[test]
fn test_sync_pool_emplace_returns_mutable_guard() {
    let pool = SyncPool::new();
    let value = pool.emplace(|| String::from("hello"));

    let value = std::thread::spawn(move || {
        let mut value = value;
        value.push_str(" sync");
        value
    })
    .join()
    .unwrap();

    assert_eq!(&*value, "hello sync");
    assert_eq!(pool.len(), 1);
}

#[cfg(feature = "std")]
#[test]
fn test_sync_pool_emplace_panic_leaves_pool_unchanged() {
    let pool = SyncPool::new();
    let value = pool.insert(1);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pool.emplace(|| -> i32 { panic!("construction failed") });
    }));

    assert!(result.is_err());
    assert_eq!(*value, 1);
    assert_eq!(pool.len(), 1);
}

#[cfg(feature = "std")]
#[test]
fn test_sync_pool_insert_with_default_initialized_slot() {
    let pool = SyncPool::new();
    let value = pool.insert_with(|value: &mut String| {
        value.push_str("sync value");
    });

    assert_eq!(&*value, "sync value");
    assert_eq!(pool.len(), 1);
}

#[cfg(feature = "std")]
#[test]
fn test_sync_pool_insert_with_panic_erases_default_value() {
    let pool = SyncPool::new();
    let value = pool.insert(1);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pool.insert_with(|candidate: &mut i32| {
            *candidate = 2;
            panic!("initialization failed");
        });
    }));

    assert!(result.is_err());
    assert_eq!(*value, 1);
    assert_eq!(pool.len(), 1);
}

#[cfg(feature = "std")]
#[test]
fn test_sync_pool_concurrent_insert_and_drop() {
    let pool = SyncPool::with_capacity(16);
    let mut threads = Vec::new();

    for thread_id in 0..4 {
        let pool = pool.clone();
        threads.push(std::thread::spawn(move || {
            for i in 0..100 {
                let mut value = pool.insert(thread_id * 100 + i);
                *value += 1;
                assert_eq!(*value, thread_id * 100 + i + 1);
            }
        }));
    }

    for thread in threads {
        thread.join().unwrap();
    }

    assert!(pool.is_empty());
}

#[cfg(feature = "std")]
#[test]
fn test_sync_pool_live_guards_across_threads() {
    let pool = SyncPool::new();
    let mut first = pool.insert(String::from("first"));
    let second = pool.insert(String::from("second"));

    let thread = std::thread::spawn(move || {
        let mut second = second;
        second.push_str(" updated");
        assert_eq!(&*second, "second updated");
        second
    });

    first.push_str(" updated");
    let second = thread.join().unwrap();

    assert_eq!(&*first, "first updated");
    assert_eq!(&*second, "second updated");
    assert_eq!(pool.len(), 2);
}

#[cfg(feature = "std")]
#[test]
fn test_sync_pool_allocator_constructors() {
    let pool = SyncPool::<i32, Global>::with_capacity_in(8, Global);
    assert!(pool.capacity() >= 8);

    let value = pool.insert(5);
    assert_eq!(*value, 5);

    assert!(SyncPool::<i32, Global>::try_new_in(Global, BlockCapacityLimits::new(2, 16)).is_err());
}

#[derive(Clone)]
struct DropCounter(Rc<Cell<usize>>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn test_pool_drops_elements_exactly_once() {
    let drops = Rc::new(Cell::new(0));
    let pool = Pool::new();
    let first = pool.insert(DropCounter(drops.clone()));
    let second = pool.insert(DropCounter(drops.clone()));

    drop(first);
    assert_eq!(drops.get(), 1);

    drop(second);
    assert_eq!(drops.get(), 2);

    drop(pool);
    assert_eq!(drops.get(), 2);
}

#[test]
fn test_pool_drop_drops_remaining_elements() {
    let drops = Rc::new(Cell::new(0));
    let pool = Pool::new();
    let first = pool.insert(DropCounter(drops.clone()));
    let second = pool.insert(DropCounter(drops.clone()));

    drop(first);
    assert_eq!(drops.get(), 1);

    std::mem::forget(second);
    drop(pool);
    assert_eq!(drops.get(), 2);
}

#[test]
fn test_splice_moves_source_elements() {
    let mut a = Hive::new();
    a.extend([1, 2]);
    let mut b = Hive::new();
    b.extend([3, 4]);

    a.splice(&mut b).unwrap();

    assert!(b.is_empty());
    assert_eq!(a.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3, 4]);
}

#[test]
fn test_splice_preserves_element_pointers() {
    let mut a = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    let a_ptrs: Vec<*const i32> = (0..6).map(|i| a.insert(i)).collect();
    let mut b = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    let b_ptrs: Vec<*const i32> = (10..16).map(|i| b.insert(i)).collect();

    a.splice(&mut b).unwrap();

    assert!(b.is_empty());
    assert_eq!(
        a.iter().copied().collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5, 10, 11, 12, 13, 14, 15]
    );
    for (i, &ptr) in a_ptrs.iter().enumerate() {
        assert_eq!(unsafe { *ptr }, i as i32);
        assert!(unsafe { a.get(ptr) }.is_some());
    }
    for (i, &ptr) in b_ptrs.iter().enumerate() {
        assert_eq!(unsafe { *ptr }, 10 + i as i32);
        assert!(unsafe { a.get(ptr) }.is_some());
    }
}

#[test]
fn test_splice_into_empty_preserves_source_pointers_and_reserved_capacity() {
    let mut a = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    a.reserve(16);
    let reserved_capacity = a.capacity();

    let mut b = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    let b_ptrs: Vec<*const i32> = (0..10).map(|i| b.insert(i)).collect();

    a.splice(&mut b).unwrap();

    assert!(b.is_empty());
    assert_eq!(a.capacity(), reserved_capacity - 8 + 16);
    assert_eq!(
        a.iter().copied().collect::<Vec<_>>(),
        (0..10).collect::<Vec<_>>()
    );
    for (i, &ptr) in b_ptrs.iter().enumerate() {
        assert_eq!(unsafe { *ptr }, i as i32);
        assert!(unsafe { a.get(ptr) }.is_some());
    }
}

#[test]
fn test_splice_reuses_old_destination_tail_gap() {
    let mut a = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    let a_ptrs: Vec<*const i32> = (0..5).map(|i| a.insert(i)).collect();
    let mut b = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    b.extend(10..14);

    a.splice(&mut b).unwrap();

    let p = a.insert(99);
    assert_eq!(p, unsafe { a_ptrs[0].add(5) });
    assert_eq!(unsafe { *p }, 99);
}

#[test]
fn test_splice_preserves_source_erased_slot_reuse() {
    let mut a = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    a.extend(0..4);

    let mut b = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    let b_ptrs: Vec<*const i32> = (10..18).map(|i| b.insert(i)).collect();
    unsafe {
        b.erase(b_ptrs[2]);
    }

    a.splice(&mut b).unwrap();

    for i in 0..4 {
        a.insert(90 + i);
    }
    let reused = a.insert(99);

    assert_eq!(reused, b_ptrs[2]);
    assert_eq!(unsafe { *reused }, 99);
}

#[test]
fn test_splice_incompatible_limits_leaves_both_hives_unchanged() {
    let mut a = Hive::try_new(BlockCapacityLimits::new(16, 16)).unwrap();
    a.extend(0..4);
    let mut b = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    let b_ptrs: Vec<*const i32> = (10..18).map(|i| b.insert(i)).collect();

    assert_eq!(a.splice(&mut b), Err(IncompatibleSplice));

    assert_eq!(a.iter().copied().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert_eq!(
        b.iter().copied().collect::<Vec<_>>(),
        vec![10, 11, 12, 13, 14, 15, 16, 17]
    );
    for (i, &ptr) in b_ptrs.iter().enumerate() {
        assert_eq!(unsafe { *ptr }, 10 + i as i32);
        assert!(unsafe { b.get(ptr) }.is_some());
    }
}

#[test]
fn test_shrink_to_fit_reduces_capacity() {
    let mut h = Hive::new();
    h.reserve(500);
    for i in 0..10 {
        h.insert(i);
    }
    let before = h.capacity();
    h.shrink_to_fit();
    assert!(h.capacity() < before);
    assert_eq!(h.len(), 10);
    let mut vals: Vec<i32> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, (0..10).collect::<Vec<_>>());
}

#[test]
fn test_shrink_to_fit_trims_reserved_without_moving_live_elements() {
    let mut h = Hive::try_new(BlockCapacityLimits::new(8, 32)).unwrap();
    let ptrs: Vec<*const i32> = (0..40).map(|i| h.insert(i)).collect();
    let live_capacity = h.capacity();
    h.reserve(100);
    assert!(h.capacity() > live_capacity);

    h.shrink_to_fit();

    assert_eq!(h.capacity(), live_capacity);
    for (i, &ptr) in ptrs.iter().enumerate() {
        assert_eq!(unsafe { *ptr }, i as i32);
    }
}

#[test]
fn test_erase_and_iterate() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..100).map(|i| h.insert(i)).collect();

    for i in (0..100).step_by(3) {
        unsafe {
            h.erase(refs[i]);
        }
    }

    // step_by(3) on 0..100: 0,3,6,...,99 → 34 indices erased
    assert_eq!(h.len(), 66);
}

#[test]
fn test_concurrent_erase_insert() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..500).map(|i| h.insert(i)).collect();

    for i in (0..500).step_by(2) {
        unsafe {
            h.erase(refs[i]);
        }
    }

    for i in 0..250 {
        h.insert(1000 + i);
    }

    assert_eq!(h.len(), 500);
    use std::collections::HashSet;
    let vals: HashSet<i32> = h.iter().copied().collect();
    for i in 0..500 {
        if i % 2 == 1 {
            assert!(vals.contains(&i));
        }
    }
    for i in 0..250 {
        assert!(vals.contains(&(1000 + i)));
    }
}

#[test]
fn test_count_consuming() {
    let mut h = Hive::new();
    for i in 0..50 {
        h.insert(i);
    }
    assert_eq!(h.iter().count(), 50);
    assert_eq!(h.len(), 50);
}

// ── erase via pointer obtained from iter_mut ──

#[test]
fn test_erase_pointer_from_iter_mut() {
    let mut h = Hive::new();
    h.insert(1);
    h.insert(2);
    h.insert(3);

    // Locate element 2 via iter_mut, capture a *mut so the provenance keeps
    // write permission, drop the &mut T, then erase by raw ptr.
    let ptr: *mut i32 = h
        .iter_mut()
        .find(|v| **v == 2)
        .map(|v| v as *mut i32)
        .expect("element 2 should exist");
    unsafe {
        h.erase(ptr);
    }
    assert_eq!(h.len(), 2);
    let mut vals: Vec<i32> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, vec![1, 3]);
}

#[test]
fn test_reuse_skipblock_from_front_in_order() {
    let mut h = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    let refs: Vec<*const i32> = (0..8).map(|i| h.insert(i)).collect();

    for &idx in &[2usize, 3, 4] {
        unsafe {
            h.erase(refs[idx]);
        }
    }

    let a = h.insert(100);
    let b = h.insert(101);
    let c = h.insert(102);

    assert_eq!([a, b, c], [refs[2], refs[3], refs[4]]);
    let mut vals: Vec<i32> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, vec![0, 1, 5, 6, 7, 100, 101, 102]);
}

#[test]
fn test_reuse_after_moving_following_skipblock_head_forward() {
    let mut h = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    let refs: Vec<*const i32> = (0..8).map(|i| h.insert(i)).collect();

    for &idx in &[3usize, 4, 2] {
        unsafe {
            h.erase(refs[idx]);
        }
    }

    let a = h.insert(100);
    let b = h.insert(101);
    let c = h.insert(102);

    assert_eq!([a, b, c], [refs[2], refs[3], refs[4]]);
}

#[test]
fn test_reuse_after_extending_previous_skipblock() {
    let mut h = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    let refs: Vec<*const i32> = (0..8).map(|i| h.insert(i)).collect();

    for &idx in &[2usize, 3, 4] {
        unsafe {
            h.erase(refs[idx]);
        }
    }

    let a = h.insert(100);
    let b = h.insert(101);
    let c = h.insert(102);

    assert_eq!([a, b, c], [refs[2], refs[3], refs[4]]);
}

#[test]
fn test_reuse_after_merging_two_skipblocks() {
    let mut h = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    let refs: Vec<*const i32> = (0..8).map(|i| h.insert(i)).collect();

    for &idx in &[2usize, 4, 3] {
        unsafe {
            h.erase(refs[idx]);
        }
    }

    let a = h.insert(100);
    let b = h.insert(101);
    let c = h.insert(102);

    assert_eq!([a, b, c], [refs[2], refs[3], refs[4]]);
}

#[test]
fn test_reuse_pointer_set_after_fragmented_erases() {
    let mut h = Hive::try_new(BlockCapacityLimits::new(16, 16)).unwrap();
    let refs: Vec<*const i32> = (0..16).map(|i| h.insert(i)).collect();
    let erased = [1usize, 2, 5, 8, 9, 10, 14];

    for &idx in &erased {
        unsafe {
            h.erase(refs[idx]);
        }
    }

    let mut reused: Vec<*const i32> = (0..erased.len())
        .map(|i| h.insert(100 + i as i32))
        .collect();
    let mut expected: Vec<*const i32> = erased.iter().map(|&idx| refs[idx]).collect();
    reused.sort();
    expected.sort();

    assert_eq!(reused, expected);
}
