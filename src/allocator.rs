//! Allocator abstraction layer.
//!
//! Hive needs to allocate memory for groups (blocks) and element storage. This
//! module abstracts over two allocation strategies:
//!
//! 1. **With `allocator_api`** (nightly): re-exports [`core::alloc::Allocator`]
//!    and [`alloc::alloc::Global`] directly, enabling custom allocators.
//!
//! 2. **Without `allocator_api`** (stable-compatible polyfill): provides a
//!    minimal, unsafe `Allocator` trait and a `Global` struct that delegates to
//!    the system allocator. This lets the rest of the crate compile without
//!    nightly feature gates, albeit with global-allocator-only support.
//!
//! In both configurations, the items [`Allocator`], [`Global`], and
//! [`AllocError`] are always available at this module's root.

#[cfg(feature = "allocator_api")]
pub use alloc::alloc::Global;
#[cfg(feature = "allocator_api")]
pub use core::alloc::{AllocError, Allocator};

#[cfg(not(feature = "allocator_api"))]
mod polyfill {
    use core::alloc::Layout;
    use core::ptr::NonNull;

    /// Minimal trait mirroring the nightly `core::alloc::Allocator`.
    ///
    /// Required methods are `allocate` (returns `NonNull<[u8]>` or
    /// [`AllocError`]) and `deallocate`.
    ///
    /// # Safety
    ///
    /// Implementors must uphold the same safety contracts as
    /// `core::alloc::Allocator`. In particular:
    /// - `allocate` must return a valid, non-null memory block for the given
    ///   `Layout`, or an error.
    /// - `deallocate` must only be called with a pointer and layout returned
    ///   by a prior `allocate` from the same allocator.
    pub unsafe trait Allocator {
        /// Attempts to allocate a block of memory described by `layout`.
        fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError>;

        /// Deallocates a block previously allocated by this allocator.
        ///
        /// # Safety
        ///
        /// `ptr` must have been allocated by this allocator with the given
        /// `layout`, and must not be used after this call.
        unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout);
    }

    /// The error type for failed allocations.
    /// Mirrors `core::alloc::AllocError`, which is not yet stabilized.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AllocError;

    impl core::fmt::Display for AllocError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("memory allocation failed")
        }
    }

    /// The global memory allocator, mirroring `alloc::alloc::Global`.
    ///
    /// Delegates to [`alloc::alloc::alloc`] and [`alloc::alloc::dealloc`].
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Global;

    unsafe impl Allocator for Global {
        fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
            if layout.size() == 0 {
                // SAFETY: `layout.align()` is guaranteed to be non-zero.
                let ptr = unsafe {
                    NonNull::new_unchecked(core::ptr::without_provenance_mut(layout.align()))
                };
                return Ok(NonNull::slice_from_raw_parts(ptr, 0));
            }
            // SAFETY: layout has non-zero size.
            let raw = unsafe { alloc::alloc::alloc(layout) };
            let ptr = NonNull::new(raw).ok_or(AllocError)?;
            Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
        }

        unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
            if layout.size() != 0 {
                // SAFETY: caller guarantees ptr was allocated with this layout.
                unsafe { alloc::alloc::dealloc(ptr.as_ptr(), layout) };
            }
        }
    }
}

#[cfg(not(feature = "allocator_api"))]
#[allow(unused_imports)]
pub use polyfill::{AllocError, Allocator, Global};
