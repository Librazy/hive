use hive::{BlockCapacityLimits, Hive};

#[test]
fn test_new_empty() {
    let h: Hive<i32> = Hive::new();
    assert!(h.is_empty());
    assert_eq!(h.len(), 0);
}

#[test]
fn test_insert_one() {
    let h = Hive::new();
    h.insert(42);
    assert!(!h.is_empty());
    assert_eq!(h.len(), 1);
}

#[test]
fn test_insert_with_uninit() {
    let h = Hive::new();
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
    let h = Hive::new();
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

#[test]
fn test_iter_single() {
    let h = Hive::new();
    h.insert(42);
    assert_eq!(h.iter().next(), Some(&42));
}

#[test]
fn test_iter_multiple() {
    let h = Hive::new();
    for i in 0..100 {
        h.insert(i);
    }
    let sum: i32 = h.iter().sum();
    assert_eq!(sum, 4950);
}

#[test]
fn test_double_ended_iter() {
    let h = Hive::new();
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
    let h = Hive::new();
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
        h.erase(&*r0);
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
        h.erase(&*r2);
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
        h.erase(&*r2);
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
        h.erase(&*erased);
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
            h.erase(&**r);
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
        h.erase(&*ptr);
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
    let h: Hive<i32> = Hive::with_capacity(200);
    assert!(h.capacity() >= 200);
    for i in 0..200 {
        h.insert(i);
    }
    assert_eq!(h.len(), 200);
}

#[test]
fn test_block_capacity_limit_apis() {
    assert_eq!(
        Hive::<i32>::block_capacity_default_limits(),
        BlockCapacityLimits::new(8, 8192)
    );
    assert_eq!(Hive::<i32>::block_capacity_hard_limits().min, 3);

    let h = Hive::<i32>::try_new(BlockCapacityLimits::new(4, 16)).unwrap();
    assert_eq!(h.block_capacity_limits(), BlockCapacityLimits::new(4, 16));

    assert!(Hive::<i32>::try_new(BlockCapacityLimits::new(2, 16)).is_err());
    assert!(Hive::<i32>::try_new(BlockCapacityLimits::new(16, 4)).is_err());
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
    let h = Hive::new();
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
fn test_stable_references() {
    let h = Hive::new();
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
    let h = Hive::new();
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
        let h = Hive::new();
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
    let h = Hive::new();
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
        let h: Hive<DropCounter> = Hive::new();
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
                h.erase(&**r);
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
            h.erase(&*refs[i]);
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
            h.erase(&*refs[i]);
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
        h.erase(&*refs[2]);
    }
    unsafe {
        h.erase(&*refs[4]);
    }
    unsafe {
        h.erase(&*refs[3]);
    }

    h.insert(100);
    h.insert(101);

    let mut vals: Vec<i32> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, vec![0, 1, 5, 6, 7, 100, 101]);
}

#[test]
fn test_small_type_u8() {
    let h = Hive::new();
    for i in 0u8..100 {
        h.insert(i);
    }
    assert_eq!(h.len(), 100);
    let mut vals: Vec<u8> = h.iter().copied().collect();
    vals.sort();
    assert_eq!(vals, (0u8..100).collect::<Vec<_>>());
}

#[test]
fn test_rev_iter() {
    let h = Hive::new();
    for i in 0..5 {
        h.insert(i);
    }
    let rev: Vec<i32> = h.iter().rev().copied().collect();
    assert_eq!(rev, vec![4, 3, 2, 1, 0]);
}

#[test]
fn test_nth() {
    let h = Hive::new();
    for i in 0..100 {
        h.insert(i);
    }
    assert_eq!(h.iter().copied().nth(50), Some(50));
    assert_eq!(h.iter().copied().nth(99), Some(99));
    assert_eq!(h.iter().copied().nth(100), None);
}

#[test]
fn test_fused() {
    let h = Hive::new();
    h.insert(1);
    let mut it = h.iter();
    assert_eq!(it.next(), Some(&1));
    for _ in 0..5 {
        assert_eq!(it.next(), None);
    }
}

#[test]
fn test_last() {
    let h = Hive::new();
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
    let h = Hive::new();
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
        h.erase(&*p1);
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
    let h = Hive::new();
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
fn test_splice_moves_source_elements() {
    let mut a = Hive::new();
    a.extend([1, 2]);
    let mut b = Hive::new();
    b.extend([3, 4]);

    a.splice(&mut b);

    assert!(b.is_empty());
    assert_eq!(a.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3, 4]);
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
fn test_erase_and_iterate() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..100).map(|i| h.insert(i)).collect();

    for i in (0..100).step_by(3) {
        unsafe {
            h.erase(&*refs[i]);
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
            h.erase(&*refs[i]);
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
    let h = Hive::new();
    for i in 0..50 {
        h.insert(i);
    }
    assert_eq!(h.iter().count(), 50);
    assert_eq!(h.len(), 50);
}

// ── Safe ref API tests ──

#[test]
fn test_insert_ref_basic() {
    let hive = Hive::new();
    let r = hive.insert_ref(42);
    assert_eq!(*r, 42);
    assert_eq!(hive.len(), 1);
}

#[test]
fn test_insert_ref_multiple() {
    let hive = Hive::new();
    let a = hive.insert_ref(1);
    let b = hive.insert_ref(2);
    let c = hive.insert_ref(3);
    // All live simultaneously through &self
    assert_eq!(*a, 1);
    assert_eq!(*b, 2);
    assert_eq!(*c, 3);
    assert_eq!(hive.len(), 3);

    // Iteration still works while refs are held
    let sum: i32 = hive.iter().sum();
    assert_eq!(sum, 6);
}

#[test]
fn test_insert_ref_stable_under_insertion() {
    let hive = Hive::new();
    let a = hive.insert_ref(10);
    let b = hive.insert_ref(20);

    // Insert many more — a and b remain valid
    for i in 0..1000 {
        hive.insert_ref(i);
    }
    assert_eq!(*a, 10);
    assert_eq!(*b, 20);
}

#[test]
fn test_insert_ref_and_iterate() {
    let hive = Hive::new();
    let markers: Vec<&i32> = (0..10).map(|i| hive.insert_ref(i)).collect();

    // Iteration sees all elements regardless of outstanding refs
    let sum: i32 = hive.iter().sum();
    assert_eq!(sum, 45);

    // And all original refs are still valid
    for (i, r) in markers.iter().enumerate() {
        assert_eq!(**r, i as i32);
    }
}

#[test]
fn test_insert_ref_then_erase() {
    let mut hive = Hive::new();

    {
        let a = hive.insert_ref(1);
        let b = hive.insert_ref(2);
        assert_eq!(*a, 1);
        assert_eq!(*b, 2);
        // a, b dropped here — now &mut self is available
    }

    // Can erase using pointer obtained previously
    let p = hive.insert(99);
    unsafe {
        hive.erase(&*p);
    }
    assert_eq!(hive.len(), 2);
}

#[test]
fn test_insert_ref_no_clone_needed() {
    // insert_ref takes T by value with &self — no Clone required
    let hive = Hive::new();
    let r = hive.insert_ref(String::from("hello"));
    assert_eq!(r, "hello");
}

#[test]
fn test_insert_ref_mut() {
    let mut hive = Hive::new();
    let r = hive.insert_ref_mut(42);
    *r = 99;
    assert_eq!(hive.iter().next(), Some(&99));
}

#[test]
fn test_insert_ref_mut_single_only() {
    let mut hive = Hive::new();
    let r = hive.insert_ref_mut(1);
    // Can't insert again while r borrows &mut self
    // hive.insert(2); // would not compile
    *r = 10;
    assert_eq!(*r, 10);
}

#[test]
fn test_insert_ref_with_capacity() {
    let hive = Hive::with_capacity(500);
    let refs: Vec<&i32> = (0..500).map(|i| hive.insert_ref(i)).collect();
    assert_eq!(hive.len(), 500);
    for (i, r) in refs.iter().enumerate() {
        assert_eq!(**r, i as i32);
    }
}

#[test]
fn test_insert_ref_erase_multiple() {
    let mut hive = Hive::new();
    let ptrs: Vec<*const i32> = (0..100).map(|i| hive.insert(i)).collect();

    // Erase half
    for i in (0..100).step_by(2) {
        unsafe {
            hive.erase(&*ptrs[i]);
        }
    }

    // Re-insert via safe API
    for i in 200..250 {
        hive.insert_ref(i);
    }

    assert_eq!(hive.len(), 100);
    use std::collections::HashSet;
    let vals: HashSet<i32> = hive.iter().copied().collect();
    for i in 0..100 {
        if i % 2 == 1 {
            assert!(vals.contains(&i), "missing {i}");
        }
    }
    for i in 200..250 {
        assert!(vals.contains(&i), "missing {i}");
    }
}
