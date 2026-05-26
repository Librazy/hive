use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use hive::Hive;
use std::collections::{LinkedList, VecDeque};
use std::time::{Duration, Instant};

const LARGE_N: usize = 100_000;
const MIXED_N: usize = 2_000;
const INSERT_PATH_N: usize = 10_000;

#[derive(Clone, Default)]
struct NonTrivialElement {
    name: String,
    data: Vec<u64>,
    checksum: u64,
}

impl NonTrivialElement {
    fn new(i: usize) -> Self {
        let data = vec![i as u64, i as u64 + 1, i as u64 + 2, i as u64 + 3];
        let checksum = data.iter().copied().fold(i as u64, u64::wrapping_add);
        Self {
            name: format!("element-{i}"),
            data,
            checksum,
        }
    }

    fn reset(&mut self, i: usize) {
        self.name.clear();
        self.name.push_str("element-");
        self.name.push_str(&i.to_string());
        self.data.clear();
        self.data
            .extend([i as u64, i as u64 + 1, i as u64 + 2, i as u64 + 3]);
        self.checksum = self.data.iter().copied().fold(i as u64, u64::wrapping_add);
    }
}

fn bench_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("append");
    group.bench_function(BenchmarkId::new("Hive::insert", LARGE_N), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let mut h = Hive::with_capacity(LARGE_N);
                for i in 0..LARGE_N {
                    h.insert(black_box(i));
                }
                black_box(&mut h);
                elapsed += start.elapsed();
            }
            elapsed
        });
    });
    group.bench_function(BenchmarkId::new("Vec::push", LARGE_N), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let mut v = Vec::with_capacity(LARGE_N);
                for i in 0..LARGE_N {
                    v.push(black_box(i));
                }
                black_box(&mut v);
                elapsed += start.elapsed();
            }
            elapsed
        });
    });
    group.bench_function(BenchmarkId::new("VecDeque::push_back", LARGE_N), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let mut d = VecDeque::with_capacity(LARGE_N);
                for i in 0..LARGE_N {
                    d.push_back(black_box(i));
                }
                black_box(&mut d);
                elapsed += start.elapsed();
            }
            elapsed
        });
    });
    group.bench_function(BenchmarkId::new("LinkedList::push_back", LARGE_N), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let mut l = LinkedList::new();
                for i in 0..LARGE_N {
                    l.push_back(black_box(i));
                }
                black_box(&mut l);
                elapsed += start.elapsed();
            }
            elapsed
        });
    });
    group.bench_function(
        BenchmarkId::new("Hive::insert (no reserve)", LARGE_N),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    let mut h = Hive::new();
                    for i in 0..LARGE_N {
                        h.insert(black_box(i));
                    }
                    black_box(&mut h);
                    elapsed += start.elapsed();
                }
                elapsed
            });
        },
    );
    group.bench_function(BenchmarkId::new("Vec::push (no reserve)", LARGE_N), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let mut v = Vec::new();
                for i in 0..LARGE_N {
                    v.push(black_box(i));
                }
                black_box(&mut v);
                elapsed += start.elapsed();
            }
            elapsed
        });
    });
    group.bench_function(
        BenchmarkId::new("VecDeque::push_back (no reserve)", LARGE_N),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    let mut d = VecDeque::new();
                    for i in 0..LARGE_N {
                        d.push_back(black_box(i));
                    }
                    black_box(&mut d);
                    elapsed += start.elapsed();
                }
                elapsed
            });
        },
    );
    group.finish();
}

fn bench_hive_insertion_paths(c: &mut Criterion) {
    let mut group = c.benchmark_group("hive_insertion_paths_non_trivial");
    group.bench_function(BenchmarkId::new("insert", INSERT_PATH_N), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let mut h = Hive::with_capacity(INSERT_PATH_N);
                for i in 0..INSERT_PATH_N {
                    h.insert(black_box(NonTrivialElement::new(i)));
                }
                black_box(&mut h);
                elapsed += start.elapsed();
            }
            elapsed
        });
    });
    group.bench_function(BenchmarkId::new("emplace", INSERT_PATH_N), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let mut h = Hive::with_capacity(INSERT_PATH_N);
                for i in 0..INSERT_PATH_N {
                    h.emplace(|| black_box(NonTrivialElement::new(i)));
                }
                black_box(&mut h);
                elapsed += start.elapsed();
            }
            elapsed
        });
    });
    group.bench_function(BenchmarkId::new("insert_with", INSERT_PATH_N), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let mut h = Hive::with_capacity(INSERT_PATH_N);
                for i in 0..INSERT_PATH_N {
                    h.insert_with(|value: &mut NonTrivialElement| {
                        value.reset(black_box(i));
                    });
                }
                black_box(&mut h);
                elapsed += start.elapsed();
            }
            elapsed
        });
    });
    group.bench_function(
        BenchmarkId::new("insert_with_reuse_erased", INSERT_PATH_N),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    let mut h = Hive::with_capacity(INSERT_PATH_N);
                    let ptrs: Vec<*const NonTrivialElement> = (0..INSERT_PATH_N)
                        .map(|i| h.insert(NonTrivialElement::new(i)))
                        .collect();

                    for i in (0..INSERT_PATH_N).step_by(2) {
                        unsafe {
                            h.erase(ptrs[i]);
                        }
                    }

                    for i in (0..INSERT_PATH_N).step_by(2) {
                        h.insert_with(|value: &mut NonTrivialElement| {
                            value.reset(black_box(i + INSERT_PATH_N));
                        });
                    }
                    black_box(&mut h);
                    black_box(&ptrs);
                    elapsed += start.elapsed();
                }
                elapsed
            });
        },
    );
    group.finish();
}

fn bench_iteration(c: &mut Criterion) {
    let mut hive = Hive::with_capacity(LARGE_N);
    for i in 0..LARGE_N as u64 {
        hive.insert(i);
    }
    let vec: Vec<u64> = (0..LARGE_N as u64).collect();
    let deque: VecDeque<u64> = (0..LARGE_N as u64).collect();
    let list: LinkedList<u64> = (0..LARGE_N as u64).collect();

    let mut group = c.benchmark_group("iteration_sum");
    group.bench_function(BenchmarkId::new("Hive::iter", LARGE_N), |b| {
        b.iter(|| black_box(hive.iter().copied().sum::<u64>()));
    });
    group.bench_function(BenchmarkId::new("Vec::iter", LARGE_N), |b| {
        b.iter(|| black_box(vec.iter().copied().sum::<u64>()));
    });
    group.bench_function(BenchmarkId::new("VecDeque::iter", LARGE_N), |b| {
        b.iter(|| black_box(deque.iter().copied().sum::<u64>()));
    });
    group.bench_function(BenchmarkId::new("LinkedList::iter", LARGE_N), |b| {
        b.iter(|| black_box(list.iter().copied().sum::<u64>()));
    });
    group.finish();
}

fn bench_erase_reinsert(c: &mut Criterion) {
    let mut group = c.benchmark_group("erase_reinsert_every_10th");
    group.bench_function(BenchmarkId::new("Hive erase+insert", LARGE_N), |b| {
        b.iter(|| {
            let mut h = Hive::with_capacity(LARGE_N);
            let ptrs: Vec<*const u64> = (0..LARGE_N as u64).map(|i| h.insert(i)).collect();

            for i in (0..LARGE_N).step_by(10) {
                unsafe {
                    h.erase(ptrs[i]);
                }
            }
            for i in (0..LARGE_N).step_by(10) {
                h.insert(black_box(i as u64 + LARGE_N as u64));
            }
            black_box(h);
        });
    });
    group.bench_function(BenchmarkId::new("Vec remove+push", LARGE_N), |b| {
        b.iter(|| {
            let mut v: Vec<u64> = (0..LARGE_N as u64).collect();
            for i in (0..LARGE_N).step_by(10).rev() {
                v.remove(i);
            }
            for i in (0..LARGE_N).step_by(10) {
                v.push(black_box(i as u64 + LARGE_N as u64));
            }
            black_box(v);
        });
    });
    group.bench_function(BenchmarkId::new("LinkedList filter+append", LARGE_N), |b| {
        b.iter(|| {
            let mut l: LinkedList<u64> = (0..LARGE_N as u64)
                .enumerate()
                .filter(|(i, _)| i % 10 != 0)
                .map(|(_, v)| v)
                .collect();
            for i in (0..LARGE_N).step_by(10) {
                l.push_back(black_box(i as u64 + LARGE_N as u64));
            }
            black_box(l);
        });
    });
    group.finish();
}

fn bench_mixed_stable_reference(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_stable_reference");
    group.bench_function(BenchmarkId::new("Hive", MIXED_N), |b| {
        b.iter(|| {
            let mut h = Hive::with_capacity(MIXED_N);
            let ptrs: Vec<*const usize> = (0..MIXED_N).map(|i| h.insert(i)).collect();

            for (i, &p) in ptrs.iter().enumerate().take(MIXED_N / 2) {
                unsafe {
                    h.erase(p);
                }
                h.insert(black_box(i + 10_000));
            }
            black_box(h);
        });
    });
    group.bench_function(BenchmarkId::new("Vec", MIXED_N), |b| {
        b.iter(|| {
            let mut v: Vec<usize> = (0..MIXED_N).collect();
            for i in (0..MIXED_N / 2).rev() {
                v.remove(i);
                v.push(black_box(i + 10_000));
            }
            black_box(v);
        });
    });
    group.finish();
}

fn bench_pointer_access(c: &mut Criterion) {
    let mut hive = Hive::with_capacity(LARGE_N);
    let ptrs: Vec<*const u64> = (0..LARGE_N as u64).map(|i| hive.insert(i)).collect();
    let vec: Vec<u64> = (0..LARGE_N as u64).collect();

    let mut group = c.benchmark_group("stable_pointer_access");
    group.bench_function(BenchmarkId::new("Hive raw pointers", LARGE_N), |b| {
        b.iter(|| {
            let mut sum = 0u64;
            for &p in &ptrs {
                sum = sum.wrapping_add(unsafe { *p });
            }
            black_box(sum);
        });
    });
    group.bench_function(BenchmarkId::new("Vec references by index", LARGE_N), |b| {
        b.iter(|| {
            let mut sum = 0u64;
            for value in &vec {
                sum = sum.wrapping_add(*value);
            }
            black_box(sum);
        });
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = bench_append,
        bench_hive_insertion_paths,
        bench_iteration,
        bench_erase_reinsert,
        bench_mixed_stable_reference,
        bench_pointer_access
}
criterion_main!(benches);
