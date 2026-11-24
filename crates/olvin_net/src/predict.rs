//! L4: client-side prediction and server reconciliation for the entity a
//! client owns (a character).
//!
//! Every net tick the client samples an input, applies it to its own entity at
//! once with the same `step` function the server uses, and sends the last few
//! inputs (tick-stamped, redundant against loss). The server applies exactly one
//! input per tick, and replicates the entity's state together with "the last input
//! I applied". When that state differs from what the client predicted for that
//! input by more than a threshold, the client moves towards the server's state
//! and re-applies the inputs the server has not seen yet. Small errors are
//! corrected a fraction at a time (no visible snap); big ones snap.
//!
//! One prediction spec per game (`net.predict`). The state `S` is an ordinary
//! replicated component that the owner gets for reconciliation instead of display.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use olvin_core::Resources;
use olvin_ecs::{Component, Entity, World};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::net::{ClientId, Net};
use crate::proto::Msg;
use crate::replicate::{Owner, ReplicateOptions, codec};
use crate::transport::Channel;
use crate::wire;

/// Server side: the last input of the owner that was applied to this entity.
/// Replicated inside the predicted state, never on its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputAck(pub u32);

/// On the client's entity while it is being predicted.
#[derive(Clone, Copy, Debug, Default)]
pub struct Predicted;

/// How to predict state `S` from inputs `I`.
pub struct Prediction<S, I> {
    /// Reads the player's input for one tick (from `ActionState`, usually).
    pub sample: fn(&Resources) -> I,
    /// Advances `entity` by one tick of `input`. Must behave the same on server
    /// and client: it is the whole simulation of the predicted entity. It must not
    /// touch the `Net` resource.
    pub step: fn(&Resources, Entity, &I, f32),
    /// How far apart two states are (metres, for a character).
    pub distance: fn(&S, &S) -> f32,
    /// `blend(a, b, t)`: `a` at 0, `b` at 1. Used to correct gradually.
    pub blend: fn(&S, &S, f32) -> S,
    /// Errors at or below this are ignored.
    pub threshold: f32,
    /// Errors above this snap to the server state at once.
    pub snap_distance: f32,
    /// Share of a small error removed per correction (0..1].
    pub correction_rate: f32,
    /// Server: cleans a client's input (clamp what a cheater could inflate).
    pub sanitize: Option<fn(&mut I)>,
    /// Client: after the state was rewritten (re-sync a physics body, say).
    pub on_correct: Option<fn(&Resources, Entity)>,
}

impl<S, I> Prediction<S, I> {
    pub fn new(
        sample: fn(&Resources) -> I,
        step: fn(&Resources, Entity, &I, f32),
        distance: fn(&S, &S) -> f32,
        blend: fn(&S, &S, f32) -> S,
    ) -> Self {
        Self {
            sample,
            step,
            distance,
            blend,
            threshold: 0.01,
            snap_distance: 2.0,
            correction_rate: 0.35,
            sanitize: None,
            on_correct: None,
        }
    }

    pub fn sanitize(mut self, f: fn(&mut I)) -> Self {
        self.sanitize = Some(f);
        self
    }

    pub fn on_correct(mut self, f: fn(&Resources, Entity)) -> Self {
        self.on_correct = Some(f);
        self
    }

    pub fn thresholds(mut self, ignore: f32, snap: f32) -> Self {
        self.threshold = ignore;
        self.snap_distance = snap;
        self
    }

    pub fn correction_rate(mut self, rate: f32) -> Self {
        self.correction_rate = rate.clamp(0.01, 1.0);
        self
    }
}

/// Counters for tests and the stats panel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PredictionStats {
    pub inputs_sent: u64,
    pub corrections: u64,
    pub snaps: u64,
    /// Error of the last comparison.
    pub last_error: f32,
}

/// Inputs a server keeps per client; more than this are dropped, oldest first.
const MAX_QUEUE: usize = 64;
const MAX_HISTORY: usize = 128;
const REDUNDANCY: usize = 8;
/// A server catches up by one extra input per tick above this backlog.
const CATCH_UP: usize = 4;

pub(crate) trait PredictionDriver {
    fn state_name(&self) -> String;
    fn input_name(&self) -> String;
    fn stats(&self) -> PredictionStats;
    /// Server: queue inputs from a client (decoded, sanitised, bounded).
    fn push_input(&self, client: ClientId, first_seq: u32, inputs: &[Vec<u8>], max_len: usize);
    /// Server: applies queued inputs, one per tick.
    fn server_step(&self, res: &Resources, dt: f32);
    fn forget_client(&self, client: ClientId);
    /// Client: samples and predicts one tick; the packet to send.
    /// `local`: this process is the server, so the input goes straight to its own queue.
    fn client_tick(
        &self,
        res: &Resources,
        me: ClientId,
        dt: f32,
        local: bool,
    ) -> Option<(u32, Vec<Vec<u8>>)>;
    /// Client: the server's state for the entity we predict.
    fn on_server_state(&self, res: &Resources, entity: Entity, data: &[u8]);
}

struct Hist<S, I> {
    seq: u32,
    input: I,
    after: S,
}

struct State<S, I> {
    queues: HashMap<ClientId, VecDeque<(u32, I)>>,
    last_queued: HashMap<ClientId, u32>,
    /// Extra steps a client may still take to drain a backlog (a speed hack gets almost none).
    catch_up: HashMap<ClientId, f32>,
    hist: VecDeque<Hist<S, I>>,
    next_seq: u32,
    unacked: VecDeque<(u32, Vec<u8>)>,
    stats: PredictionStats,
}

struct Driver<S, I> {
    spec: Prediction<S, I>,
    st: RefCell<State<S, I>>,
}

fn owned_entity<S: Component>(world: &World, client: ClientId) -> Option<Entity> {
    world
        .query::<(Entity, &Owner)>()
        .with::<&S>()
        .iter()
        .find(|(_, o)| o.0 == client)
        .map(|(e, _)| e)
}

impl<S, I> PredictionDriver for Driver<S, I>
where
    S: Component + Clone + Serialize + DeserializeOwned,
    I: Clone + Serialize + DeserializeOwned + 'static,
{
    fn state_name(&self) -> String {
        crate::replicate::short_type_name::<S>()
    }

    fn input_name(&self) -> String {
        crate::replicate::short_type_name::<I>()
    }

    fn stats(&self) -> PredictionStats {
        self.st.borrow().stats
    }

    fn push_input(&self, client: ClientId, first_seq: u32, inputs: &[Vec<u8>], max_len: usize) {
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        for (i, bytes) in inputs.iter().take(REDUNDANCY * 2).enumerate() {
            let seq = first_seq.wrapping_add(i as u32);
            let last = st.last_queued.get(&client).copied().unwrap_or(0);
            if seq <= last {
                continue;
            }
            let Ok(mut input) = wire::decode::<I>(bytes, max_len) else {
                continue;
            };
            if let Some(clean) = self.spec.sanitize {
                clean(&mut input);
            }
            let q = st.queues.entry(client).or_default();
            if q.len() >= MAX_QUEUE {
                q.pop_front();
            }
            q.push_back((seq, input));
            st.last_queued.insert(client, seq);
        }
    }

    fn server_step(&self, res: &Resources, dt: f32) {
        let clients: Vec<ClientId> = self.st.borrow().queues.keys().copied().collect();
        for client in clients {
            let Some(entity) = owned_entity::<S>(&res.fetch::<World>(), client) else {
                continue;
            };
            let mut steps = 1;
            {
                let mut st = self.st.borrow_mut();
                let c = st.catch_up.entry(client).or_insert(0.0);
                *c = (*c + 0.1).min(3.0);
            }
            while steps > 0 {
                steps -= 1;
                let next = {
                    let mut st = self.st.borrow_mut();
                    let backlog = st.queues.get(&client).map_or(0, |q| q.len());
                    let credit = st.catch_up.entry(client).or_insert(0.0);
                    if backlog > CATCH_UP && *credit >= 1.0 {
                        *credit -= 1.0;
                        steps += 1;
                    }
                    st.queues.entry(client).or_default().pop_front()
                };
                let Some((seq, input)) = next else {
                    break;
                };
                (self.spec.step)(res, entity, &input, dt);
                let _ = res.fetch_mut::<World>().insert_one(entity, InputAck(seq));
            }
        }
    }

    fn forget_client(&self, client: ClientId) {
        let mut st = self.st.borrow_mut();
        st.queues.remove(&client);
        st.last_queued.remove(&client);
        st.catch_up.remove(&client);
    }

    fn client_tick(
        &self,
        res: &Resources,
        me: ClientId,
        dt: f32,
        local: bool,
    ) -> Option<(u32, Vec<Vec<u8>>)> {
        let entity = owned_entity::<S>(&res.fetch::<World>(), me)?;
        let input = (self.spec.sample)(res);
        let bytes = wire::encode(&input).ok()?;
        let seq = {
            let mut st = self.st.borrow_mut();
            st.next_seq += 1;
            st.next_seq
        };
        if local {
            // The server is us: it applies the input in its own tick.
            self.push_input(me, seq, std::slice::from_ref(&bytes), usize::MAX);
            return None;
        }
        let _ = res.fetch_mut::<World>().insert_one(entity, Predicted);
        (self.spec.step)(res, entity, &input, dt);
        let after: S = S::clone(&*res.fetch::<World>().get::<&S>(entity).ok()?);
        let mut st = self.st.borrow_mut();
        st.hist.push_back(Hist { seq, input, after });
        while st.hist.len() > MAX_HISTORY {
            st.hist.pop_front();
        }
        st.unacked.push_back((seq, bytes));
        while st.unacked.len() > MAX_HISTORY {
            st.unacked.pop_front();
        }
        let skip = st.unacked.len().saturating_sub(REDUNDANCY);
        let inputs: Vec<Vec<u8>> = st
            .unacked
            .iter()
            .skip(skip)
            .map(|(_, b)| b.clone())
            .collect();
        let first = st.unacked.get(skip).map_or(seq, |(s, _)| *s);
        st.stats.inputs_sent += 1;
        Some((first, inputs))
    }

    fn on_server_state(&self, res: &Resources, entity: Entity, data: &[u8]) {
        let mut rest = data;
        let Ok(ack) = codec::get_varint(&mut rest) else {
            return;
        };
        let ack = ack as u32;
        let Ok((server, _)) = postcard::take_from_bytes::<S>(rest) else {
            return;
        };
        if ack == 0 {
            return;
        }
        let (predicted, input_tail) = {
            let mut st = self.st.borrow_mut();
            st.unacked.retain(|(s, _)| *s > ack);
            let Some(pos) = st.hist.iter().position(|h| h.seq == ack) else {
                // Older than what we remember, or ahead of us: nothing to compare.
                st.hist.retain(|h| h.seq > ack);
                return;
            };
            let predicted = st.hist[pos].after.clone();
            let tail: Vec<(u32, I)> = st
                .hist
                .iter()
                .skip(pos + 1)
                .map(|h| (h.seq, h.input.clone()))
                .collect();
            st.hist.retain(|h| h.seq > ack);
            (predicted, tail)
        };
        let err = (self.spec.distance)(&predicted, &server);
        self.st.borrow_mut().stats.last_error = err;
        if err <= self.spec.threshold {
            return;
        }
        let snap = err > self.spec.snap_distance;
        let start = if snap {
            server
        } else {
            (self.spec.blend)(&predicted, &server, self.spec.correction_rate)
        };
        let _ = res.fetch_mut::<World>().insert_one(entity, start);
        if let Some(f) = self.spec.on_correct {
            f(res, entity);
        }
        let dt = res_tick_dt(res);
        let mut replayed = Vec::with_capacity(input_tail.len());
        for (seq, input) in input_tail {
            (self.spec.step)(res, entity, &input, dt);
            if let Ok(s) = res.fetch::<World>().get::<&S>(entity) {
                replayed.push((seq, S::clone(&*s)));
            }
        }
        let mut st = self.st.borrow_mut();
        for (seq, after) in replayed {
            if let Some(h) = st.hist.iter_mut().find(|h| h.seq == seq) {
                h.after = after;
            }
        }
        if snap {
            st.stats.snaps += 1;
        } else {
            st.stats.corrections += 1;
        }
    }
}

fn res_tick_dt(res: &Resources) -> f32 {
    res.get::<Net>()
        .map_or(1.0 / 30.0, |n| 1.0 / n.cfg.tick_rate.max(1) as f32)
}

impl Net {
    /// Turns on prediction for the entity a client owns: `S` is its replicated
    /// state, `I` its input. Call once, before the first frame, after
    /// registering nothing else that uses `S`.
    pub fn predict<S, I>(&mut self, spec: Prediction<S, I>) -> &mut Self
    where
        S: Component + Clone + Serialize + DeserializeOwned,
        I: Clone + Serialize + DeserializeOwned + 'static,
    {
        assert!(
            self.prediction.is_none(),
            "net: only one prediction spec per game"
        );
        let driver = Rc::new(Driver::<S, I> {
            spec,
            st: RefCell::new(State {
                queues: HashMap::new(),
                last_queued: HashMap::new(),
                catch_up: HashMap::new(),
                hist: VecDeque::new(),
                next_seq: 0,
                unacked: VecDeque::new(),
                stats: PredictionStats::default(),
            }),
        });
        self.registry.add_name(
            "prediction",
            &format!("{}/{}", driver.state_name(), driver.input_name()),
        );
        let opts = ReplicateOptions::<S> {
            ack_prefix: true,
            ..Default::default()
        };
        self.predicted_comp = Some(self.registry.components.len());
        self.replicate_with(opts);
        self.prediction = Some(driver);
        self
    }

    /// The owning client simulates `T` itself (the prediction moves it), so
    /// snapshots no longer write it on entities that client owns. Other clients
    /// still see it replicated and interpolated.
    pub fn owner_simulated<T: Component>(&mut self) -> &mut Self {
        let id = std::any::TypeId::of::<T>();
        match self
            .registry
            .components
            .iter()
            .position(|c| c.type_id() == id)
        {
            Some(i) => self.owner_local.push(i),
            None => log::error!(
                "net: owner_simulated::<{}>: replicate it first",
                std::any::type_name::<T>()
            ),
        }
        self
    }

    /// Counters of the prediction (zeros if there is none).
    pub fn prediction_stats(&self) -> PredictionStats {
        self.prediction
            .as_ref()
            .map(|d| d.stats())
            .unwrap_or_default()
    }

    /// Server: a client's inputs arrived.
    pub(crate) fn on_input(&mut self, client: ClientId, first_seq: u32, inputs: Vec<Vec<u8>>) {
        let ok = inputs.len() <= REDUNDANCY * 2;
        let Some(driver) = self.prediction.clone() else {
            self.violation(client, 5.0, "input without prediction");
            return;
        };
        if !ok {
            self.violation(client, 5.0, "too many inputs in one packet");
            return;
        }
        // Input packets share the client's overall message budget.
        if !self.allow_message(client) {
            return;
        }
        driver.push_input(client, first_seq, &inputs, self.cfg.limits.max_message_size);
    }

    /// Plans this frame's work: server ticks due, client ticks due, and the driver.
    pub(crate) fn plan_ticks(&mut self, dt: f32) -> TickPlan {
        let step = 1.0 / self.cfg.tick_rate.max(1) as f32;
        let mut plan = TickPlan {
            server_ticks: 0,
            client_ticks: 0,
            dt: step,
            driver: self.prediction.clone(),
            me: self.local_client(),
        };
        if let Some(s) = self.server.as_mut() {
            s.tick_accum = (s.tick_accum + dt.max(0.0)).min(step * 5.0);
            while s.tick_accum >= step {
                s.tick_accum -= step;
                plan.server_ticks += 1;
            }
        } else if self
            .client
            .as_ref()
            .is_some_and(|c| c.phase == crate::client::Phase::Connected)
        {
            self.client_accum = (self.client_accum + dt.max(0.0)).min(step * 5.0);
            while self.client_accum >= step {
                self.client_accum -= step;
                plan.client_ticks += 1;
            }
        }
        plan
    }

    /// Client: sends predicted inputs.
    pub(crate) fn send_input(&mut self, first_seq: u32, inputs: Vec<Vec<u8>>) {
        if let Some(c) = self.client.as_mut() {
            c.send(Channel::Unreliable, &Msg::Input { first_seq, inputs });
        }
    }
}

pub(crate) struct TickPlan {
    pub server_ticks: u32,
    pub client_ticks: u32,
    pub dt: f32,
    pub driver: Option<Rc<dyn PredictionDriver>>,
    pub me: Option<ClientId>,
}

/// Runs one frame's network ticks. Called from the PostUpdate system with nothing borrowed.
pub(crate) fn run_ticks(res: &Resources, plan: TickPlan) {
    for _ in 0..plan.server_ticks {
        if let (Some(d), Some(me)) = (&plan.driver, plan.me) {
            // A host or offline player: its input joins the queue like anybody's.
            let _ = d.client_tick(res, me, plan.dt, true);
        }
        if let Some(d) = &plan.driver {
            d.server_step(res, plan.dt);
        }
        let mut net = res.fetch_mut::<Net>();
        let mut world = res.fetch_mut::<World>();
        net.replicate_tick(&mut world);
    }
    for _ in 0..plan.client_ticks {
        if let (Some(d), Some(me)) = (&plan.driver, plan.me)
            && let Some((first, inputs)) = d.client_tick(res, me, plan.dt, false)
        {
            res.fetch_mut::<Net>().send_input(first, inputs);
        }
    }
}

/// Client: hands the server's states for the predicted entity to the driver.
pub(crate) fn reconcile(res: &Resources) {
    let (driver, pending) = {
        let mut net = res.fetch_mut::<Net>();
        (
            net.prediction.clone(),
            std::mem::take(&mut net.prediction_inbox),
        )
    };
    if let Some(d) = driver {
        for (entity, data, _tick) in pending {
            d.on_server_state(res, entity, &data);
        }
    }
}
