//! Safe object-pool wrappers around [`Hive`].
//!
//! This module provides two object pool types:
//!
//! - [`Pool<T>`] — a single-threaded pool that hands out [`Pooled`] guards.
//! - [`SyncPool<T>`] — a thread-safe pool that hands out [`SyncPooled`] guards
//!   (requires the `std` feature).
//!
//! # Why pools?
//!
//! `Hive` exposes raw pointers and requires `unsafe` for erasure. The pool
//! wrappers provide a safe subset of that API: callers insert elements and
//! receive RAII guard types that automatically erase their element on drop.
//! Direct references (`&T`/`&mut T`) are handed out safely because the guards
//! enforce exclusive ownership and automatic cleanup.
//!
//! # Safety model
//!
//! Pools are safe because:
//! - Only insertion and metadata (`len`, `is_empty`, `capacity`) are exposed.
//! - Each guard uniquely owns its element. Guards are not cloneable.
//! - Guard drops erase the element, and drop is guaranteed to run exactly once.
//!
//! The backing `Hive` is always accessed under the pool's internal invariants,
//! so no raw pointer management is exposed to the caller.

use crate::allocator::{Allocator, Global};
use crate::{BlockCapacityLimits, Hive, InvalidBlockCapacityLimits};
use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

#[cfg(feature = "std")]
use std::sync::{Arc, Mutex};

/// A safe single-threaded object pool backed by [`Hive`].
///
/// `Pool` restricts the `Hive` API to insertion and metadata, hiding raw
/// pointer manipulation. Inserted objects are represented by [`Pooled`] guards,
/// which erase their element when dropped. The restricted API is what makes it
/// sound to hand out `&T`/`&mut T` references while the pool remains available.
///
/// `Pool` is not `Sync`; use [`SyncPool`] when sharing a pool between threads.
///
/// # Examples
///
/// ```
/// use hive::Pool;
///
/// let pool = Pool::new();
/// let mut guard = pool.insert(42u32);
/// assert_eq!(*guard, 42);
/// *guard = 99;
/// assert_eq!(*guard, 99);
/// drop(guard); // element erased from pool
/// assert!(pool.is_empty());
/// ```
pub struct Pool<T, A: Allocator + Clone = Global> {
    hive: UnsafeCell<Hive<T, A>>,
}

/// A live object stored in a [`Pool`].
///
/// Provides `Deref`/`DerefMut` access to the underlying `T`. When the guard is
/// dropped, the element is erased from the pool.
///
/// `Pooled` is not `Send` or `Sync` because it borrows the pool mutably.
///
/// # Panic safety
///
/// Dropping a guard erases its element from the pool. If `T::drop` panics while
/// the guard is being dropped, the backing hive may still mark that slot as
/// live. Do not rely on recovering and continuing to use the pool after an
/// element destructor panics during guard drop.
pub struct Pooled<'a, T, A: Allocator + Clone = Global> {
    pool: &'a Pool<T, A>,
    ptr: NonNull<T>,
    _not_send_sync: PhantomData<&'a mut T>,
}

impl<T> Pool<T, Global> {
    /// Creates an empty pool.
    ///
    /// Uses the global allocator and default block capacity limits.
    pub fn new() -> Self {
        Self::new_in(Global)
    }

    /// Creates an empty pool with space pre-allocated for at least `capacity`
    /// elements.
    ///
    /// Equivalent to constructing a new pool and calling
    /// [`reserve`](Hive::reserve) on the backing hive.
    pub fn with_capacity(capacity: usize) -> Self {
        let pool = Self::new();
        // SAFETY: no guards exist yet, so reserving through the backing hive
        // cannot invalidate exposed references.
        unsafe { (&mut *pool.hive.get()).reserve(capacity) };
        pool
    }

    /// Creates an empty pool with custom block capacity limits.
    ///
    /// Returns [`InvalidBlockCapacityLimits`] if limits are out of bounds.
    pub fn try_new(limits: BlockCapacityLimits) -> Result<Self, InvalidBlockCapacityLimits> {
        Self::try_new_in(Global, limits)
    }
}

impl<T, A: Allocator + Clone> Pool<T, A> {
    /// Creates an empty pool using `allocator`.
    pub fn new_in(allocator: A) -> Self {
        Self {
            hive: UnsafeCell::new(Hive::new_in(allocator)),
        }
    }

    /// Creates an empty pool using `allocator` with space for at least
    /// `capacity` elements.
    pub fn with_capacity_in(capacity: usize, allocator: A) -> Self {
        let pool = Self::new_in(allocator);
        // SAFETY: no guards exist yet, so reserving through the backing hive
        // cannot invalidate exposed references.
        unsafe { (&mut *pool.hive.get()).reserve(capacity) };
        pool
    }

    /// Creates an empty pool using `allocator` and custom block capacity
    /// limits.
    ///
    /// Returns [`InvalidBlockCapacityLimits`] if limits are out of bounds.
    pub fn try_new_in(
        allocator: A,
        limits: BlockCapacityLimits,
    ) -> Result<Self, InvalidBlockCapacityLimits> {
        Ok(Self {
            hive: UnsafeCell::new(Hive::try_new_in(allocator, limits)?),
        })
    }

    /// Inserts `value` into the pool and returns a [`Pooled`] guard that
    /// provides `Deref`/`DerefMut` access.
    ///
    /// The element is automatically erased from the pool when the guard is
    /// dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use hive::Pool;
    ///
    /// let pool = Pool::new();
    /// {
    ///     let guard = pool.insert(42);
    ///     assert_eq!(*guard, 42);
    ///     assert_eq!(pool.len(), 1);
    /// }
    /// assert!(pool.is_empty());
    /// ```
    pub fn insert(&self, value: T) -> Pooled<'_, T, A> {
        // SAFETY: `Pool` never exposes references into the whole `Hive`, only
        // per-element guards. `Hive` insertion does not move active elements,
        // and the returned guard owns the newly inserted slot uniquely.
        let ptr = unsafe { (&mut *self.hive.get()).insert_mut(value) };
        let ptr = NonNull::new(ptr).expect("Hive::insert_mut returned null");

        Pooled {
            pool: self,
            ptr,
            _not_send_sync: PhantomData,
        }
    }

    /// Constructs a value from a closure and inserts it.
    ///
    /// The closure runs before pool insertion, so a panic in the closure
    /// leaves the pool unchanged.
    pub fn emplace<F>(&self, f: F) -> Pooled<'_, T, A>
    where
        F: FnOnce() -> T,
    {
        self.insert(f())
    }

    /// Inserts `T::default()`, then calls `f` on the initialized element.
    ///
    /// If `f` panics, the default value remains in the pool and will be erased
    /// when the pool is dropped (or when the guard is dropped, if the guard was
    /// successfully obtained).
    pub fn insert_with<F>(&self, f: F) -> Pooled<'_, T, A>
    where
        T: Default,
        F: FnOnce(&mut T),
    {
        let mut value = self.insert(T::default());
        f(&mut value);
        value
    }

    /// Pin-initializes an element in-place in the pool and returns a
    /// [`Pooled`] guard.
    ///
    /// The initializer receives a pinned, uninitialized slot through
    /// [`pin_init::PinUninit`]. The element is considered inserted only if the
    /// initializer succeeds; on error, the pool remains unchanged apart from any
    /// allocation that may be retained for later reuse.
    ///
    /// Requires the `pin-init` feature.
    #[cfg(feature = "pin-init")]
    pub fn insert_pin_init<I, E>(&self, init: I) -> Result<Pooled<'_, T, A>, E>
    where
        I: pin_init::Init<T, E>,
    {
        let ptr = unsafe { (&mut *self.hive.get()).insert_pin_init_mut(init)? };
        let ptr = NonNull::new(ptr).expect("Hive::insert_pin_init_mut returned null");

        Ok(Pooled {
            pool: self,
            ptr,
            _not_send_sync: PhantomData,
        })
    }

    /// Returns the number of live objects in the pool.
    pub fn len(&self) -> usize {
        // SAFETY: reading metadata does not create references to elements.
        unsafe { (&*self.hive.get()).len() }
    }

    /// Returns `true` if the pool contains no live objects.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the total element capacity of the backing hive.
    ///
    /// This includes both live and reserved (empty) slots.
    pub fn capacity(&self) -> usize {
        // SAFETY: reading metadata does not create references to elements.
        unsafe { (&*self.hive.get()).capacity() }
    }
}

impl<T> Default for Pool<T, Global> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, A: Allocator + Clone> Deref for Pooled<'_, T, A> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: each `Pooled` guard is created from a fresh insertion and is
        // not cloneable, so it is the only safe owner of this element reference.
        unsafe { self.ptr.as_ref() }
    }
}

impl<T, A: Allocator + Clone> DerefMut for Pooled<'_, T, A> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: mutable access requires `&mut self`, and the guard uniquely
        // owns its element until drop.
        unsafe { self.ptr.as_mut() }
    }
}

impl<T, A: Allocator + Clone> Drop for Pooled<'_, T, A> {
    fn drop(&mut self) {
        // SAFETY: the pointer came from this pool's hive, and a `Pooled` guard
        // erases exactly once because it is not cloneable and owns its drop.
        unsafe {
            (&mut *self.pool.hive.get()).erase(self.ptr.as_ptr());
        }
    }
}

/// A thread-safe object pool backed by [`Hive`].
///
/// Unlike [`Pool`], `SyncPool` wraps the hive in `Arc<Mutex<...>>`, allowing
/// the pool handle to be cloned and shared across threads. Each [`SyncPooled`]
/// guard provides direct access to a single live element; pool metadata
/// mutations are serialized by the internal mutex.
///
/// `SyncPool` requires the `std` feature.
///
/// # Examples
///
/// ```
/// use hive::SyncPool;
///
/// let pool = SyncPool::new();
/// let mut guard = pool.insert(42u32);
/// assert_eq!(*guard, 42);
/// *guard = 100;
/// drop(guard);
/// ```
#[cfg(feature = "std")]
pub struct SyncPool<T, A: Allocator + Clone = Global> {
    hive: Arc<Mutex<Hive<T, A>>>,
}

/// A live object stored in a [`SyncPool`].
///
/// Provides `Deref`/`DerefMut` access. Dropping the guard erases the element
/// from the pool (acquiring the mutex to do so). The guard implements `Send`
/// when `T` and `A` are `Send`, and `Sync` when `T: Sync` and `A: Send`.
///
/// # Panic safety
///
/// Dropping a guard erases its element from the pool. If `T::drop` panics while
/// the guard is being dropped, the backing hive may still mark that slot as
/// live. Do not rely on recovering and continuing to use the pool after an
/// element destructor panics during guard drop.
#[cfg(feature = "std")]
pub struct SyncPooled<T, A: Allocator + Clone = Global> {
    hive: Arc<Mutex<Hive<T, A>>>,
    ptr: NonNull<T>,
    _owns_element: PhantomData<T>,
}

#[cfg(feature = "std")]
impl<T> SyncPool<T, Global> {
    /// Creates an empty synchronized pool.
    ///
    /// Uses the global allocator and default block capacity limits.
    pub fn new() -> Self {
        Self::new_in(Global)
    }

    /// Creates an empty synchronized pool with space pre-allocated for at
    /// least `capacity` elements.
    pub fn with_capacity(capacity: usize) -> Self {
        let pool = Self::new();
        pool.hive.lock().unwrap().reserve(capacity);
        pool
    }

    /// Creates an empty synchronized pool with custom block capacity limits.
    ///
    /// Returns [`InvalidBlockCapacityLimits`] if limits are out of bounds.
    pub fn try_new(limits: BlockCapacityLimits) -> Result<Self, InvalidBlockCapacityLimits> {
        Self::try_new_in(Global, limits)
    }
}

#[cfg(feature = "std")]
impl<T, A: Allocator + Clone> Clone for SyncPool<T, A> {
    fn clone(&self) -> Self {
        Self {
            hive: self.hive.clone(),
        }
    }
}

#[cfg(feature = "std")]
impl<T, A: Allocator + Clone> SyncPool<T, A> {
    /// Creates an empty synchronized pool using `allocator`.
    pub fn new_in(allocator: A) -> Self {
        Self {
            hive: Arc::new(Mutex::new(Hive::new_in(allocator))),
        }
    }

    /// Creates an empty synchronized pool using `allocator` with space for at
    /// least `capacity` elements.
    pub fn with_capacity_in(capacity: usize, allocator: A) -> Self {
        let pool = Self::new_in(allocator);
        pool.hive.lock().unwrap().reserve(capacity);
        pool
    }

    /// Creates an empty synchronized pool using `allocator` and custom block
    /// capacity limits.
    ///
    /// Returns [`InvalidBlockCapacityLimits`] if limits are out of bounds.
    pub fn try_new_in(
        allocator: A,
        limits: BlockCapacityLimits,
    ) -> Result<Self, InvalidBlockCapacityLimits> {
        Ok(Self {
            hive: Arc::new(Mutex::new(Hive::try_new_in(allocator, limits)?)),
        })
    }

    /// Inserts `value` and returns a [`SyncPooled`] guard.
    ///
    /// The guard can be moved independently of the pool handle and implements
    /// `Send`/`Sync` when the element and allocator types allow it.
    ///
    /// # Examples
    ///
    /// ```
    /// use hive::SyncPool;
    ///
    /// let pool = SyncPool::new();
    /// let guard = pool.insert("hello");
    /// assert_eq!(*guard, "hello");
    /// ```
    pub fn insert(&self, value: T) -> SyncPooled<T, A> {
        let ptr = self.hive.lock().unwrap().insert_mut(value);
        let ptr = NonNull::new(ptr).expect("Hive::insert_mut returned null");

        SyncPooled {
            hive: self.hive.clone(),
            ptr,
            _owns_element: PhantomData,
        }
    }

    /// Constructs a value from a closure and inserts it.
    ///
    /// The closure runs before the pool lock is acquired, so expensive
    /// construction does not block unrelated pool operations.
    pub fn emplace<F>(&self, f: F) -> SyncPooled<T, A>
    where
        F: FnOnce() -> T,
    {
        self.insert(f())
    }

    /// Inserts `T::default()`, then calls `f` on the initialized element.
    ///
    /// If `f` panics, the default value remains in the pool and will be erased
    /// when the pool is dropped (or when the guard is dropped).
    pub fn insert_with<F>(&self, f: F) -> SyncPooled<T, A>
    where
        T: Default,
        F: FnOnce(&mut T),
    {
        let mut value = self.insert(T::default());
        f(&mut value);
        value
    }

    /// Pin-initializes an element in-place in the pool and returns a
    /// [`SyncPooled`] guard.
    ///
    /// The initializer receives a pinned, uninitialized slot through
    /// [`pin_init::PinUninit`]. The element is considered inserted only if the
    /// initializer succeeds; on error, the pool remains unchanged apart from any
    /// allocation that may be retained for later reuse.
    ///
    /// Requires the `pin-init` feature.
    #[cfg(feature = "pin-init")]
    pub fn insert_pin_init<I, E>(&self, init: I) -> Result<SyncPooled<T, A>, E>
    where
        I: pin_init::Init<T, E>,
    {
        let ptr = self.hive.lock().unwrap().insert_pin_init_mut(init)?;
        let ptr = NonNull::new(ptr).expect("Hive::insert_pin_init_mut returned null");

        Ok(SyncPooled {
            hive: self.hive.clone(),
            ptr,
            _owns_element: PhantomData,
        })
    }

    /// Returns the number of live objects in the pool.
    pub fn len(&self) -> usize {
        self.hive.lock().unwrap().len()
    }

    /// Returns `true` if the pool contains no live objects.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the total element capacity of the backing hive.
    ///
    /// This includes both live and reserved (empty) slots.
    pub fn capacity(&self) -> usize {
        self.hive.lock().unwrap().capacity()
    }
}

#[cfg(feature = "std")]
impl<T> Default for SyncPool<T, Global> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "std")]
impl<T, A: Allocator + Clone> Deref for SyncPooled<T, A> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: `SyncPooled` is unique for its element and is not cloneable.
        // Pool operations may mutate metadata concurrently, but they do not move
        // active elements. Erasure happens only through this guard's `Drop`.
        unsafe { self.ptr.as_ref() }
    }
}

#[cfg(feature = "std")]
impl<T, A: Allocator + Clone> DerefMut for SyncPooled<T, A> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: mutable access requires `&mut self`, and no other safe handle
        // to this element can exist.
        unsafe { self.ptr.as_mut() }
    }
}

#[cfg(feature = "std")]
impl<T, A: Allocator + Clone> Drop for SyncPooled<T, A> {
    fn drop(&mut self) {
        unsafe {
            self.hive.lock().unwrap().erase(self.ptr.as_ptr());
        }
    }
}

#[cfg(feature = "std")]
unsafe impl<T: Send, A: Allocator + Clone + Send> Send for SyncPooled<T, A> {}

#[cfg(feature = "std")]
unsafe impl<T: Sync, A: Allocator + Clone + Send> Sync for SyncPooled<T, A> {}
