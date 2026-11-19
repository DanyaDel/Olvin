//! Game states with transitions and enter/exit hooks.

use std::fmt::Debug;
use std::hash::Hash;

use olvin_core::{App, Resources, Stage};

/// A game state: usually a plain enum (`Menu`, `Playing`, `Paused`).
pub trait GameState: Copy + Eq + Hash + Debug + 'static {}
impl<T: Copy + Eq + Hash + Debug + 'static> GameState for T {}

/// The current state of type `S` and a requested transition. Requests are applied
/// once per frame at the start of `Stage::PreUpdate`, so the whole frame sees one state.
pub struct State<S: GameState> {
    current: S,
    next: Option<S>,
}

impl<S: GameState> State<S> {
    pub fn get(&self) -> S {
        self.current
    }

    pub fn is(&self, state: S) -> bool {
        self.current == state
    }

    /// Requests a transition. The last request before the frame's apply wins.
    pub fn set(&mut self, state: S) {
        self.next = Some(state);
    }

    pub fn pending(&self) -> Option<S> {
        self.next
    }
}

/// Sent through `Events<StateChanged<S>>` after a transition, once the hooks ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateChanged<S> {
    pub from: S,
    pub to: S,
}

type Hook = Box<dyn FnMut(&mut Resources)>;

struct Hooks<S: GameState> {
    enter: Vec<(S, Hook)>,
    exit: Vec<(S, Hook)>,
}

/// A transition requested inside a hook can chain, but not forever.
const MAX_CHAIN: usize = 8;

fn run_hooks<S: GameState>(res: &mut Resources, exit: bool, state: S) {
    // Хуки лежат в ресурсе, а сами берут `&mut Resources`: на время вызова вынимаем их.
    let Some(mut hooks) = res.remove::<Hooks<S>>() else {
        return;
    };
    for (s, hook) in if exit {
        hooks.exit.iter_mut()
    } else {
        hooks.enter.iter_mut()
    } {
        if *s == state {
            hook(res);
        }
    }
    res.insert(hooks);
}

fn apply<S: GameState>(res: &mut Resources) {
    for _ in 0..MAX_CHAIN {
        let step = {
            let mut state = res.fetch_mut::<State<S>>();
            match state.next.take() {
                Some(to) if to != state.current => Some((state.current, to)),
                _ => None,
            }
        };
        let Some((from, to)) = step else { return };
        log::debug!("state: {from:?} -> {to:?}");
        run_hooks::<S>(res, true, from);
        res.fetch_mut::<State<S>>().current = to;
        run_hooks::<S>(res, false, to);
        if let Some(mut events) = res.get_mut::<olvin_core::Events<StateChanged<S>>>() {
            events.send(StateChanged { from, to });
        }
    }
    log::warn!(
        "state {}: transition chain cut off",
        std::any::type_name::<S>()
    );
}

/// State support for [`App`].
pub trait StateAppExt {
    /// Adds the state machine for `S` starting in `initial`. `on_enter(initial)` hooks
    /// run in `Stage::Startup`. Calling it twice for the same `S` is a no-op.
    fn add_state<S: GameState>(&mut self, initial: S) -> &mut Self;
    /// Runs `hook` each time `state` is entered.
    fn on_enter<S: GameState>(
        &mut self,
        state: S,
        hook: impl FnMut(&mut Resources) + 'static,
    ) -> &mut Self;
    /// Runs `hook` each time `state` is left.
    fn on_exit<S: GameState>(
        &mut self,
        state: S,
        hook: impl FnMut(&mut Resources) + 'static,
    ) -> &mut Self;
    /// A system that runs only while the state is `state`.
    fn add_system_in_state<S: GameState>(
        &mut self,
        stage: Stage,
        state: S,
        system: impl FnMut(&mut Resources) + 'static,
    ) -> &mut Self;
}

impl StateAppExt for App {
    fn add_state<S: GameState>(&mut self, initial: S) -> &mut Self {
        if self.resources.contains::<State<S>>() {
            return self;
        }
        self.insert_resource(State {
            current: initial,
            next: None,
        });
        self.insert_resource(Hooks::<S> {
            enter: Vec::new(),
            exit: Vec::new(),
        });
        self.add_event::<StateChanged<S>>();
        self.add_system(Stage::Startup, move |res| {
            run_hooks::<S>(res, false, initial)
        });
        self.add_system(Stage::PreUpdate, apply::<S>);
        self
    }

    fn on_enter<S: GameState>(
        &mut self,
        state: S,
        hook: impl FnMut(&mut Resources) + 'static,
    ) -> &mut Self {
        self.resources
            .fetch_mut::<Hooks<S>>()
            .enter
            .push((state, Box::new(hook)));
        self
    }

    fn on_exit<S: GameState>(
        &mut self,
        state: S,
        hook: impl FnMut(&mut Resources) + 'static,
    ) -> &mut Self {
        self.resources
            .fetch_mut::<Hooks<S>>()
            .exit
            .push((state, Box::new(hook)));
        self
    }

    fn add_system_in_state<S: GameState>(
        &mut self,
        stage: Stage,
        state: S,
        mut system: impl FnMut(&mut Resources) + 'static,
    ) -> &mut Self {
        self.add_system(stage, move |res| {
            if res.fetch::<State<S>>().is(state) {
                system(res);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_core::{EventCursor, Events, Time};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum Mode {
        Menu,
        Playing,
        Paused,
    }

    type Log = Rc<RefCell<Vec<&'static str>>>;

    fn app_with_log() -> (App, Log) {
        let log: Log = Rc::default();
        let mut app = App::new();
        app.add_state(Mode::Menu);
        for (mode, enter, exit) in [
            (Mode::Menu, "enter menu", "exit menu"),
            (Mode::Playing, "enter playing", "exit playing"),
            (Mode::Paused, "enter paused", "exit paused"),
        ] {
            let (a, b) = (log.clone(), log.clone());
            app.on_enter(mode, move |_| a.borrow_mut().push(enter));
            app.on_exit(mode, move |_| b.borrow_mut().push(exit));
        }
        (app, log)
    }

    fn set(app: &App, mode: Mode) {
        app.resources.fetch_mut::<State<Mode>>().set(mode);
    }

    #[test]
    fn initial_state_enters_on_startup_and_transitions_run_hooks_in_order() {
        let (mut app, log) = app_with_log();
        app.update();
        assert_eq!(*log.borrow(), ["enter menu"]);
        assert!(app.resources.fetch::<State<Mode>>().is(Mode::Menu));

        set(&app, Mode::Playing);
        app.update();
        set(&app, Mode::Paused);
        app.update();
        set(&app, Mode::Playing);
        app.update();
        assert_eq!(
            *log.borrow(),
            [
                "enter menu",
                "exit menu",
                "enter playing",
                "exit playing",
                "enter paused",
                "exit paused",
                "enter playing"
            ]
        );
    }

    #[test]
    fn setting_the_same_state_does_nothing_and_last_request_wins() {
        let (mut app, log) = app_with_log();
        app.update();
        set(&app, Mode::Menu);
        app.update();
        assert_eq!(log.borrow().len(), 1);

        set(&app, Mode::Paused);
        set(&app, Mode::Playing);
        app.update();
        assert!(app.resources.fetch::<State<Mode>>().is(Mode::Playing));
        assert!(!log.borrow().contains(&"enter paused"));
    }

    #[test]
    fn systems_run_only_in_their_state_and_see_the_change_the_same_frame() {
        let mut app = App::new();
        app.add_state(Mode::Menu);
        let ticks = Rc::new(RefCell::new(0));
        let t = ticks.clone();
        app.add_system_in_state(Stage::Update, Mode::Playing, move |_| *t.borrow_mut() += 1);
        app.update();
        app.update();
        assert_eq!(*ticks.borrow(), 0);
        set(&app, Mode::Playing);
        app.update();
        app.update();
        assert_eq!(*ticks.borrow(), 2);
    }

    #[test]
    fn hooks_can_use_resources_and_chain_transitions() {
        let mut app = App::new();
        app.add_state(Mode::Menu);
        // Пауза замораживает время, выход из паузы возвращает его.
        app.on_enter(Mode::Paused, |res| res.fetch_mut::<Time>().pause());
        app.on_exit(Mode::Paused, |res| res.fetch_mut::<Time>().resume());
        // Загрузочный промежуточный стейт сразу переходит дальше.
        app.on_enter(Mode::Playing, |res| {
            res.fetch_mut::<State<Mode>>().set(Mode::Paused)
        });
        app.update();
        set(&app, Mode::Playing);
        app.update();
        assert!(app.resources.fetch::<State<Mode>>().is(Mode::Paused));
        assert!(app.resources.fetch::<Time>().paused());
        set(&app, Mode::Menu);
        app.update();
        assert!(!app.resources.fetch::<Time>().paused());
    }

    #[test]
    fn endless_chain_is_cut_off() {
        let mut app = App::new();
        app.add_state(Mode::Menu);
        app.on_enter(Mode::Playing, |res| {
            res.fetch_mut::<State<Mode>>().set(Mode::Paused)
        });
        app.on_enter(Mode::Paused, |res| {
            res.fetch_mut::<State<Mode>>().set(Mode::Playing)
        });
        app.update();
        set(&app, Mode::Playing);
        app.update(); // не зависает
    }

    #[test]
    fn state_changed_events_and_duplicate_add_state() {
        let mut app = App::new();
        app.add_state(Mode::Menu).add_state(Mode::Playing);
        assert!(app.resources.fetch::<State<Mode>>().is(Mode::Menu));
        app.update();
        set(&app, Mode::Playing);
        app.update();
        let mut cursor = EventCursor::default();
        let events = app.resources.fetch::<Events<StateChanged<Mode>>>();
        let got: Vec<_> = events.read(&mut cursor).copied().collect();
        assert_eq!(
            got,
            [StateChanged {
                from: Mode::Menu,
                to: Mode::Playing
            }]
        );
    }
}
