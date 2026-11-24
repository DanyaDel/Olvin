//! Lending a `&mut T` to code that cannot take a lifetime parameter (Lua
//! callbacks), and getting it back.
//!
//! This is the only `unsafe` of the scripting layer, kept in one small file
//! without Lua so it can run under Miri (`tools/miri.sh`). The rules:
//!
//! 1. [`Slot::lend`] stores a pointer made from `&'a mut T` and returns a
//!    guard that borrows both the slot and the value for `'a`: while the
//!    guard lives the compiler refuses every other use of the value.
//! 2. The guard's `Drop` puts the previous state back, so the pointer never
//!    outlives the borrow, also when the callback panics.
//! 3. [`Slot::with`] gives out `&mut T` only while no other `with` is running
//!    on the slot: a re-entrant call (a Lua `__gc` finalizer running inside
//!    an API call, say) gets [`SlotError::Busy`] instead of a second `&mut`.
#![cfg_attr(not(feature = "lua"), allow(dead_code))]

use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// Why [`Slot::with`] did not run its closure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotError {
    /// Nothing is lent: the callback ended (a script stored `ctx`).
    NotLent,
    /// Another `with` is running on this slot.
    Busy,
}

impl fmt::Display for SlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotLent => "ctx is only valid inside the callback it was passed to",
            Self::Busy => "ctx cannot be used from inside another ctx call (a finalizer?)",
        })
    }
}

/// A place a `&mut T` can be lent to, null when nothing is lent.
pub(crate) struct Slot<T> {
    ptr: Cell<Option<NonNull<T>>>,
    busy: Cell<bool>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            ptr: Cell::new(None),
            busy: Cell::new(false),
        }
    }
}

impl<T> Slot<T> {
    /// Runs `f` on the lent value.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R, SlotError> {
        let Some(mut ptr) = self.ptr.get() else {
            return Err(SlotError::NotLent);
        };
        if self.busy.replace(true) {
            return Err(SlotError::Busy);
        }
        // Clears `busy` also if `f` panics.
        let _busy = Reset(&self.busy);
        // SAFETY: `ptr` comes from the `&'a mut T` that `lend` took, and the
        // guard from that call (which borrows the value for `'a`) is alive,
        // since its `Drop` is what clears `ptr`. `busy` makes this the only
        // `&mut T` made from the pointer right now, and `f` cannot keep it
        // past this call.
        Ok(f(unsafe { ptr.as_mut() }))
    }

    /// Lends `value` until the guard drops. Lending again while lent hides the
    /// first value until the second guard drops.
    pub(crate) fn lend<'a>(&'a self, value: &'a mut T) -> Lent<'a, T> {
        let previous = self.ptr.replace(Some(NonNull::from(value)));
        Lent {
            slot: self,
            previous,
            _value: PhantomData,
        }
    }
}

/// The loan: dropping it takes the value back.
pub(crate) struct Lent<'a, T> {
    slot: &'a Slot<T>,
    previous: Option<NonNull<T>>,
    _value: PhantomData<&'a mut T>,
}

impl<T> Drop for Lent<'_, T> {
    fn drop(&mut self) {
        self.slot.ptr.set(self.previous);
    }
}

struct Reset<'a>(&'a Cell<bool>);

impl Drop for Reset<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_value_is_reachable_only_while_lent_and_changes_stick() {
        let slot = Slot::<Vec<i32>>::default();
        let mut value = vec![1];
        assert_eq!(slot.with(|v| v.len()), Err(SlotError::NotLent));
        {
            let _lent = slot.lend(&mut value);
            slot.with(|v| v.push(2)).unwrap();
            slot.with(|v| v.push(3)).unwrap();
        }
        assert_eq!(slot.with(|v| v.len()), Err(SlotError::NotLent));
        assert_eq!(value, [1, 2, 3]);
    }

    #[test]
    fn a_reentrant_with_is_refused_instead_of_aliasing() {
        let slot = Slot::<String>::default();
        let mut value = String::from("a");
        let _lent = slot.lend(&mut value);
        let inner = slot
            .with(|outer| {
                outer.push('b');
                let inner = slot.with(|again| again.push('X'));
                outer.push('c');
                inner
            })
            .unwrap();
        assert_eq!(inner, Err(SlotError::Busy));
        slot.with(|v| assert_eq!(v, "abc")).unwrap();
        // The refused call did not leave the slot stuck.
        slot.with(|v| v.push('d')).unwrap();
    }

    #[test]
    fn a_panic_in_the_closure_does_not_stick_the_slot() {
        let slot = Slot::<i32>::default();
        let mut value = 0;
        let _lent = slot.lend(&mut value);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = slot.with(|_| panic!("callback failed"));
        }));
        assert!(caught.is_err());
        assert_eq!(slot.with(|v| *v += 1), Ok(()));
    }

    #[test]
    fn a_panic_while_lent_clears_the_pointer() {
        let slot = Slot::<i32>::default();
        let mut value = 5;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _lent = slot.lend(&mut value);
            panic!("callback failed");
        }));
        assert!(caught.is_err());
        assert_eq!(slot.with(|v| *v), Err(SlotError::NotLent));
        assert_eq!(value, 5);
    }

    #[test]
    fn lending_again_hides_the_first_value_until_the_second_guard_drops() {
        let slot = Slot::<i32>::default();
        let (mut a, mut b) = (1, 2);
        let first = slot.lend(&mut a);
        {
            let _second = slot.lend(&mut b);
            slot.with(|v| *v += 10).unwrap();
        }
        slot.with(|v| *v += 100).unwrap();
        drop(first);
        assert_eq!((a, b), (101, 12));
    }
}
