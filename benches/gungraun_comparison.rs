#[cfg(target_os = "linux")]
use gungraun::prelude::*;
#[cfg(target_os = "linux")]
use hive::Hive;
#[cfg(all(target_os = "linux", feature = "pin-init"))]
use pin_init::{init_from_closure, InitResult, PinUninit};
#[cfg(all(target_os = "linux", feature = "pin-init"))]
use std::convert::Infallible;
#[cfg(target_os = "linux")]
use std::hint::black_box;
#[cfg(all(target_os = "linux", feature = "pin-init"))]
use std::ptr;

#[cfg(target_os = "linux")]
const MIXED_N: usize = 65536;
#[cfg(target_os = "linux")]
const INSERT_PATH_N: usize = 131072;
#[cfg(target_os = "linux")]
const LARGE_N: usize = 1048576;

#[cfg(target_os = "linux")]
#[derive(Clone, Default)]
struct NonTrivialElement {
    name: String,
    data: Vec<u64>,
    checksum: u64,
}

#[cfg(target_os = "linux")]
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

#[cfg(all(target_os = "linux", feature = "pin-init"))]
fn non_trivial_pin_init(i: usize) -> impl pin_init::Init<NonTrivialElement, Infallible> {
    init_from_closure(
        move |mut this: PinUninit<'_, NonTrivialElement>| -> InitResult<'_, NonTrivialElement, Infallible> {
            let ptr = this.get_mut().as_mut_ptr();
            let data = vec![i as u64, i as u64 + 1, i as u64 + 2, i as u64 + 3];
            let checksum = data.iter().copied().fold(i as u64, u64::wrapping_add);
            unsafe {
                ptr::addr_of_mut!((*ptr).name).write(format!("element-{i}"));
                ptr::addr_of_mut!((*ptr).data).write(data);
                ptr::addr_of_mut!((*ptr).checksum).write(checksum);
                Ok(this.init_ok())
            }
        },
    )
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_insert() -> Hive<NonTrivialElement> {
    let mut h = Hive::with_capacity(INSERT_PATH_N);
    for i in 0..INSERT_PATH_N {
        h.insert(black_box(NonTrivialElement::new(i)));
    }
    black_box(h)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_emplace() -> Hive<NonTrivialElement> {
    let mut h = Hive::with_capacity(INSERT_PATH_N);
    for i in 0..INSERT_PATH_N {
        h.emplace(|| black_box(NonTrivialElement::new(i)));
    }
    black_box(h)
}

#[cfg(all(target_os = "linux", feature = "pin-init"))]
#[library_benchmark]
fn hive_insert_pin_init() -> Hive<NonTrivialElement> {
    let mut h = Hive::with_capacity(INSERT_PATH_N);
    for i in 0..INSERT_PATH_N {
        h.insert_pin_init::<_, Infallible>(black_box(NonTrivialElement::new(i)))
            .unwrap();
    }
    black_box(h)
}

#[cfg(all(target_os = "linux", feature = "pin-init"))]
#[library_benchmark]
fn hive_insert_pin_init_fields() -> Hive<NonTrivialElement> {
    let mut h = Hive::with_capacity(INSERT_PATH_N);
    for i in 0..INSERT_PATH_N {
        h.insert_pin_init(non_trivial_pin_init(black_box(i)))
            .unwrap();
    }
    black_box(h)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_insert_with() -> Hive<NonTrivialElement> {
    let mut h = Hive::with_capacity(INSERT_PATH_N);
    for i in 0..INSERT_PATH_N {
        h.insert_with(|value: &mut NonTrivialElement| {
            value.reset(black_box(i));
        });
    }
    black_box(h)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_insert_with_reuse_erased() -> Hive<NonTrivialElement> {
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

    black_box(&ptrs);
    black_box(h)
}

#[cfg(target_os = "linux")]
#[derive(Clone)]
#[repr(transparent)]
struct HugeElement {
    data: [u64; 1024],
}

impl HugeElement {
    fn new(i: usize) -> Self {
        let mut data = [0u64; 1024];
        for j in 0..1024 {
            data[j] = i as u64 + j as u64;
        }
        Self { data }
    }
}

#[cfg(all(target_os = "linux", feature = "pin-init"))]
fn huge_pin_init(i: usize) -> impl pin_init::Init<HugeElement, Infallible> {
    init_from_closure(
        move |mut this: PinUninit<'_, HugeElement>| -> InitResult<'_, HugeElement, Infallible> {
            let ptr = this.get_mut().as_mut_ptr() as *mut u64;

            unsafe {
                for j in 0..1024 {
                    ptr.add(j).write(i as u64 + j as u64);
                }
                Ok(this.init_ok())
            }
        },
    )
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_insert_huge() -> Hive<HugeElement> {
    let mut h = Hive::with_capacity(MIXED_N);
    for i in 0..MIXED_N {
        let element = HugeElement::new(i);
        h.insert(black_box(element));
    }
    black_box(h)
}

#[cfg(all(target_os = "linux", feature = "pin-init"))]
#[library_benchmark]
fn hive_insert_pin_init_huge() -> Hive<HugeElement> {
    let mut h = Hive::with_capacity(MIXED_N);
    for i in 0..MIXED_N {
        h.insert_pin_init::<_, Infallible>(huge_pin_init(black_box(i)))
            .unwrap();
    }
    black_box(h)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_erase_insert() -> Hive<u64> {
    let mut h = Hive::with_capacity(INSERT_PATH_N);
    let ptrs: Vec<*const u64> = (0..INSERT_PATH_N as u64).map(|i| h.insert(i)).collect();

    for i in (0..INSERT_PATH_N).step_by(10) {
        unsafe {
            h.erase(ptrs[i]);
        }
    }
    for i in (0..INSERT_PATH_N).step_by(10) {
        h.insert(black_box(i as u64 + INSERT_PATH_N as u64));
    }

    black_box(h)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_mixed_stable_reference() -> Hive<usize> {
    let mut h = Hive::with_capacity(MIXED_N);
    let ptrs: Vec<*const usize> = (0..MIXED_N).map(|i| h.insert(i)).collect();

    for (i, &p) in ptrs.iter().enumerate().take(MIXED_N / 2) {
        unsafe {
            h.erase(p);
        }
        h.insert(black_box(i + 10_000));
    }

    black_box(h)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_raw_pointer_access() -> u64 {
    let mut hive = Hive::with_capacity(LARGE_N);
    let ptrs: Vec<*const u64> = (0..LARGE_N as u64).map(|i| hive.insert(i)).collect();

    let mut sum = 0u64;
    for &p in &ptrs {
        sum = sum.wrapping_add(unsafe { *p });
    }

    black_box(&hive);
    black_box(sum)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_splice_into_empty() -> Hive<u64> {
    let mut dest = Hive::new();
    let mut source = Hive::with_capacity(INSERT_PATH_N);
    for i in 0..INSERT_PATH_N as u64 {
        source.insert(black_box(i));
    }

    dest.splice(&mut source).unwrap();

    black_box(&source);
    black_box(dest)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_splice_append() -> Hive<u64> {
    let mut dest = Hive::with_capacity(INSERT_PATH_N);
    let mut source = Hive::with_capacity(INSERT_PATH_N);
    for i in 0..INSERT_PATH_N as u64 {
        dest.insert(black_box(i));
        source.insert(black_box(i + INSERT_PATH_N as u64));
    }

    dest.splice(&mut source).unwrap();

    black_box(&source);
    black_box(dest)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
fn hive_splice_append_with_gaps() -> Hive<u64> {
    let mut dest = Hive::with_capacity(INSERT_PATH_N);
    let mut source = Hive::with_capacity(INSERT_PATH_N);
    let ptrs: Vec<*const u64> = (0..INSERT_PATH_N as u64)
        .map(|i| dest.insert(black_box(i)))
        .collect();

    for &ptr in ptrs.iter().step_by(8) {
        unsafe {
            dest.erase(ptr);
        }
    }
    for i in 0..INSERT_PATH_N as u64 {
        source.insert(black_box(i + INSERT_PATH_N as u64));
    }

    dest.splice(&mut source).unwrap();

    black_box(&ptrs);
    black_box(&source);
    black_box(dest)
}

#[cfg(all(target_os = "linux", feature = "pin-init"))]
library_benchmark_group!(
    name = hive_comparison,
    benchmarks = [
        hive_insert,
        hive_emplace,
        hive_insert_pin_init,
        hive_insert_pin_init_fields,
        hive_insert_with,
        hive_insert_with_reuse_erased,
        hive_erase_insert,
        hive_mixed_stable_reference,
        hive_raw_pointer_access,
        hive_splice_into_empty,
        hive_splice_append,
        hive_splice_append_with_gaps,
        hive_insert_huge,
        hive_insert_pin_init_huge,
    ]
);

#[cfg(all(target_os = "linux", not(feature = "pin-init")))]
library_benchmark_group!(
    name = hive_comparison,
    benchmarks = [
        hive_insert,
        hive_emplace,
        hive_insert_with,
        hive_insert_with_reuse_erased,
        hive_erase_insert,
        hive_mixed_stable_reference,
        hive_raw_pointer_access,
        hive_splice_into_empty,
        hive_splice_append,
        hive_splice_append_with_gaps,
        hive_insert_huge,
    ]
);

#[cfg(target_os = "linux")]
gungraun::main!(library_benchmark_groups = hive_comparison);

#[cfg(not(target_os = "linux"))]
fn main() {}
