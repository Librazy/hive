//! Allocator abstraction layer.
//!
//! When the `allocator_api` feature is enabled, this module re-exports the
//! nightly `core::alloc::Allocator` trait and `alloc::alloc::Global`.
//!
//! When the feature is *disabled*, it provides a minimal polyfill so the rest
//! of the crate compiles on stable Rust (using the global allocator only).

#[cfg(feature = "allocator_api")]
pub use alloc::alloc::Global;
#[cfg(feature = "allocator_api")]
pub use core::alloc::Allocator;

#[cfg(not(feature = "allocator_api"))]
mod polyfill {
    use core::alloc::Layout;
    use core::ptr::NonNull;

    /// Minimal polyfill for the nightly `Allocator` trait.
    ///
    /// # Safety
    ///
    /// Implementors must uphold the same safety contracts as the nightly
    /// `core::alloc::Allocator` trait.
    pub unsafe trait Allocator {
        fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError>;

        /// # Safety
        ///
        /// `ptr` must have been allocated by this allocator with the given `layout`.
        unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout);
    }

    /// Polyfill for `core::alloc::AllocError` (not stabilised).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AllocError;

    impl core::fmt::Display for AllocError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("memory allocation failed")
        }
    }

    /// Polyfill for `alloc::alloc::Global`.
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
