#![no_std]
#![feature(allocator_api)]

#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

mod free_list;
mod group;
mod skipfield;

pub mod hive;
mod iter;

pub use hive::{BlockCapacityLimits, Hive, InvalidBlockCapacityLimits};
