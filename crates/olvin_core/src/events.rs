use crate::Resources;

/// Per-system read position in an [`Events`] queue. Each reader owns one, so
/// several systems can read the same events independently.
#[derive(Clone, Copy, Debug, Default)]
pub struct EventCursor {
    next: u64,
}

/// Typed event queue, a resource. Systems [`send`](Events::send) and read with
/// an [`EventCursor`]; `App::add_event` swaps the buffers once per frame.
///
/// Two buffers: an event stays readable for the frame it was sent in and the
/// whole next one, so a system that runs earlier in the frame than the sender
/// still sees it. Buffers are cleared, not freed, so a steady state allocates nothing.
#[derive(Debug)]
pub struct Events<T> {
    old: Vec<T>,
    new: Vec<T>,
    /// Id of `old[0]`; ids grow by one per sent event and never repeat.
    old_start: u64,
}

impl<T> Default for Events<T> {
    fn default() -> Self {
        Self {
            old: Vec::new(),
            new: Vec::new(),
            old_start: 0,
        }
    }
}

impl<T> Events<T> {
    pub fn send(&mut self, event: T) {
        self.new.push(event);
    }

    pub fn send_batch(&mut self, events: impl IntoIterator<Item = T>) {
        self.new.extend(events);
    }

    /// Events this cursor has not seen yet, oldest first. Moves the cursor to the end.
    pub fn read<'a>(&'a self, cursor: &mut EventCursor) -> impl Iterator<Item = &'a T> + 'a {
        let new_start = self.old_start + self.old.len() as u64;
        let end = new_start + self.new.len() as u64;
        let from = cursor.next.max(self.old_start);
        cursor.next = end;
        let old_skip = (from - self.old_start).min(self.old.len() as u64) as usize;
        let new_skip = from.saturating_sub(new_start) as usize;
        self.old[old_skip..].iter().chain(&self.new[new_skip..])
    }

    /// Number of events still stored (both buffers).
    pub fn len(&self) -> usize {
        self.old.len() + self.new.len()
    }

    pub fn is_empty(&self) -> bool {
        self.old.is_empty() && self.new.is_empty()
    }

    /// Drops everything right now. Cursors stay valid and just see nothing old.
    pub fn clear(&mut self) {
        self.old_start += (self.old.len() + self.new.len()) as u64;
        self.old.clear();
        self.new.clear();
    }

    /// Frame boundary: events older than one full frame are dropped.
    pub fn update(&mut self) {
        self.old_start += self.old.len() as u64;
        std::mem::swap(&mut self.old, &mut self.new);
        self.new.clear();
    }
}

pub(crate) fn update_events<T: 'static>(res: &mut Resources) {
    res.fetch_mut::<Events<T>>().update();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, Plugin, Stage};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn collect(ev: &Events<u32>, c: &mut EventCursor) -> Vec<u32> {
        ev.read(c).copied().collect()
    }

    #[test]
    fn cursors_read_independently_and_once() {
        let mut ev = Events::<u32>::default();
        let (mut a, mut b) = (EventCursor::default(), EventCursor::default());
        ev.send(1);
        ev.send_batch([2, 3]);
        assert_eq!(collect(&ev, &mut a), [1, 2, 3]);
        assert!(collect(&ev, &mut a).is_empty());
        ev.send(4);
        assert_eq!(collect(&ev, &mut a), [4]);
        assert_eq!(collect(&ev, &mut b), [1, 2, 3, 4]);
    }

    #[test]
    fn events_live_two_updates() {
        let mut ev = Events::<u32>::default();
        let mut late = EventCursor::default();
        let mut early = EventCursor::default();
        ev.send(1);
        ev.update();
        ev.send(2);
        assert_eq!(collect(&ev, &mut early), [1, 2]);
        ev.update();
        // 1 expired, 2 still lives.
        assert_eq!(collect(&ev, &mut late), [2]);
        assert!(collect(&ev, &mut early).is_empty());
        ev.update();
        assert!(ev.is_empty());
    }

    #[test]
    fn clear_keeps_cursors_valid() {
        let mut ev = Events::<u32>::default();
        let mut c = EventCursor::default();
        ev.send(1);
        ev.clear();
        assert!(collect(&ev, &mut c).is_empty());
        ev.send(2);
        assert_eq!(collect(&ev, &mut c), [2]);
    }

    #[test]
    fn steady_state_does_not_reallocate() {
        let mut ev = Events::<u32>::default();
        for _ in 0..4 {
            ev.send_batch(0..64);
            ev.update();
        }
        let caps = (ev.old.capacity(), ev.new.capacity());
        for _ in 0..100 {
            ev.send_batch(0..64);
            ev.update();
        }
        assert_eq!((ev.old.capacity(), ev.new.capacity()), caps);
    }

    struct Pinger;
    impl Plugin for Pinger {
        fn build(&self, app: &mut App) {
            app.add_event::<u32>().add_system(Stage::Update, |res| {
                let frame = res.fetch::<crate::Time>().frame() as u32;
                res.fetch_mut::<Events<u32>>().send(frame);
            });
        }
    }

    #[test]
    fn app_swaps_per_frame_and_add_event_is_idempotent() {
        let mut app = App::new();
        app.add_plugin(Pinger).add_event::<u32>();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s = seen.clone();
        let mut cursor = EventCursor::default();
        // Reader runs before the sender in the frame and must still see every event once.
        app.add_system(Stage::PreUpdate, move |res| {
            s.borrow_mut()
                .extend(res.fetch::<Events<u32>>().read(&mut cursor).copied());
        });
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(*seen.borrow(), [0, 1, 2]);
    }
}
