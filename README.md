# hive

A Rust port of `plf::hive`, the bucket-based unordered container proposed for C++ as `std::hive` in [P0447](https://wg21.link/p0447).

`Hive<T>` stores elements in multiple blocks and tracks erased slots with a skipfield. Insertions reuse erased slots, so element addresses remain stable across insertions and across erasure of other elements. This makes it useful for object pools, entity lists, particle systems, and other workloads that need stable references plus frequent insertion and erasure.

## Status

This crate is experimental and currently requires nightly Rust because it uses `allocator_api`.

Implemented highlights:
- Stable raw pointers from `insert` and `insert_mut`.
- Safe reference insertion helpers: `insert_ref` and `insert_ref_mut`.
- Unsafe in-place construction helpers using `MaybeUninit`.
- Bidirectional iteration with `iter`, `iter_mut`, and `IntoIterator`.
- Erased-slot reuse with O(1)-style insertion/erasure behavior.
- `retain`, `clear`, `sort`, `sort_by`, `unique`, `unique_by`.
- `reserve`, `trim_capacity`, `trim_capacity_to`, `shrink_to_fit`.
- Block capacity limits and `reshape`.
- Unsafe pointer lookup helpers: `get`, `get_mut`, `iter_from`, `iter_mut_from`.
- Bulk helpers: `insert_many`, `assign`, `assign_from_iter`, `Extend`, `FromIterator`.

Not a perfect C++ API mirror:
- `reserve(additional)` follows Rust collection semantics, not C++ total-capacity semantics.
- `splice` moves source elements into the destination instead of linking blocks in O(1).
- There is no public C++-style iterator handle with ordering comparisons.
- Range erase by iterator pair and optimized `advance`/`distance` equivalents are not currently exposed.

## Example

```rust
use hive::Hive;

let mut hive = Hive::new();

let a = hive.insert(10);
let b = hive.insert(20);
let c = hive.insert(30);

for i in 0..1_000 {
    hive.insert(i);
}

unsafe {
    assert_eq!(*a, 10);
    assert_eq!(*b, 20);
    assert_eq!(*c, 30);
}

unsafe {
    hive.erase(&*b);
}

let reused = hive.insert(40);
assert_eq!(reused, b);
```

## In-Place Construction

For emplace-like use cases, `Hive` exposes unsafe `MaybeUninit` APIs:

```rust
use hive::Hive;

let hive = Hive::new();

let ptr = unsafe {
    hive.insert_with_uninit(|slot| {
        slot.write(String::from("constructed in place"));
    })
};

unsafe {
    assert_eq!(&*ptr, "constructed in place");
}
```

The closure must initialize the slot exactly once, must not read before initialization, and must not unwind after initialization.

## Capacity Limits

```rust
use hive::{BlockCapacityLimits, Hive};

let mut hive = Hive::<i32>::try_new(BlockCapacityLimits::new(8, 256)).unwrap();
assert_eq!(hive.block_capacity_limits(), BlockCapacityLimits::new(8, 256));

hive.reshape(BlockCapacityLimits::new(16, 512)).unwrap();
```

## Benchmarks

Benchmarks use Criterion:

```sh
cargo bench --bench comparison
```

Criterion writes reports and raw data under `target/criterion/`.

Recent local results on this workspace:

| Workload | Hive | Vec | VecDeque | LinkedList |
|---|---:|---:|---:|---:|
| Append 100k | 1.30 ms | 92 us | 156 us | 2.61 ms |
| Iterate/sum 100k | 1.42 ms | 17 us | 16 us | 133 us |
| Erase/reinsert every 10th, 100k | 1.45 ms | 70.1 ms | n/a | 2.62 ms |
| Mixed stable-reference, 2k | 40.3 us | 96.6 us | n/a | n/a |
| Pointer/index access 100k | 41.4 us | 17.1 us | n/a | n/a |

The pattern is expected: `Vec`/`VecDeque` dominate dense append and iteration, while `Hive` is stronger when stable addresses and frequent erase/reinsert cycles matter.

## Development

```sh
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## License

Zlib, matching the bundled reference implementation license.
