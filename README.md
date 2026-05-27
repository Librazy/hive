# hive

[![License](https://img.shields.io/badge/license-Zlib-blue.svg)](LICENSE.md)

A Rust port of [`plf::hive`](https://github.com/mattreecebentley/plf_hive), the bucket-based unordered container proposed as `std::hive` in [P0447](https://wg21.link/p0447).

`Hive<T>` stores elements across multiple memory blocks and tracks erased slots via a skipfield. Insertions reuse erased slots, so element addresses remain stable across insertions and across erasure of other elements. Operations that explicitly compact or reshape blocks may invalidate pointers. This makes `Hive` well-suited for object pools, entity-component systems, particle systems, and workloads that need stable references with frequent insertion and erasure.

Builds on stable Rust by default. Nightly Rust is only required when enabling
the `allocator_api` feature for custom allocator support.

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

- **Stable addresses** — pointers returned by `insert` and `insert_mut` remain valid until the element is erased or a compaction operation such as `reshape`/`shrink_to_fit` moves elements.
- **O(1) amortized insertion and erasure** — erased slots are immediately reused.
- **Bidirectional iteration** — `iter`, `iter_mut`, and `IntoIterator`.
- **In-place construction** — `insert_with_uninit` and `insert_with_uninit_mut` using `MaybeUninit`.
- **Erased-slot reuse** — pointers to erased elements become pointers to newly inserted elements.
- **Bulk operations** — `insert_many`, `assign`, `assign_from_iter`, `Extend`, `FromIterator`.
- **Retain and deduplicate** — `retain`, `sort`, `sort_by`, `unique`, `unique_by`.
- **Group splice** — `splice` transfers compatible source blocks without moving elements.
- **Capacity management** — `reserve`, `trim_capacity`, `trim_capacity_to`, `shrink_to_fit`.
- **Block limits** — configure minimum and maximum block sizes via `BlockCapacityLimits`; reshape at runtime.
- **Unsafe pointer helpers** — `get`, `get_mut`, `iter_from`, `iter_mut_from` for navigating from a raw pointer.
- **`no_std` compatible** — with `default-features = false`.

## Crate Features

| Feature | Default | Description |
|---|---|---|
| `std` | Yes | Enables `std` support (disabling gives `no_std` + `alloc`) |
| `allocator_api` | No | Unlocks custom allocator support (nightly-only) |
| `pin-init` | No | Enables safe in-place pinned initialization via `pin-init` |

Default builds use the crate's stable-compatible global allocator shim. Enable
`allocator_api` only when you need Rust's nightly `Allocator` trait and custom
allocator constructors:

```sh
cargo +nightly test --features allocator_api
```

## Differences from the C++ API

- `reserve(additional)` follows Rust collection semantics (additional capacity), not C++ total-capacity semantics.
- `splice` returns `Result<(), IncompatibleSplice>` instead of throwing when source block capacities are incompatible with the destination limits.
- No public C++-style iterator handle with ordering comparisons.
- Range erasure by iterator pair, `merge`, and optimized `advance`/`distance` equivalents are not currently exposed.

## Benchmarks

```sh
cargo bench --bench comparison
```

Benchmarks compare `Hive` against `Vec`, `VecDeque`, and `LinkedList`.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Use nightly for feature combinations that include `allocator_api`:

```sh
cargo +nightly clippy --all-targets --all-features -- -D warnings
cargo +nightly test --all-features
```

## License

Zlib. See [LICENSE.md](LICENSE.md).
