use hive::{BlockCapacityLimits, Hive};
use std::cell::Cell;
use std::rc::Rc;

fn filled(n: i32) -> Hive<i32> {
    let mut hive = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    hive.extend(0..n);
    hive
}

fn check_iterators(mut hive: Hive<i32>, expected: &[i32]) {
    assert_eq!(hive.len(), expected.len());
    assert_eq!(hive.iter().copied().collect::<Vec<_>>(), expected);
    assert_eq!(
        hive.iter().rev().copied().collect::<Vec<_>>(),
        expected.iter().rev().copied().collect::<Vec<_>>()
    );
    let mut iter = hive.iter();
    let mut reference = expected.iter();
    let mut back = false;
    while reference.len() > 0 {
        assert_eq!(
            if back { iter.next_back() } else { iter.next() },
            if back {
                reference.next_back()
            } else {
                reference.next()
            }
        );
        back = !back;
    }
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next_back(), None);
    // Hold all mutable references at once, also checking uniqueness under Miri.
    let refs: Vec<_> = hive.iter_mut().rev().collect();
    for (value, expected) in refs.into_iter().zip(expected.iter().rev()) {
        assert_eq!(*value, *expected);
        *value += 100;
    }
    assert_eq!(
        hive.into_iter().rev().collect::<Vec<_>>(),
        expected.iter().rev().map(|v| v + 100).collect::<Vec<_>>()
    );
}

#[test]
fn reverse_crosses_erased_prefix() {
    let mut hive = filled(16);
    hive.retain(|v| *v < 8 || *v >= 10);
    check_iterators(hive, &[0, 1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15]);
}

#[test]
fn reverse_crosses_erased_suffix() {
    for last in [0, 3, 6] {
        let mut hive = filled(16);
        hive.retain(|v| *v == last || *v == 8);
        check_iterators(hive, &[last, 8]);
    }
}

#[test]
fn removing_head_skips_new_heads_erased_prefix() {
    let mut hive = filled(24);
    hive.retain(|v| *v != 8 && *v != 9);
    hive.retain(|v| *v >= 8);
    check_iterators(hive, &(10..24).collect::<Vec<_>>());
}

#[test]
fn removing_tail_repairs_end_before_iteration_and_splice() {
    for surviving in [1, 4, 8, 9, 16] {
        let mut hive = filled(17);
        hive.retain(|v| *v < surviving);
        assert_eq!(
            hive.iter().rev().copied().collect::<Vec<_>>(),
            (0..surviving).rev().collect::<Vec<_>>()
        );
        hive.trim_capacity(); // Release removed groups before using the cursor.
        let mut source = filled(3);
        source.iter_mut().for_each(|v| *v += 1000);
        hive.splice(&mut source).unwrap();
        assert!(source.is_empty());
        let mut expected: Vec<_> = (0..surviving).collect();
        expected.extend(1000..1003);
        check_iterators(hive, &expected);
    }
}

#[test]
fn removed_tail_and_erased_suffix_can_be_reused() {
    let mut hive = filled(17);
    hive.retain(|v| *v < 3);
    hive.extend(100..125);
    let mut actual = hive.iter().copied().collect::<Vec<_>>();
    actual.sort_unstable();
    let expected: Vec<_> = (0..3).chain(100..125).collect();
    assert_eq!(actual, expected);
    let order = hive.iter().copied().collect::<Vec<_>>();
    check_iterators(hive, &order);
}

#[test]
fn unique_removing_tail_preserves_owned_values() {
    let mut hive = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    hive.extend((0..9).map(|_| Box::new(42)));
    assert_eq!(hive.unique(), 8);
    assert_eq!(**hive.iter().next_back().unwrap(), 42);
    assert_eq!(*hive.into_iter().next_back().unwrap(), 42);
}

#[test]
fn head_and_tail_removal_drop_each_value_once() {
    struct Tracked(i32, Rc<Cell<usize>>);
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.1.set(self.1.get() + 1);
        }
    }
    let drops = Rc::new(Cell::new(0));
    let mut hive = Hive::try_new(BlockCapacityLimits::new(8, 8)).unwrap();
    hive.extend((0..25).map(|v| Tracked(v, drops.clone())));
    hive.retain(|v| v.0 != 8 && v.0 != 9);
    hive.retain(|v| v.0 >= 8 && v.0 < 16);
    assert_eq!(
        hive.iter().rev().map(|v| v.0).collect::<Vec<_>>(),
        (10..16).rev().collect::<Vec<_>>()
    );
    hive.clear();
    assert_eq!(drops.get(), 25);
    hive.insert(Tracked(100, drops.clone()));
    drop(hive);
    assert_eq!(drops.get(), 26);
}

#[cfg(target_pointer_width = "32")]
#[test]
#[should_panic(expected = "element allocation size overflow")]
fn large_element_allocation_rejects_32_bit_overflow() {
    let mut hive = Hive::<[u8; 65540]>::try_new(BlockCapacityLimits::new(65535, 65535)).unwrap();
    hive.reserve(1);
}

#[test]
fn removing_spliced_tail_preserves_unused_suffix_as_erased() {
    let mut hive = filled(3);
    let mut source = filled(1);
    *source.iter_mut().next().unwrap() = 1000;
    hive.splice(&mut source).unwrap();
    hive.retain(|v| *v < 1000);
    hive.extend(100..110);
    check_iterators(hive, &(0..3).chain(100..110).collect::<Vec<_>>());
}
