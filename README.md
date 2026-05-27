# hive

[![License](https://img.shields.io/badge/license-Zlib-blue.svg)](LICENSE.md)

A Rust port of [`plf::hive`](https://github.com/mattreecebentley/plf_hive), the bucket-based unordered container proposed as `std::hive` in [P0447](https://wg21.link/p0447).

`Hive<T>` stores elements across multiple memory blocks and tracks erased slots via a skipfield. Insertions reuse erased slots, so element addresses remain stable across insertions and across erasure of other elements — making it well-suited for object pools, entity-component systems, particle systems, and workloads that need stable references with frequent insertion and erasure.

**Requires nightly Rust** (`allocator_api`).

## Getting Started

```toml
[dependencies]
hive = "0.1"
```

```rust
use hive::Hive;

let mut hive = Hive::new();

let a = hive.insert(10);
let b = hive.insert(20);
let c = hive.insert(30);

for i in 0..1_000 {
    hive.insert(i);
}

// stable raw pointers
unsafe {
    assert_eq!(*a, 10);
    assert_eq!(*b, 20);
    assert_eq!(*c, 30);
}

// erasure by raw pointer reuses the slot
unsafe { hive.erase(b); }
let reused = hive.insert(40);
assert_eq!(reused, b);
```

See the [examples](./examples/) directory for more complete usage, including an object pool and a safe wrapper API.

## Features

- **Stable addresses** — pointers returned by `insert` and `insert_mut` remain valid until the element is erased.
- **O(1) amortized insertion and erasure** — erased slots are immediately reused.
- **Bidirectional iteration** — `iter`, `iter_mut`, and `IntoIterator`.
- **In-place construction** — `insert_with_uninit` and `insert_with_uninit_raw` using `MaybeUninit`.
- **Erased-slot reuse** — pointers to erased elements become pointers to newly inserted elements.
- **Bulk operations** — `insert_many`, `assign`, `assign_from_iter`, `Extend`, `FromIterator`.
- **Retain and deduplicate** — `retain`, `sort`, `sort_by`, `unique`, `unique_by`.
- **Capacity management** — `reserve`, `trim_capacity`, `trim_capacity_to`, `shrink_to_fit`.
- **Block limits** — configure minimum and maximum block sizes via `BlockCapacityLimits`; reshape at runtime.
- **Unsafe pointer helpers** — `get`, `get_mut`, `iter_from`, `iter_mut_from` for navigating from a raw pointer.
- **`no_std` compatible** — with `default-features = false`.

## Crate Features

| Feature | Default | Description |
|---|---|---|
| `std` | Yes | Enables `std` support (disabling gives `no_std` + `alloc`) |
| `allocator_api` | No | Unlocks custom allocator support (nightly-only) |

## Differences from the C++ API

- `reserve(additional)` follows Rust collection semantics (additional capacity), not C++ total-capacity semantics.
- `splice` moves source elements into the destination rather than O(1) block relinking.
- No public C++-style iterator handle with ordering comparisons.
- Range erasure by iterator pair and optimized `advance`/`distance` equivalents are not currently exposed.

## Benchmarks

```sh
cargo bench --bench comparison
```

Benchmarks compare `Hive` against `Vec`, `VecDeque`, and `LinkedList`.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## License

Zlib. See [LICENSE.md](LICENSE.md).
