//! A restricted safe object-pool wrapper around [`Hive`].

use crate::allocator::{Allocator, Global};
use crate::{BlockCapacityLimits, Hive, InvalidBlockCapacityLimits};
use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

#[cfg(feature = "std")]
use std::sync::{Arc, Mutex};

/// A safe object pool backed by [`Hive`].
///
/// `Pool` intentionally exposes only insertion and metadata APIs. Inserted
/// objects are represented by [`Pooled`] guards, which erase their element when
/// dropped. The restricted API is what makes it sound to hand out direct
/// references to elements while the pool remains available for more insertions.
pub struct Pool<T, A: Allocator + Clone = Global> {
    hive: UnsafeCell<Hive<T, A>>,
}

/// A live object stored in a [`Pool`].
///
/// Dropping this guard removes the object from its pool.
pub struct Pooled<'a, T, A: Allocator + Clone = Global> {
    pool: &'a Pool<T, A>,
    ptr: NonNull<T>,
    _not_send_sync: PhantomData<&'a mut T>,
}

impl<T> Pool<T, Global> {
    /// Creates an empty pool.
    pub fn new() -> Self {
        Self::new_in(Global)
    }

    /// Creates an empty pool with space for at least `capacity` elements.
    pub fn with_capacity(capacity: usize) -> Self {
        let pool = Self::new();
        // SAFETY: no guards exist yet, so reserving through the backing hive
        // cannot invalidate exposed references.
        unsafe { (&mut *pool.hive.get()).reserve(capacity) };
        pool
    }

    /// Creates an empty pool with custom block-capacity limits.
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

    /// Creates an empty pool using `allocator` and custom block-capacity limits.
    pub fn try_new_in(
        allocator: A,
        limits: BlockCapacityLimits,
    ) -> Result<Self, InvalidBlockCapacityLimits> {
        Ok(Self {
            hive: UnsafeCell::new(Hive::try_new_in(allocator, limits)?),
        })
    }

    /// Inserts `value` into the pool and returns a guard for the live object.
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

    /// Constructs a value from a closure and inserts it into the pool.
    ///
    /// The closure returns a fully initialized value, so this avoids the unsafe
    /// `MaybeUninit<T>` contract exposed by `Hive::insert_with_uninit`.
    pub fn emplace<F>(&self, f: F) -> Pooled<'_, T, A>
    where
        F: FnOnce() -> T,
    {
        self.insert(f())
    }

    /// Inserts `T::default()`, then lets `f` mutate the initialized element in
    /// place before returning its guard.
    ///
    /// If `f` panics, the default value remains in the pool and will be erased
    /// when the pool is dropped.
    pub fn insert_with<F>(&self, f: F) -> Pooled<'_, T, A>
    where
        T: Default,
        F: FnOnce(&mut T),
    {
        let mut value = self.insert(T::default());
        f(&mut value);
        value
    }

    /// Returns the number of live objects in the pool.
    pub fn len(&self) -> usize {
        // SAFETY: reading metadata does not create references to elements.
        unsafe { (&*self.hive.get()).len() }
    }

    /// Returns true if the pool contains no live objects.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the total element capacity of the backing hive.
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
/// `SyncPool` allows handles to be cloned and used from multiple threads. Each
/// [`SyncPooled`] guard still provides direct access to only one element; pool
/// metadata mutations are serialized by an internal mutex.
#[cfg(feature = "std")]
pub struct SyncPool<T, A: Allocator + Clone = Global> {
    hive: Arc<Mutex<Hive<T, A>>>,
}

/// A live object stored in a [`SyncPool`].
///
/// The guard may be sent to another thread when `T` and `A` are `Send`. Dropping
/// it removes the object from the pool.
#[cfg(feature = "std")]
pub struct SyncPooled<T, A: Allocator + Clone = Global> {
    hive: Arc<Mutex<Hive<T, A>>>,
    ptr: NonNull<T>,
    _owns_element: PhantomData<T>,
}

#[cfg(feature = "std")]
impl<T> SyncPool<T, Global> {
    /// Creates an empty synchronized pool.
    pub fn new() -> Self {
        Self::new_in(Global)
    }

    /// Creates an empty synchronized pool with space for at least `capacity`
    /// elements.
    pub fn with_capacity(capacity: usize) -> Self {
        let pool = Self::new();
        pool.hive.lock().unwrap().reserve(capacity);
        pool
    }

    /// Creates an empty synchronized pool with custom block-capacity limits.
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

    /// Creates an empty synchronized pool using `allocator` and custom
    /// block-capacity limits.
    pub fn try_new_in(
        allocator: A,
        limits: BlockCapacityLimits,
    ) -> Result<Self, InvalidBlockCapacityLimits> {
        Ok(Self {
            hive: Arc::new(Mutex::new(Hive::try_new_in(allocator, limits)?)),
        })
    }

    /// Inserts `value` and returns a guard that can move independently of the
    /// pool handle.
    pub fn insert(&self, value: T) -> SyncPooled<T, A> {
        let ptr = self.hive.lock().unwrap().insert_mut(value);
        let ptr = NonNull::new(ptr).expect("Hive::insert_mut returned null");

        SyncPooled {
            hive: self.hive.clone(),
            ptr,
            _owns_element: PhantomData,
        }
    }

    /// Constructs a value from a closure and inserts it into the synchronized
    /// pool.
    ///
    /// The closure runs before the pool lock is acquired, so expensive
    /// construction does not block unrelated pool operations.
    pub fn emplace<F>(&self, f: F) -> SyncPooled<T, A>
    where
        F: FnOnce() -> T,
    {
        self.insert(f())
    }

    /// Inserts `T::default()`, then lets `f` mutate the initialized element in
    /// place before returning its guard.
    ///
    /// If `f` panics, the default value remains in the pool and will be erased
    /// when the pool is dropped.
    pub fn insert_with<F>(&self, f: F) -> SyncPooled<T, A>
    where
        T: Default,
        F: FnOnce(&mut T),
    {
        let mut value = self.insert(T::default());
        f(&mut value);
        value
    }

    /// Returns the number of live objects in the pool.
    pub fn len(&self) -> usize {
        self.hive.lock().unwrap().len()
    }

    /// Returns true if the pool contains no live objects.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the total element capacity of the backing hive.
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
