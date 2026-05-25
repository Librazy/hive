#![feature(test)]

extern crate test;

use hive::Hive;
use std::collections::{LinkedList, VecDeque};
use test::Bencher;

const N: usize = 100_000;

// ── Append (push back) ──

#[bench]
fn bench_hive_append(b: &mut Bencher) {
    b.iter(|| {
        let h = Hive::with_capacity(N);
        for i in 0..N {
            h.insert(i);
        }
        test::black_box(h);
    });
}

#[bench]
fn bench_vec_push(b: &mut Bencher) {
    b.iter(|| {
        let mut v = Vec::with_capacity(N);
        for i in 0..N {
            v.push(i);
        }
        test::black_box(v);
    });
}

#[bench]
fn bench_linkedlist_push_back(b: &mut Bencher) {
    b.iter(|| {
        let mut l = LinkedList::new();
        for i in 0..N {
            l.push_back(i);
        }
        test::black_box(l);
    });
}

#[bench]
fn bench_vecdeque_push_back(b: &mut Bencher) {
    b.iter(|| {
        let mut d = VecDeque::with_capacity(N);
        for i in 0..N {
            d.push_back(i);
        }
        test::black_box(d);
    });
}

// ── Iteration (sum) ──

#[bench]
fn bench_hive_iter_sum(b: &mut Bencher) {
    let h = Hive::with_capacity(N);
    for i in 0..N {
        h.insert(i);
    }
    b.iter(|| {
        let sum: u64 = h.iter().map(|&x| x as u64).sum();
        test::black_box(sum);
    });
}

#[bench]
fn bench_vec_iter_sum(b: &mut Bencher) {
    let v: Vec<u64> = (0..N as u64).collect();
    b.iter(|| {
        let sum: u64 = v.iter().sum();
        test::black_box(sum);
    });
}

#[bench]
fn bench_linkedlist_iter_sum(b: &mut Bencher) {
    let l: LinkedList<u64> = (0..N as u64).collect();
    b.iter(|| {
        let sum: u64 = l.iter().sum();
        test::black_box(sum);
    });
}

// ── Erase from middle ──

#[bench]
fn bench_hive_erase(b: &mut Bencher) {
    let mut h = Hive::with_capacity(N);
    let ptrs: Vec<*const u64> = (0..N as u64).map(|i| h.insert(i)).collect();
    b.iter(|| {
        // Erase every 10th element
        for i in (0..N).step_by(10) {
            unsafe { h.erase(&*ptrs[i]); }
        }
        // Re-insert to restore size
        for i in (0..N).step_by(10) {
            h.insert(i as u64 + N as u64);
        }
        test::black_box(&mut h);
    });
}

#[bench]
fn bench_vec_remove(b: &mut Bencher) {
    b.iter(|| {
        let mut v: Vec<u64> = (0..N as u64).collect();
        // Remove every 10th from the end (least expensive for Vec)
        for i in (0..N).step_by(10).rev() {
            v.remove(i);
        }
        // Re-insert
        for i in (0..N).step_by(10) {
            v.push(i as u64 + N as u64);
        }
        test::black_box(v);
    });
}

#[bench]
fn bench_linkedlist_remove(b: &mut Bencher) {
    b.iter(|| {
        let mut l: LinkedList<u64> = (0..N as u64).collect();
        let mut split = l.split_off(N / 2);
        // Remove every 10th from the front half by draining
        let mut l: LinkedList<u64> = l
            .into_iter()
            .enumerate()
            .filter(|(j, _)| j % 10 != 0)
            .map(|(_, v)| v)
            .collect();
        // Re-insert
        for j in (0..l.len() + split.len()).step_by(10).take(N / (10 * 2)) {
            l.push_back(j as u64 + N as u64);
        }
        l.append(&mut split);
        test::black_box(l);
    });
}

// ── Mixed insert + erase cycle (stable reference scenario) ──

#[bench]
fn bench_hive_mixed(b: &mut Bencher) {
    b.iter(|| {
        let mut h = Hive::with_capacity(2000);
        let mut ptrs = Vec::new();

        for i in 0..2000 {
            ptrs.push(h.insert(i));
        }
        for (i, &p) in ptrs.iter().enumerate().take(1000) {
            unsafe { h.erase(&*p); }
            h.insert(i + 10_000);
        }
        test::black_box(h);
    });
}

#[bench]
fn bench_vec_mixed(b: &mut Bencher) {
    b.iter(|| {
        let mut v: Vec<usize> = (0..2000).collect();
        for i in (0..1000).rev() {
            v.remove(i);
            v.push(i + 10_000);
        }
        test::black_box(v);
    });
}

// ── Random access (by key, not index) ──

#[bench]
fn bench_hive_random_access(b: &mut Bencher) {
    let h = Hive::with_capacity(N);
    let ptrs: Vec<*const u64> = (0..N as u64).map(|i| h.insert(i)).collect();
    b.iter(|| {
        let mut sum: u64 = 0;
        for chunk in ptrs.chunks(10) {
            for &p in chunk {
                sum += unsafe { *p };
            }
        }
        test::black_box(sum);
    });
}

#[bench]
fn bench_vec_random_access(b: &mut Bencher) {
    let v: Vec<u64> = (0..N as u64).collect();
    b.iter(|| {
        let mut sum: u64 = 0;
        for chunk in v.chunks(10) {
            for &x in chunk {
                sum += x;
            }
        }
        test::black_box(sum);
    });
}
