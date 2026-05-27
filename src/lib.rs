//! A Rust port of `plf::hive`, the bucket-based unordered container proposed
//! as `std::hive` in [P0447](https://wg21.link/p0447).
//!
//! [`Hive<T>`] stores elements across multiple memory blocks and tracks erased
//! slots via a skipfield. Insertions reuse erased slots, so element addresses
//! remain stable across insertions and across erasure of other elements. This
//! makes it well-suited for object pools, entity-component systems, particle
//! systems, and workloads that need stable references with frequent insertion
//! and erasure.
//!
//! # Quick start
//!
//! ```
//! use hive::Hive;
//!
//! let mut hive = Hive::new();
//! let a = hive.insert(10);
//! let b = hive.insert(20);
//!
//! unsafe { hive.erase(b); }
//! let reused = hive.insert(30);
//! assert_eq!(reused, b); // erased slot is reused
//! ```
//!
//! # Pointer stability
//!
//! Raw pointers returned by [`insert`](Hive::insert) and
//! [`insert_mut`](Hive::insert_mut) remain valid until the corresponding element
//! is erased. Inserting new elements never moves existing ones.
//!
//! # Safety around erasure
//!
//! [`erase`](Hive::erase) takes a `*const T` rather than `&T` or `&mut T` to
//! avoid Stacked/Tree Borrows protector violations — the function overwrites
//! erased-slot memory for the free-list. The caller must ensure no Rust
//! references (`&T` or `&mut T`) to the same element are live.
//!
//! # Crate features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | Yes | Enables `std` support (disabling gives `no_std` + `alloc`) |
//! | `allocator_api` | No | Unlocks custom allocator support via nightly `Allocator` trait |
//!
//! # Nightly requirement
//!
//! This crate requires a nightly Rust toolchain because it uses the
//! `allocator_api` unstable feature.
//!
//! # Object pools
//!
//! The [`Pool<T>`] and [`SyncPool<T>`] types offer safe, restricted wrappers
//! around `Hive` that hand out guard types ([`Pooled`], [`SyncPooled`]) instead
//! of raw pointers. These are appropriate when the caller does not need direct
//! index-based access or fine-grained iteration.

#![no_std]
#![cfg_attr(feature = "allocator_api", feature(allocator_api))]

#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

pub mod allocator;
mod free_list;
mod group;
mod skipfield;

pub mod hive;
mod iter;
pub mod pool;

pub use hive::{BlockCapacityLimits, Hive, InvalidBlockCapacityLimits};
pub use pool::{Pool, Pooled};
#[cfg(feature = "std")]
pub use pool::{SyncPool, SyncPooled};
