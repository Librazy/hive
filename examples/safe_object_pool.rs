//! Safe object-pool usage with `Pool`.
//!
//! Run with the default global allocator:
//! `cargo run --example safe_object_pool`
//!
//! Run with the example custom allocator:
//! `cargo run --example safe_object_pool --features safe_object_pool_custom_allocator`

#![cfg_attr(feature = "allocator_api", feature(allocator_api))]

use hive::Pool;

#[cfg(feature = "safe_object_pool_custom_allocator")]
use hive::allocator::{AllocError, Allocator};
#[cfg(feature = "safe_object_pool_custom_allocator")]
use std::alloc::{alloc, dealloc};
#[cfg(feature = "safe_object_pool_custom_allocator")]
use std::cell::Cell;
#[cfg(feature = "safe_object_pool_custom_allocator")]
use std::rc::Rc;

#[derive(Debug)]
struct Connection {
    id: u32,
    requests: u32,
    closed: bool,
}

impl Connection {
    fn new(id: u32) -> Self {
        Self {
            id,
            requests: 0,
            closed: false,
        }
    }

    fn handle_request(&mut self) {
        self.requests += 1;
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

#[cfg(feature = "safe_object_pool_custom_allocator")]
#[derive(Clone, Default)]
struct AllocStats {
    allocations: Rc<Cell<usize>>,
    deallocations: Rc<Cell<usize>>,
}

#[cfg(feature = "safe_object_pool_custom_allocator")]
impl AllocStats {
    fn allocations(&self) -> usize {
        self.allocations.get()
    }

    fn deallocations(&self) -> usize {
        self.deallocations.get()
    }
}

#[cfg(feature = "safe_object_pool_custom_allocator")]
#[derive(Clone)]
struct CountingAllocator {
    stats: AllocStats,
}

#[cfg(feature = "safe_object_pool_custom_allocator")]
unsafe impl Allocator for CountingAllocator {
    fn allocate(&self, layout: std::alloc::Layout) -> Result<std::ptr::NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            let ptr = std::ptr::NonNull::new(layout.align() as *mut u8).ok_or(AllocError)?;
            return Ok(std::ptr::NonNull::slice_from_raw_parts(ptr, 0));
        }

        let ptr = unsafe { alloc(layout) };
        let ptr = std::ptr::NonNull::new(ptr).ok_or(AllocError)?;
        self.stats.allocations.set(self.stats.allocations() + 1);
        Ok(std::ptr::NonNull::slice_from_raw_parts(ptr, layout.size()))
    }

    unsafe fn deallocate(&self, ptr: std::ptr::NonNull<u8>, layout: std::alloc::Layout) {
        if layout.size() != 0 {
            self.stats.deallocations.set(self.stats.deallocations() + 1);
            unsafe { dealloc(ptr.as_ptr(), layout) };
        }
    }
}

#[cfg(feature = "safe_object_pool_custom_allocator")]
fn make_pool(capacity: usize) -> (Pool<Connection, CountingAllocator>, AllocStats) {
    let stats = AllocStats::default();
    let allocator = CountingAllocator {
        stats: stats.clone(),
    };

    (Pool::with_capacity_in(capacity, allocator), stats)
}

#[cfg(not(feature = "safe_object_pool_custom_allocator"))]
fn make_pool(capacity: usize) -> Pool<Connection> {
    Pool::with_capacity(capacity)
}

fn main() {
    println!("=== Safe Pool Demo ===\n");

    #[cfg(feature = "safe_object_pool_custom_allocator")]
    let (pool, stats) = make_pool(4);

    #[cfg(not(feature = "safe_object_pool_custom_allocator"))]
    let pool = make_pool(4);

    let mut frontend = pool.insert(Connection::new(1));
    let mut worker = pool.insert(Connection::new(2));

    frontend.handle_request();
    frontend.handle_request();
    worker.handle_request();

    println!("frontend: {:?}", *frontend);
    println!("worker:   {:?}", *worker);
    println!("live connections: {}", pool.len());

    drop(frontend);
    println!("after frontend guard drops: {}", pool.len());

    let mut replacement = pool.insert(Connection::new(3));
    replacement.handle_request();
    replacement.close();

    println!("replacement: {:?}", *replacement);
    println!("worker id still available through guard: {}", worker.id);
    println!("pool capacity: {}", pool.capacity());

    drop(worker);
    drop(replacement);
    println!("after all guards drop: {}", pool.len());

    #[cfg(feature = "safe_object_pool_custom_allocator")]
    {
        drop(pool);
        println!(
            "custom allocator calls: {} alloc, {} dealloc",
            stats.allocations(),
            stats.deallocations()
        );
    }
}
