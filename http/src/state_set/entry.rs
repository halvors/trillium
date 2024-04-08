use super::Value;
use std::{any::TypeId, collections::btree_map, marker::PhantomData};

pub enum Entry<'a, T> {
    Vacant(VacantEntry<'a, T>),
    Occupied(OccupiedEntry<'a, T>),
}

pub struct VacantEntry<'a, T>(
    pub(super) btree_map::VacantEntry<'a, TypeId, Value>,
    PhantomData<T>,
);
pub struct OccupiedEntry<'a, T>(
    pub(super) btree_map::OccupiedEntry<'a, TypeId, Value>,
    PhantomData<T>,
);

impl<'a, T: Send + Sync + 'static> Entry<'a, T> {
    pub fn or_insert(self, default: T) -> &'a mut T {
        match self {
            Entry::Vacant(vacant) => vacant.insert(default),
            Entry::Occupied(occupied) => occupied.into_mut(),
        }
    }

    pub fn or_insert_with(self, default: impl FnOnce() -> T) -> &'a mut T {
        match self {
            Entry::Vacant(vacant) => vacant.insert(default()),
            Entry::Occupied(occupied) => occupied.into_mut(),
        }
    }

    pub fn and_modify(self, f: impl FnOnce(&mut T)) -> Self {
        match self {
            Entry::Vacant(vacant) => Entry::Vacant(vacant),
            Entry::Occupied(mut occupied) => {
                f(occupied.get_mut());
                Entry::Occupied(occupied)
            }
        }
    }

    pub fn take(self) -> Option<T> {
        match self {
            Entry::Vacant(_) => None,
            Entry::Occupied(occupied) => Some(occupied.remove()),
        }
    }

    pub(super) fn new(entry: btree_map::Entry<'a, TypeId, Value>) -> Self {
        match entry {
            btree_map::Entry::Vacant(vacant) => Self::Vacant(VacantEntry(vacant, PhantomData)),
            btree_map::Entry::Occupied(occupied) => {
                Self::Occupied(OccupiedEntry(occupied, PhantomData))
            }
        }
    }
}

impl<'a, T: Default + Send + Sync + 'static> Entry<'a, T> {
    pub fn or_default(self) -> &'a mut T {
        #[allow(clippy::unwrap_or_default)] // this is the implementation of or_default
        self.or_insert_with(T::default)
    }
}

impl<'a, T: Send + Sync + 'static> VacantEntry<'a, T> {
    pub fn insert(self, value: T) -> &'a mut T {
        self.0.insert(Box::new(value)).downcast_mut().unwrap()
    }
}

impl<'a, T: Send + Sync + 'static> OccupiedEntry<'a, T> {
    pub fn get(&self) -> &T {
        self.0.get().downcast_ref().unwrap()
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.0.get_mut().downcast_mut().unwrap()
    }

    pub fn insert(&mut self, value: T) -> T {
        *self.0.insert(Box::new(value)).downcast().unwrap()
    }

    pub fn remove(self) -> T {
        *self.0.remove().downcast().unwrap()
    }

    pub fn into_mut(self) -> &'a mut T {
        self.0.into_mut().downcast_mut().unwrap()
    }
}
