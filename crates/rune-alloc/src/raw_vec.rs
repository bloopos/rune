use core::alloc::{Layout, LayoutError};
use core::cmp;
use core::mem::{self, ManuallyDrop, MaybeUninit};
use core::slice;

use crate::alloc::SizedTypeProperties;
use crate::alloc::{AllocError, Allocator, Global};
use crate::boxed::Box;
use crate::error::Error;
use crate::ptr::{self, NonNull, Unique};

enum AllocInit {
    /// The contents of the new memory are uninitialized.
    Uninitialized,
    /// The new memory is guaranteed to be zeroed.
    #[cfg(rune_nightly)]
    Zeroed,
}

/// A low-level utility for more ergonomically allocating, reallocating, and deallocating
/// a buffer of memory on the heap without having to worry about all the corner cases
/// involved. This type is excellent for building your own data structures like Vec and VecDeque.
/// In particular:
///
/// * Produces `Unique::dangling()` on zero-sized types.
/// * Produces `Unique::dangling()` on zero-length allocations.
/// * Avoids freeing `Unique::dangling()`.
/// * Catches all overflows in capacity computations (promotes them to "capacity overflow" panics).
/// * Guards against 32-bit systems allocating more than isize::MAX bytes.
/// * Guards against overflowing your length.
/// * Calls `handle_alloc_error` for fallible allocations.
/// * Contains a `ptr::Unique` and thus endows the user with all related benefits.
/// * Uses the excess returned from the allocator to use the largest available capacity.
///
/// This type does not in anyway inspect the memory that it manages. When dropped it *will*
/// free its memory, but it *won't* try to drop its contents. It is up to the user of `RawVec`
/// to handle the actual things *stored* inside of a `RawVec`.
///
/// Note that the excess of a zero-sized types is always infinite, so `capacity()` always returns
/// `usize::MAX`. This means that you need to be careful when round-tripping this type with a
/// `Box<[T]>`, since `capacity()` won't yield the length.
#[allow(missing_debug_implementations)]
pub(crate) struct RawVec<T, A: Allocator = Global> {
    ptr: Unique<T>,
    cap: usize,
    alloc: A,
}

impl<T> RawVec<T, Global> {
    /// HACK(Centril): This exists because stable `const fn` can only call
    /// stable `const fn`, so they cannot call `Self::new()`.
    ///
    /// If you change `RawVec<T>::new` or dependencies, please take care to not
    /// introduce anything that would truly const-call something unstable.
    pub const NEW: Self = Self::new();

    /// Creates the biggest possible `RawVec` (on the system heap)
    /// without allocating. If `T` has positive size, then this makes a
    /// `RawVec` with capacity `0`. If `T` is zero-sized, then it makes a
    /// `RawVec` with capacity `usize::MAX`. Useful for implementing
    /// delayed allocation.
    #[must_use]
    pub const fn new() -> Self {
        Self::new_in(Global)
    }
}

impl<T, A> RawVec<T, A>
where
    A: Allocator,
{
    // Tiny Vecs are dumb. Skip to:
    // - 8 if the element size is 1, because any heap allocators is likely
    //   to round up a request of less than 8 bytes to at least 8 bytes.
    // - 4 if elements are moderate-sized (<= 1 KiB).
    // - 1 otherwise, to avoid wasting too much space for very short Vecs.
    pub(crate) const MIN_NON_ZERO_CAP: usize = if mem::size_of::<T>() == 1 {
        8
    } else if mem::size_of::<T>() <= 1024 {
        4
    } else {
        1
    };

    /// Like `new`, but parameterized over the choice of allocator for
    /// the returned `RawVec`.
    pub const fn new_in(alloc: A) -> Self {
        // `cap: 0` means "unallocated". zero-sized types are ignored.
        Self {
            ptr: Unique::dangling(),
            cap: 0,
            alloc,
        }
    }

    /// Like `with_capacity`, but parameterized over the choice of
    /// allocator for the returned `RawVec`.
    #[inline]
    pub(crate) fn try_with_capacity_in(capacity: usize, alloc: A) -> Result<Self, Error> {
        Self::try_allocate_in(capacity, AllocInit::Uninitialized, alloc)
    }

    /// Like `with_capacity_zeroed`, but parameterized over the choice
    /// of allocator for the returned `RawVec`.
    #[inline]
    #[cfg(rune_nightly)]
    pub(crate) fn try_with_capacity_zeroed_in(capacity: usize, alloc: A) -> Result<Self, Error> {
        Self::try_allocate_in(capacity, AllocInit::Zeroed, alloc)
    }

    /// Converts the entire buffer into `Box<[MaybeUninit<T>]>` with the specified `len`.
    ///
    /// Note that this will correctly reconstitute any `cap` changes
    /// that may have been performed. (See description of type for details.)
    ///
    /// # Safety
    ///
    /// * `len` must be greater than or equal to the most recently requested capacity, and
    /// * `len` must be less than or equal to `self.capacity()`.
    ///
    /// Note, that the requested capacity and `self.capacity()` could differ, as
    /// an allocator could overallocate and return a greater memory block than requested.
    pub unsafe fn into_box(self, len: usize) -> Box<[MaybeUninit<T>], A> {
        // Sanity-check one half of the safety requirement (we cannot check the other half).
        debug_assert!(
            len <= self.capacity(),
            "`len` must be smaller than or equal to `self.capacity()`"
        );

        let me = ManuallyDrop::new(self);
        unsafe {
            let slice = slice::from_raw_parts_mut(me.ptr() as *mut MaybeUninit<T>, len);
            Box::from_raw_in(slice, ptr::read(&me.alloc))
        }
    }

    fn try_allocate_in(capacity: usize, init: AllocInit, alloc: A) -> Result<Self, Error> {
        // Don't allocate here because `Drop` will not deallocate when `capacity` is 0.
        if T::IS_ZST || capacity == 0 {
            Ok(Self::new_in(alloc))
        } else {
            // We avoid `unwrap_or_else` here because it bloats the amount of
            // LLVM IR generated.
            let layout = match Layout::array::<T>(capacity) {
                Ok(layout) => layout,
                Err(_) => return Err(Error::CapacityOverflow),
            };
            match alloc_guard(layout.size()) {
                Ok(_) => {}
                Err(_) => return Err(Error::CapacityOverflow),
            }
            let ptr = match init {
                AllocInit::Uninitialized => alloc.allocate(layout)?,
                #[cfg(rune_nightly)]
                AllocInit::Zeroed => alloc.allocate_zeroed(layout)?,
            };

            // Allocators currently return a `NonNull<[u8]>` whose length
            // matches the size requested. If that ever changes, the capacity
            // here should change to `ptr.len() / mem::size_of::<T>()`.
            Ok(Self {
                ptr: unsafe { Unique::new_unchecked(ptr.cast().as_ptr()) },
                cap: capacity,
                alloc,
            })
        }
    }

    /// Reconstitutes a `RawVec` from a pointer, capacity, and allocator.
    ///
    /// # Safety
    ///
    /// The `ptr` must be allocated (via the given allocator `alloc`), and with the given
    /// `capacity`.
    /// The `capacity` cannot exceed `isize::MAX` for sized types. (only a concern on 32-bit
    /// systems). ZST vectors may have a capacity up to `usize::MAX`.
    /// If the `ptr` and `capacity` come from a `RawVec` created via `alloc`, then this is
    /// guaranteed.
    #[inline]
    pub unsafe fn from_raw_parts_in(ptr: *mut T, capacity: usize, alloc: A) -> Self {
        Self {
            ptr: unsafe { Unique::new_unchecked(ptr) },
            cap: capacity,
            alloc,
        }
    }

    /// Gets a raw pointer to the start of the allocation. Note that this is
    /// `Unique::dangling()` if `capacity == 0` or `T` is zero-sized. In the former case, you must
    /// be careful.
    #[inline]
    pub(crate) fn ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }

    /// Gets the capacity of the allocation.
    ///
    /// This will always be `usize::MAX` if `T` is zero-sized.
    #[inline(always)]
    pub(crate) fn capacity(&self) -> usize {
        if T::IS_ZST {
            usize::MAX
        } else {
            self.cap
        }
    }

    /// Returns a shared reference to the allocator backing this `RawVec`.
    pub(crate) fn allocator(&self) -> &A {
        &self.alloc
    }

    fn current_memory(&self) -> Option<(NonNull<u8>, Layout)> {
        if T::IS_ZST || self.cap == 0 {
            None
        } else {
            // We could use Layout::array here which ensures the absence of isize and usize overflows
            // and could hypothetically handle differences between stride and size, but this memory
            // has already been allocated so we know it can't overflow and currently rust does not
            // support such types. So we can do better by skipping some checks and avoid an unwrap.
            assert!(mem::size_of::<T>() % mem::align_of::<T>() == 0);

            unsafe {
                let align = mem::align_of::<T>();
                let size = mem::size_of::<T>().wrapping_mul(self.cap);
                let layout = Layout::from_size_align_unchecked(size, align);
                Some((self.ptr.cast().into(), layout))
            }
        }
    }

    /// Ensures that the buffer contains at least enough space to hold `len +
    /// additional` elements. If it doesn't already have enough capacity, will
    /// reallocate enough space plus comfortable slack space to get amortized
    /// *O*(1) behavior. Will limit this behavior if it would needlessly cause
    /// itself to panic.
    ///
    /// If `len` exceeds `self.capacity()`, this may fail to actually allocate
    /// the requested space. This is not really unsafe, but the unsafe
    /// code *you* write that relies on the behavior of this function may break.
    ///
    /// This is ideal for implementing a bulk-push operation like `extend`.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity exceeds `isize::MAX` bytes.
    ///
    /// # Aborts
    ///
    /// Aborts on OOM.
    pub(crate) fn try_reserve(&mut self, len: usize, additional: usize) -> Result<(), Error> {
        if self.needs_to_grow(len, additional) {
            self.grow_amortized(len, additional)?;
        }

        Ok(())
    }

    /// A specialized version of `reserve()` used only by the hot and
    /// oft-instantiated `Vec::push()`, which does its own capacity check.
    pub(crate) fn try_reserve_for_push(&mut self, len: usize) -> Result<(), Error> {
        self.grow_amortized(len, 1)
    }

    /// The same as `reserve_exact`, but returns on errors instead of panicking or aborting.
    pub(crate) fn try_reserve_exact(&mut self, len: usize, additional: usize) -> Result<(), Error> {
        if self.needs_to_grow(len, additional) {
            self.grow_exact(len, additional)
        } else {
            Ok(())
        }
    }

    /// Shrinks the buffer down to the specified capacity. If the given amount
    /// is 0, actually completely deallocates.
    ///
    /// # Aborts
    ///
    /// Aborts on OOM.
    pub(crate) fn try_shrink_to_fit(&mut self, cap: usize) -> Result<(), AllocError> {
        self.shrink(cap)
    }
}

impl<T, A> RawVec<T, A>
where
    A: Allocator,
{
    /// Returns if the buffer needs to grow to fulfill the needed extra capacity.
    /// Mainly used to make inlining reserve-calls possible without inlining `grow`.
    fn needs_to_grow(&self, len: usize, additional: usize) -> bool {
        additional > self.capacity().wrapping_sub(len)
    }

    fn set_ptr_and_cap(&mut self, ptr: NonNull<[u8]>, cap: usize) {
        // Allocators currently return a `NonNull<[u8]>` whose length matches
        // the size requested. If that ever changes, the capacity here should
        // change to `ptr.len() / mem::size_of::<T>()`.
        self.ptr = unsafe { Unique::new_unchecked(ptr.cast().as_ptr()) };
        self.cap = cap;
    }

    // This method is usually instantiated many times. So we want it to be as
    // small as possible, to improve compile times. But we also want as much of
    // its contents to be statically computable as possible, to make the
    // generated code run faster. Therefore, this method is carefully written
    // so that all of the code that depends on `T` is within it, while as much
    // of the code that doesn't depend on `T` as possible is in functions that
    // are non-generic over `T`.
    fn grow_amortized(&mut self, len: usize, additional: usize) -> Result<(), Error> {
        // This is ensured by the calling contexts.
        debug_assert!(additional > 0);

        if T::IS_ZST {
            // Since we return a capacity of `usize::MAX` when `elem_size` is
            // 0, getting to here necessarily means the `RawVec` is overfull.
            return Err(Error::CapacityOverflow);
        }

        // Nothing we can really do about these checks, sadly.
        let required_cap = len.checked_add(additional).ok_or(Error::CapacityOverflow)?;

        // This guarantees exponential growth. The doubling cannot overflow
        // because `cap <= isize::MAX` and the type of `cap` is `usize`.
        let cap = cmp::max(self.cap * 2, required_cap);
        let cap = cmp::max(Self::MIN_NON_ZERO_CAP, cap);

        let new_layout = Layout::array::<T>(cap);

        // `finish_grow` is non-generic over `T`.
        let ptr = finish_grow(new_layout, self.current_memory(), &self.alloc)?;
        self.set_ptr_and_cap(ptr, cap);
        Ok(())
    }

    // The constraints on this method are much the same as those on
    // `grow_amortized`, but this method is usually instantiated less often so
    // it's less critical.
    fn grow_exact(&mut self, len: usize, additional: usize) -> Result<(), Error> {
        if T::IS_ZST {
            // Since we return a capacity of `usize::MAX` when the type size is
            // 0, getting to here necessarily means the `RawVec` is overfull.
            return Err(Error::CapacityOverflow);
        }

        let cap = len.checked_add(additional).ok_or(Error::CapacityOverflow)?;
        let new_layout = Layout::array::<T>(cap);

        // `finish_grow` is non-generic over `T`.
        let ptr = finish_grow(new_layout, self.current_memory(), &self.alloc)?;
        self.set_ptr_and_cap(ptr, cap);
        Ok(())
    }

    fn shrink(&mut self, cap: usize) -> Result<(), AllocError> {
        // See current_memory() why this assert is here
        assert!(mem::size_of::<T>() % mem::align_of::<T>() == 0);
        assert!(
            cap <= self.capacity(),
            "Tried to shrink to a larger capacity"
        );

        let (ptr, layout) = if let Some(mem) = self.current_memory() {
            mem
        } else {
            return Ok(());
        };

        // If shrinking to 0, deallocate the buffer. We don't reach this point
        // for the T::IS_ZST case since current_memory() will have returned
        // None.
        if cap == 0 {
            unsafe { self.alloc.deallocate(ptr, layout) };
            self.ptr = Unique::dangling();
            self.cap = 0;
        } else {
            let ptr = unsafe {
                // `Layout::array` cannot overflow here because it would have
                // overflowed earlier when capacity was larger.
                let new_size = mem::size_of::<T>().wrapping_mul(cap);
                let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
                self.alloc
                    .shrink(ptr, layout, new_layout)
                    .map_err(|_| AllocError { layout: new_layout })?
            };
            self.set_ptr_and_cap(ptr, cap);
        }
        Ok(())
    }
}

// This function is outside `RawVec` to minimize compile times. See the comment
// above `RawVec::grow_amortized` for details. (The `A` parameter isn't
// significant, because the number of different `A` types seen in practice is
// much smaller than the number of `T` types.)
#[inline(never)]
fn finish_grow<A>(
    new_layout: Result<Layout, LayoutError>,
    current_memory: Option<(NonNull<u8>, Layout)>,
    alloc: &A,
) -> Result<NonNull<[u8]>, Error>
where
    A: Allocator,
{
    // Check for the error here to minimize the size of `RawVec::grow_*`.
    let new_layout = new_layout.map_err(|_| Error::CapacityOverflow)?;

    alloc_guard(new_layout.size())?;

    let memory = if let Some((ptr, old_layout)) = current_memory {
        debug_assert_eq!(old_layout.align(), new_layout.align());
        unsafe {
            // The allocator checks for alignment equality
            debug_assert!(old_layout.align() == new_layout.align());
            alloc.grow(ptr, old_layout, new_layout)
        }
    } else {
        alloc.allocate(new_layout)
    };

    memory.map_err(|_| AllocError { layout: new_layout }.into())
}

#[cfg(not(rune_nightly))]
impl<T, A> Drop for RawVec<T, A>
where
    A: Allocator,
{
    /// Frees the memory owned by the `RawVec` *without* trying to drop its contents.
    fn drop(&mut self) {
        if let Some((ptr, layout)) = self.current_memory() {
            unsafe { self.alloc.deallocate(ptr, layout) }
        }
    }
}

#[cfg(rune_nightly)]
unsafe impl<#[may_dangle] T, A> Drop for RawVec<T, A>
where
    A: Allocator,
{
    /// Frees the memory owned by the `RawVec` *without* trying to drop its contents.
    fn drop(&mut self) {
        if let Some((ptr, layout)) = self.current_memory() {
            unsafe { self.alloc.deallocate(ptr, layout) }
        }
    }
}

// We need to guarantee the following:
// * We don't ever allocate `> isize::MAX` byte-size objects.
// * We don't overflow `usize::MAX` and actually allocate too little.
//
// On 64-bit we just need to check for overflow since trying to allocate
// `> isize::MAX` bytes will surely fail. On 32-bit and 16-bit we need to add
// an extra guard for this in case we're running on a platform which can use
// all 4GB in user-space, e.g., PAE or x32.

#[inline]
fn alloc_guard(alloc_size: usize) -> Result<(), Error> {
    if usize::BITS < 64 && alloc_size > isize::MAX as usize {
        Err(Error::CapacityOverflow)
    } else {
        Ok(())
    }
}
