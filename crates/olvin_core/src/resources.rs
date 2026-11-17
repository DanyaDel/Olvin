use std::any::{Any, TypeId, type_name};
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;

/// Type-keyed storage for engine-wide singletons (ECS world, renderer, input...).
///
/// Borrows are checked at runtime: different resources can be borrowed mutably
/// at the same time, the same one twice cannot.
#[derive(Default)]
pub struct Resources {
    map: HashMap<TypeId, RefCell<Box<dyn Any>>>,
}

impl Resources {
    /// Inserts a resource, returning the previous value of the same type.
    pub fn insert<T: 'static>(&mut self, value: T) -> Option<T> {
        self.map
            .insert(TypeId::of::<T>(), RefCell::new(Box::new(value)))
            .map(|old| *old.into_inner().downcast::<T>().expect("resource type mismatch"))
    }

    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .map(|old| *old.into_inner().downcast::<T>().expect("resource type mismatch"))
    }

    pub fn contains<T: 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    pub fn get<T: 'static>(&self) -> Option<Ref<'_, T>> {
        let cell = self.map.get(&TypeId::of::<T>())?;
        Some(Ref::map(cell.borrow(), |b| b.downcast_ref::<T>().unwrap()))
    }

    pub fn get_mut<T: 'static>(&self) -> Option<RefMut<'_, T>> {
        let cell = self.map.get(&TypeId::of::<T>())?;
        Some(RefMut::map(cell.borrow_mut(), |b| b.downcast_mut::<T>().unwrap()))
    }

    /// Like [`Resources::get`], but panics with the type name if the resource is missing.
    #[track_caller]
    pub fn fetch<T: 'static>(&self) -> Ref<'_, T> {
        self.get::<T>()
            .unwrap_or_else(|| panic!("resource `{}` is missing", type_name::<T>()))
    }

    #[track_caller]
    pub fn fetch_mut<T: 'static>(&self) -> RefMut<'_, T> {
        self.get_mut::<T>()
            .unwrap_or_else(|| panic!("resource `{}` is missing", type_name::<T>()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_replace_remove() {
        let mut res = Resources::default();
        assert!(res.insert(1u32).is_none());
        res.insert(String::from("a"));
        {
            // Two different resources can be borrowed mutably at once.
            let mut n = res.fetch_mut::<u32>();
            let mut s = res.fetch_mut::<String>();
            *n += 1;
            s.push('b');
        }
        assert_eq!(*res.fetch::<u32>(), 2);
        assert_eq!(res.insert(5u32), Some(2));
        assert_eq!(res.remove::<String>().as_deref(), Some("ab"));
        assert!(!res.contains::<String>());
    }
}
