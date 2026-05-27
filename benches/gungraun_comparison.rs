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
        hive_mixed_stable_reference
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
        hive_mixed_stable_reference
    ]
);

#[cfg(target_os = "linux")]
gungraun::main!(library_benchmark_groups = hive_comparison);

#[cfg(not(target_os = "linux"))]
fn main() {}
