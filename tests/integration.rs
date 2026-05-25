use hive::Hive;

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
fn test_erase_basic() {
    let mut h = Hive::new();
    let r0 = h.insert(42);
    let _r1 = h.insert(99);
    assert_eq!(h.len(), 2);

    unsafe { h.erase(&*r0); }
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

    unsafe { h.erase(&*r2); }

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

    unsafe { h.erase(&*r2); }
    h.insert(4);

    assert_eq!(h.len(), 3);
    let mut sorted: Vec<i32> = h.iter().copied().collect();
    sorted.sort();
    assert_eq!(sorted, vec![1, 3, 4]);
}

#[test]
fn test_erase_all_one_by_one() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..100).map(|i| h.insert(i)).collect();

    for r in &refs {
        unsafe { h.erase(&**r); }
    }
    assert!(h.is_empty());
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
    struct DropCounter { _val: i32 }
    impl Drop for DropCounter {
        fn drop(&mut self) { DROP_COUNT.fetch_add(1, Ordering::SeqCst); }
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
            unsafe { h.erase(&**r); }
        }
        assert!(h.is_empty());
    }
}

#[test]
fn test_alternating_erase() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..500).map(|i| h.insert(i)).collect();

    for i in (0..500).step_by(2) {
        unsafe { h.erase(&*refs[i]); }
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
        unsafe { h.erase(&*refs[i]); }
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
fn test_erase_and_iterate() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..100).map(|i| h.insert(i)).collect();

    for i in (0..100).step_by(3) {
        unsafe { h.erase(&*refs[i]); }
    }

    // step_by(3) on 0..100: 0,3,6,...,99 → 34 indices erased
    assert_eq!(h.len(), 66);
}

#[test]
fn test_concurrent_erase_insert() {
    let mut h = Hive::new();
    let refs: Vec<*const i32> = (0..500).map(|i| h.insert(i)).collect();

    for i in (0..500).step_by(2) {
        unsafe { h.erase(&*refs[i]); }
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
