use core::fmt::{self, Debug};
use core::marker::PhantomData;
use core::mem;

use crate::alloc::{Allocator, Global};

use super::super::borrow::DormantMutRef;
use super::super::node::{marker, Handle, NodeRef};
use super::BTreeMap;

use crate::alloc::AllocError;
#[cfg(test)]
use crate::testing::*;

use Entry::*;

/// A view into a single entry in a map, which may either be vacant or occupied.
///
/// This `enum` is constructed from the [`entry`] method on [`BTreeMap`].
///
/// [`entry`]: BTreeMap::entry
pub enum Entry<'a, K, V, A = Global>
where
    K: 'a,
    V: 'a,
    A: Allocator,
{
    /// A vacant entry.
    Vacant(VacantEntry<'a, K, V, A>),

    /// An occupied entry.
    Occupied(OccupiedEntry<'a, K, V, A>),
}

impl<K, V, A> Debug for Entry<'_, K, V, A>
where
    K: Debug + Ord,
    V: Debug,
    A: Allocator,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Vacant(ref v) => f.debug_tuple("Entry").field(v).finish(),
            Occupied(ref o) => f.debug_tuple("Entry").field(o).finish(),
        }
    }
}

/// A view into a vacant entry in a `BTreeMap`.
/// It is part of the [`Entry`] enum.
pub struct VacantEntry<'a, K, V, A = Global>
where
    A: Allocator,
{
    pub(super) key: K,
    /// `None` for a (empty) map without root
    pub(super) handle: Option<Handle<NodeRef<marker::Mut<'a>, K, V, marker::Leaf>, marker::Edge>>,
    pub(super) dormant_map: DormantMutRef<'a, BTreeMap<K, V, A>>,

    /// The BTreeMap will outlive this IntoIter so we don't care about drop order for `alloc`.
    pub(super) alloc: &'a A,

    // Be invariant in `K` and `V`
    pub(super) _marker: PhantomData<&'a mut (K, V)>,
}

impl<K, V, A> Debug for VacantEntry<'_, K, V, A>
where
    K: Debug + Ord,
    A: Allocator,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("VacantEntry").field(self.key()).finish()
    }
}

/// A view into an occupied entry in a `BTreeMap`.
/// It is part of the [`Entry`] enum.
pub struct OccupiedEntry<'a, K, V, A = Global>
where
    A: Allocator,
{
    pub(super) handle: Handle<NodeRef<marker::Mut<'a>, K, V, marker::LeafOrInternal>, marker::KV>,
    pub(super) dormant_map: DormantMutRef<'a, BTreeMap<K, V, A>>,

    /// The BTreeMap will outlive this IntoIter so we don't care about drop order for `alloc`.
    pub(super) alloc: &'a A,

    // Be invariant in `K` and `V`
    pub(super) _marker: PhantomData<&'a mut (K, V)>,
}

impl<K, V, A> Debug for OccupiedEntry<'_, K, V, A>
where
    K: Debug + Ord,
    V: Debug,
    A: Allocator,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OccupiedEntry")
            .field("key", self.key())
            .field("value", self.get())
            .finish()
    }
}

/// The error returned by [`try_insert`](BTreeMap::try_insert) when the key already exists.
///
/// Contains the occupied entry, and the value that was not inserted.
pub struct OccupiedError<'a, K, V, A = Global>
where
    K: 'a,
    V: 'a,
    A: Allocator,
{
    /// The entry in the map that was already occupied.
    pub entry: OccupiedEntry<'a, K, V, A>,
    /// The value which was not inserted, because the entry was already occupied.
    pub value: V,
}

impl<K, V, A> Debug for OccupiedError<'_, K, V, A>
where
    K: Debug + Ord,
    V: Debug,
    A: Allocator,
{
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OccupiedError")
            .field("key", self.entry.key())
            .field("old_value", self.entry.get())
            .field("new_value", &self.value)
            .finish()
    }
}

impl<K, V, A> fmt::Display for OccupiedError<'_, K, V, A>
where
    K: Debug + Ord,
    V: Debug,
    A: Allocator,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to insert {:?}, key {:?} already exists with value {:?}",
            self.value,
            self.entry.key(),
            self.entry.get(),
        )
    }
}

impl<'a, K, V, A> Entry<'a, K, V, A>
where
    K: Ord,
    A: Allocator,
{
    /// Ensures a value is in the entry by inserting the default if empty, and
    /// returns a mutable reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_try_insert(12)?;
    ///
    /// assert_eq!(map["poneyland"], 12);
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    #[inline]
    pub fn or_try_insert(self, default: V) -> Result<&'a mut V, AllocError> {
        match self {
            Occupied(entry) => Ok(entry.into_mut()),
            Vacant(entry) => entry.try_insert(default),
        }
    }

    /// Ensures a value is in the entry by inserting the result of the default
    /// function if empty, and returns a mutable reference to the value in the
    /// entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, String> = BTreeMap::new();
    /// let s = "hoho".to_string();
    ///
    /// map.entry("poneyland").or_try_insert_with(|| s)?;
    ///
    /// assert_eq!(map["poneyland"], "hoho".to_string());
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    #[inline]
    pub fn or_try_insert_with<F>(self, default: F) -> Result<&'a mut V, AllocError>
    where
        F: FnOnce() -> V,
    {
        match self {
            Occupied(entry) => Ok(entry.into_mut()),
            Vacant(entry) => entry.try_insert(default()),
        }
    }

    /// Ensures a value is in the entry by inserting, if empty, the result of
    /// the default function. This method allows for generating key-derived
    /// values for insertion by providing the default function a reference to
    /// the key that was moved during the `.entry(key)` method call.
    ///
    /// The reference to the moved key is provided so that cloning or copying
    /// the key is unnecessary, unlike with `.or_insert_with(|| ... )`.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    ///
    /// map.entry("poneyland").or_try_insert_with_key(|key| key.chars().count())?;
    ///
    /// assert_eq!(map["poneyland"], 9);
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    #[inline]
    pub fn or_try_insert_with_key<F>(self, default: F) -> Result<&'a mut V, AllocError>
    where
        F: FnOnce(&K) -> V,
    {
        match self {
            Occupied(entry) => Ok(entry.into_mut()),
            Vacant(entry) => {
                let value = default(entry.key());
                entry.try_insert(value)
            }
        }
    }

    /// Returns a reference to this entry's key.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// assert_eq!(map.entry("poneyland").key(), &"poneyland");
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    pub fn key(&self) -> &K {
        match *self {
            Occupied(ref entry) => entry.key(),
            Vacant(ref entry) => entry.key(),
        }
    }

    /// Provides in-place mutable access to an occupied entry before any
    /// potential inserts into the map.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    ///
    /// map.entry("poneyland")
    ///    .and_modify(|e| { *e += 1 })
    ///    .or_try_insert(42)?;
    /// assert_eq!(map["poneyland"], 42);
    ///
    /// map.entry("poneyland")
    ///    .and_modify(|e| { *e += 1 })
    ///    .or_try_insert(42)?;
    /// assert_eq!(map["poneyland"], 43);
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    pub fn and_modify<F>(self, f: F) -> Self
    where
        F: FnOnce(&mut V),
    {
        match self {
            Occupied(mut entry) => {
                f(entry.get_mut());
                Occupied(entry)
            }
            Vacant(entry) => Vacant(entry),
        }
    }
}

impl<'a, K, V, A> Entry<'a, K, V, A>
where
    K: Ord,
    V: Default,
    A: Allocator,
{
    /// Ensures a value is in the entry by inserting the default value if empty,
    /// and returns a mutable reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, Option<usize>> = BTreeMap::new();
    /// map.entry("poneyland").or_try_default()?;
    ///
    /// assert_eq!(map["poneyland"], None);
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    pub fn or_try_default(self) -> Result<&'a mut V, AllocError> {
        match self {
            Occupied(entry) => Ok(entry.into_mut()),
            Vacant(entry) => entry.try_insert(Default::default()),
        }
    }
}

impl<'a, K, V, A> VacantEntry<'a, K, V, A>
where
    A: Allocator,
{
    /// Gets a reference to the key that would be used when inserting a value
    /// through the VacantEntry.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// assert_eq!(map.entry("poneyland").key(), &"poneyland");
    /// ```
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Take ownership of the key.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    /// use rune::alloc::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    ///
    /// if let Entry::Vacant(v) = map.entry("poneyland") {
    ///     v.into_key();
    /// }
    /// ```
    #[inline]
    pub fn into_key(self) -> K {
        self.key
    }

    /// Sets the value of the entry with the `VacantEntry`'s key,
    /// and returns a mutable reference to it.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    /// use rune::alloc::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, u32> = BTreeMap::new();
    ///
    /// if let Entry::Vacant(o) = map.entry("poneyland") {
    ///     o.try_insert(37)?;
    /// }
    ///
    /// assert_eq!(map["poneyland"], 37);
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    pub fn try_insert(mut self, value: V) -> Result<&'a mut V, AllocError> {
        let out_ptr = match self.handle {
            None => {
                // SAFETY: There is no tree yet so no reference to it exists.
                let map = unsafe { self.dormant_map.awaken() };
                let mut root = NodeRef::new_leaf(self.alloc)?;
                let val_ptr = root.borrow_mut().push(self.key, value) as *mut V;
                map.root = Some(root.forget_type());
                map.length = 1;
                val_ptr
            }
            Some(handle) => {
                let new_handle = handle.insert_recursing(self.key, value, self.alloc, |ins| {
                    drop(ins.left);
                    // SAFETY: Pushing a new root node doesn't invalidate
                    // handles to existing nodes.
                    let map = unsafe { self.dormant_map.reborrow() };
                    let root = map.root.as_mut().unwrap(); // same as ins.left
                    root.push_internal_level(self.alloc)?
                        .push(ins.kv.0, ins.kv.1, ins.right);
                    Ok(())
                })?;

                // Get the pointer to the value
                let val_ptr = new_handle.into_val_mut();

                // SAFETY: We have consumed self.handle.
                let map = unsafe { self.dormant_map.awaken() };
                map.length += 1;
                val_ptr
            }
        };

        // Now that we have finished growing the tree using borrowed references,
        // dereference the pointer to a part of it, that we picked up along the way.
        Ok(unsafe { &mut *out_ptr })
    }

    #[cfg(test)]
    pub(crate) fn insert(self, value: V) -> &'a mut V {
        self.try_insert(value).abort()
    }
}

impl<'a, K, V, A> OccupiedEntry<'a, K, V, A>
where
    A: Allocator,
{
    /// Gets a reference to the key in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_try_insert(12)?;
    /// assert_eq!(map.entry("poneyland").key(), &"poneyland");
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    #[must_use]
    pub fn key(&self) -> &K {
        self.handle.reborrow().into_kv().0
    }

    /// Take ownership of the key and value from the map.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    /// use rune::alloc::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_try_insert(12)?;
    ///
    /// if let Entry::Occupied(o) = map.entry("poneyland") {
    ///     // We delete the entry from the map.
    ///     assert_eq!(o.remove_entry(), ("poneyland", 12));
    /// }
    ///
    /// // If now try to get the value, it will panic:
    /// // println!("{}", map["poneyland"]);
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    pub fn remove_entry(self) -> (K, V) {
        self.remove_kv()
    }

    /// Gets a reference to the value in the entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    /// use rune::alloc::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_try_insert(12)?;
    ///
    /// if let Entry::Occupied(o) = map.entry("poneyland") {
    ///     assert_eq!(o.get(), &12);
    /// }
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    #[must_use]
    pub fn get(&self) -> &V {
        self.handle.reborrow().into_kv().1
    }

    /// Gets a mutable reference to the value in the entry.
    ///
    /// If you need a reference to the `OccupiedEntry` that may outlive the
    /// destruction of the `Entry` value, see [`into_mut`].
    ///
    /// [`into_mut`]: OccupiedEntry::into_mut
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    /// use rune::alloc::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_try_insert(12)?;
    ///
    /// assert_eq!(map["poneyland"], 12);
    /// if let Entry::Occupied(mut o) = map.entry("poneyland") {
    ///     *o.get_mut() += 10;
    ///     assert_eq!(*o.get(), 22);
    ///
    ///     // We can use the same Entry multiple times.
    ///     *o.get_mut() += 2;
    /// }
    /// assert_eq!(map["poneyland"], 24);
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    pub fn get_mut(&mut self) -> &mut V {
        self.handle.kv_mut().1
    }

    /// Converts the entry into a mutable reference to its value.
    ///
    /// If you need multiple references to the `OccupiedEntry`, see [`get_mut`].
    ///
    /// [`get_mut`]: OccupiedEntry::get_mut
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    /// use rune::alloc::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_try_insert(12)?;
    ///
    /// assert_eq!(map["poneyland"], 12);
    /// if let Entry::Occupied(o) = map.entry("poneyland") {
    ///     *o.into_mut() += 10;
    /// }
    /// assert_eq!(map["poneyland"], 22);
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    #[must_use = "`self` will be dropped if the result is not used"]
    pub fn into_mut(self) -> &'a mut V {
        self.handle.into_val_mut()
    }

    /// Sets the value of the entry with the `OccupiedEntry`'s key,
    /// and returns the entry's old value.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    /// use rune::alloc::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_try_insert(12)?;
    ///
    /// if let Entry::Occupied(mut o) = map.entry("poneyland") {
    ///     assert_eq!(o.insert(15), 12);
    /// }
    ///
    /// assert_eq!(map["poneyland"], 15);
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    pub fn insert(&mut self, value: V) -> V {
        mem::replace(self.get_mut(), value)
    }

    /// Takes the value of the entry out of the map, and returns it.
    ///
    /// # Examples
    ///
    /// ```
    /// use rune::alloc::BTreeMap;
    /// use rune::alloc::btree_map::Entry;
    ///
    /// let mut map: BTreeMap<&str, usize> = BTreeMap::new();
    /// map.entry("poneyland").or_try_insert(12)?;
    ///
    /// if let Entry::Occupied(o) = map.entry("poneyland") {
    ///     assert_eq!(o.remove(), 12);
    /// }
    ///
    /// // If we try to get "poneyland"'s value, it'll panic:
    /// // println!("{}", map["poneyland"]);
    /// # Ok::<_, rune::alloc::Error>(())
    /// ```
    pub fn remove(self) -> V {
        self.remove_kv().1
    }

    // Body of `remove_entry`, probably separate because the name reflects the returned pair.
    pub(super) fn remove_kv(self) -> (K, V) {
        let mut emptied_internal_root = false;
        let (old_kv, _) = self
            .handle
            .remove_kv_tracking(|| emptied_internal_root = true, self.alloc);
        // SAFETY: we consumed the intermediate root borrow, `self.handle`.
        let map = unsafe { self.dormant_map.awaken() };
        map.length -= 1;
        if emptied_internal_root {
            let root = map.root.as_mut().unwrap();
            root.pop_internal_level(self.alloc);
        }
        old_kv
    }
}
