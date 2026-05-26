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
