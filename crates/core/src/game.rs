//! A reign without UI: action -> tick -> event -> choice.

use crate::data::Data;
use crate::fx::Fx;
use crate::rng::Rng;
use crate::rules::{ActionTarget, Choice, Ctx, Event, EventTarget, Target, add_axis};
use crate::state::{ActiveAction, Holder, NeighbourId, Preset, ProvinceId, World};
use crate::time::Tick;
use serde::{Deserialize, Serialize};

pub type ActionId = String;

/// A player decision, enough to replay the game from seed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Decision {
    pub tick: Tick,
    pub kind: DecisionKind,
    pub cause_tag: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum DecisionKind {
    EventChoice {
        event_id: String,
        choice_idx: usize,
        target: Option<Target>,
    },
    ActionStarted {
        action_id: ActionId,
        target: Option<Target>,
    },
}

/// The event waiting for `choose`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PendingEvent {
    pub event_id: String,
    pub target: Option<Target>,
}

/// An event with `{province}`, `{neighbour}`, `{ruler}` filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct EventView {
    pub event_id: String,
    pub title: String,
    pub text: String,
    pub importance: u32,
    pub target: Option<Target>,
    pub choices: Vec<Choice>,
}

// One Step per tick, and the World copy in ReignEnd is the point of that variant.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Event(EventView),
    Idle,
    /// Returned from stage 5 on.
    ReignEnded(ReignEnd),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReignEnd {
    pub cause: String,
    pub tick: Tick,
    pub world: World,
}

/// Errors of `start_action`, `wait` and `choose`.
#[derive(Debug, PartialEq)]
pub enum GameError {
    /// No such action.
    Unknown,
    /// Not in `available_actions` with this target.
    Unavailable,
    NoSlot,
    /// An event waits for `choose`.
    EventPending,
    /// `choose` without a pending event.
    NoEvent,
    BadChoice,
    /// The reign is over; returned from stage 5 on.
    ReignEnded,
}

#[derive(Clone, Debug)]
pub struct Game {
    pub world: World,
    pub rng: Rng,
    pub data: Data,
    pub decisions: Vec<Decision>,
    pub pending_event: Option<PendingEvent>,
    /// Deferred events from `SpawnEvent`: `(due tick, event id)`.
    pub queue: Vec<(Tick, String)>,
}

impl Game {
    /// `data` must already hold the events and actions.
    pub fn new(data: Data, preset: &Preset, seed: u64) -> Game {
        Game {
            world: World::from_preset(&data, preset),
            rng: Rng::from_seed(seed),
            data,
            decisions: Vec::new(),
            pending_event: None,
            queue: Vec::new(),
        }
    }

    /// Actions startable now with their free targets; an empty list means no target needed.
    /// Slots are not checked here.
    pub fn available_actions(&self) -> Vec<(ActionId, Vec<Target>)> {
        let w = &self.world;
        let treasury = w.axes[&self.data.economy.treasury];
        let capital = w.provinces.get(&w.capital.province);
        let capital_power = capital.map_or(Fx(0), |p| p.crown_power);
        let actions = self.data.actions.iter();
        let actions = actions.filter(|a| a.cost <= treasury && a.requires.eval(w));
        actions
            .filter_map(|a| {
                let targets: Vec<Option<Target>> = match &a.target {
                    ActionTarget::Province(f) => (w.provinces.values())
                        .filter(|p| f.matches(p, w) && p.crown_power >= a.min_crown_power)
                        .map(|p| Some(Target::Province(p.id.clone())))
                        .collect(),
                    _ if capital_power < a.min_crown_power => return None,
                    ActionTarget::None => vec![None],
                    ActionTarget::Neighbour => (w.neighbours.keys())
                        .map(|n| Some(Target::Neighbour(n.clone())))
                        .collect(),
                    ActionTarget::Heir => (0..w.heirs.len() as u32)
                        .map(|i| Some(Target::Heir(i)))
                        .collect(),
                };
                // The same action on the same target runs once at a time.
                let busy = |t: &Option<Target>| {
                    let key = t.as_ref().map(target_key);
                    (w.active_actions.iter()).any(|x| x.id == a.id && x.target == key)
                };
                let free: Vec<_> = targets.into_iter().filter(|t| !busy(t)).collect();
                (!free.is_empty()).then(|| (a.id.clone(), free.into_iter().flatten().collect()))
            })
            .collect()
    }

    pub fn start_action(&mut self, id: &str, target: Option<Target>) -> Result<(), GameError> {
        if self.pending_event.is_some() {
            return Err(GameError::EventPending);
        }
        let Some(action) = self.data.actions.iter().find(|a| a.id == id) else {
            return Err(GameError::Unknown);
        };
        let available = self.available_actions().into_iter().any(|(a, targets)| {
            a == id
                && match &target {
                    None => targets.is_empty(),
                    Some(t) => targets.contains(t),
                }
        });
        if !available {
            return Err(GameError::Unavailable);
        }
        let w = &mut self.world;
        if w.active_actions.len() as u32 >= self.data.action_slots.slots(w) {
            return Err(GameError::NoSlot);
        }
        add_axis(
            w,
            &self.data,
            &self.data.economy.treasury,
            Fx(0) - action.cost,
        );
        w.active_actions.push(ActiveAction {
            id: id.into(),
            target: target.as_ref().map(target_key),
            ends_at: Tick(w.tick.0 + action.duration_years.ticks(w.time_unit).0),
        });
        self.decisions.push(Decision {
            tick: w.tick,
            kind: DecisionKind::ActionStarted {
                action_id: id.into(),
                target,
            },
            cause_tag: action.cause_tag.clone(),
        });
        Ok(())
    }

    /// Advances one tick, or repeats the pending event without advancing.
    pub fn wait(&mut self) -> Result<Step, GameError> {
        if let Some(p) = &self.pending_event {
            return Ok(Step::Event(self.view(p)));
        }
        self.world.tick.0 += 1;
        self.passive();
        self.complete_actions();
        self.world.recompute_loyalty(&self.data);
        self.world.recompute_crown_power(&self.data);
        let picked = pick_event(&self.data, &self.world, &mut self.rng, &mut self.queue);
        let Some(p) = picked else {
            return Ok(Step::Idle);
        };
        self.world
            .last_fired
            .insert(p.event_id.clone(), self.world.tick);
        let view = self.view(&p);
        self.pending_event = Some(p);
        Ok(Step::Event(view))
    }

    pub fn choose(&mut self, idx: usize) -> Result<(), GameError> {
        let p = self.pending_event.as_ref().ok_or(GameError::NoEvent)?;
        let event = find_event(&self.data, &p.event_id).expect("pending events exist");
        let choice = event.choices.get(idx).ok_or(GameError::BadChoice)?;
        let mut ctx = Ctx {
            data: &self.data,
            queue: &mut self.queue,
            target: p.target.as_ref(),
        };
        for e in &choice.effects {
            e.apply(&mut self.world, &mut ctx);
        }
        self.world.recompute_loyalty(&self.data);
        self.world.recompute_crown_power(&self.data);
        self.decisions.push(Decision {
            tick: self.world.tick,
            kind: DecisionKind::EventChoice {
                event_id: p.event_id.clone(),
                choice_idx: idx,
                target: p.target.clone(),
            },
            cause_tag: choice.cause_tag.clone(),
        });
        self.pending_event = None;
        Ok(())
    }

    /// Treasury, aging, drift. Yearly amounts are spread over the ticks of a year.
    fn passive(&mut self) {
        let (d, w) = (&self.data, &mut self.world);
        let per_tick = |v: Fx| v / Fx::from_int(d.time_unit.ticks_per_year as i64);
        let crown = w.provinces.values().filter(|p| p.holder == Holder::Crown);
        let income = crown.fold(Fx(0), |sum, p| sum + p.income);
        let flows = d.economy.flows.iter();
        let income = flows.fold(income, |sum, (a, k)| sum + w.axes[a] * *k);
        add_axis(w, d, &d.economy.treasury, per_tick(income));

        if w.tick.0 % d.time_unit.ticks_per_year == 0 {
            w.ruler.age += 1;
            w.heirs.iter_mut().for_each(|h| h.age += 1);
        }

        let step = per_tick(d.drift.step);
        let toward = |v: Fx, base: Fx| match v < base {
            true => (v + step).min(base),
            false => (v - step).max(base),
        };
        for f in &d.factions {
            let def = d
                .axes
                .iter()
                .find(|a| a.id == f.axis)
                .expect("checked on load");
            let v = w.axes.get_mut(&f.axis).expect("the world has every axis");
            *v = toward(*v, def.default);
        }
        for p in w.provinces.values_mut() {
            p.loyalty = toward(p.loyalty, d.drift.province_loyalty);
        }
        for m in w.crown_modifiers.values_mut() {
            *m = toward(*m, Fx(0));
        }
        w.crown_modifiers.retain(|_, m| *m != Fx(0));
    }

    /// Applies `on_complete` of actions ending this tick, in start order.
    fn complete_actions(&mut self) {
        let tick = self.world.tick;
        let all = std::mem::take(&mut self.world.active_actions);
        let (done, running): (Vec<_>, Vec<_>) = all.into_iter().partition(|a| a.ends_at <= tick);
        self.world.active_actions = running;
        for a in done {
            let def = self.data.actions.iter().find(|x| x.id == a.id);
            let def = def.expect("only known actions start");
            let target = a.target.map(|key| match def.target {
                ActionTarget::Province(_) => Target::Province(ProvinceId(key)),
                ActionTarget::Neighbour => Target::Neighbour(NeighbourId(key)),
                ActionTarget::Heir => Target::Heir(key.parse().expect("written by target_key")),
                ActionTarget::None => unreachable!("untargeted actions store no target"),
            });
            let mut ctx = Ctx {
                data: &self.data,
                queue: &mut self.queue,
                target: target.as_ref(),
            };
            for e in &def.on_complete {
                e.apply(&mut self.world, &mut ctx);
            }
        }
    }

    fn view(&self, p: &PendingEvent) -> EventView {
        let e = find_event(&self.data, &p.event_id).expect("pending events exist");
        let w = &self.world;
        let name = match &p.target {
            Some(Target::Province(id)) => w.provinces.get(id).map(|x| ("{province}", &x.name)),
            Some(Target::Neighbour(id)) => w.neighbours.get(id).map(|x| ("{neighbour}", &x.name)),
            _ => None,
        };
        let fill = |s: &str| {
            let s = s.replace("{ruler}", &w.ruler.name);
            match name {
                Some((k, v)) => s.replace(k, v),
                None => s,
            }
        };
        let choices = e.choices.iter().map(|c| Choice {
            text: fill(&c.text),
            hint: c.hint.as_deref().map(fill),
            ..c.clone()
        });
        EventView {
            event_id: e.id.clone(),
            title: fill(&e.title),
            text: fill(&e.text),
            importance: e.importance,
            target: p.target.clone(),
            choices: choices.collect(),
        }
    }
}

/// `ActiveAction.target` is a string; the action's `ActionTarget` says how to read it back.
fn target_key(t: &Target) -> String {
    match t {
        Target::Province(id) => id.0.clone(),
        Target::Neighbour(id) => id.0.clone(),
        Target::Heir(i) => i.to_string(),
    }
}

fn find_event<'a>(data: &'a Data, id: &str) -> Option<&'a Event> {
    data.events.iter().find(|e| e.id == id)
}

/// `None`: the event needs no target. `Some(empty)`: it cannot fire now.
fn candidates(target: &EventTarget, w: &World) -> Option<Vec<Target>> {
    match target {
        EventTarget::None => None,
        EventTarget::RandomProvince(f) => Some(
            (w.provinces.values())
                .filter(|p| f.matches(p, w))
                .map(|p| Target::Province(p.id.clone()))
                .collect(),
        ),
        EventTarget::Neighbour => Some(
            w.neighbours
                .keys()
                .map(|n| Target::Neighbour(n.clone()))
                .collect(),
        ),
    }
}

/// At most one event per tick. Due deferred events go first, earliest due first; one whose
/// `when` is false, which already fired `once`, or has no target is dropped. Otherwise a
/// weighted pick over ready events off cooldown, with `quiet_weight` for no event.
fn pick_event(
    data: &Data,
    w: &World,
    rng: &mut Rng,
    queue: &mut Vec<(Tick, String)>,
) -> Option<PendingEvent> {
    let ready = |e: &Event| {
        let fired = w.last_fired.get(&e.id);
        let targets = candidates(&e.target, w);
        e.when.eval(w) && !(e.once && fired.is_some()) && targets.is_none_or(|t| !t.is_empty())
    };
    let fire = |e: &Event, rng: &mut Rng| {
        let pick = |t: Vec<Target>| t[rng.range(0, t.len() as i64) as usize].clone();
        let target = candidates(&e.target, w).map(pick);
        Some(PendingEvent {
            event_id: e.id.clone(),
            target,
        })
    };

    let due = |q: &Vec<(Tick, String)>| {
        (0..q.len())
            .filter(|&i| q[i].0 <= w.tick)
            .min_by_key(|&i| q[i].0)
    };
    while let Some(i) = due(queue) {
        let (_, id) = queue.remove(i);
        if let Some(e) = find_event(data, &id).filter(|e| ready(e)) {
            return fire(e, rng);
        }
    }

    let cooling = |e: &Event| {
        let cooldown = e.cooldown_years.ticks(w.time_unit).0;
        w.last_fired
            .get(&e.id)
            .is_some_and(|t| w.tick.0 < t.0 + cooldown)
    };
    let pool: Vec<&Event> = (data.events.iter())
        .filter(|e| e.weight > 0 && !cooling(e) && ready(e))
        .collect();
    let total: u32 = data.quiet_weight + pool.iter().map(|e| e.weight).sum::<u32>();
    if total == 0 {
        return None;
    }
    let mut roll = rng.range(0, total as i64) as u32;
    for e in pool {
        if roll < e.weight {
            return fire(e, rng);
        }
        roll -= e.weight;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{
        Action, Effect, Predicate, ProvinceField, ProvinceFilter, ProvinceTarget, Sign,
    };
    use crate::state::AxisId;
    use crate::time::{TimeUnit, Years};

    const RULES: &str = include_str!("../../../data/rules.ron");
    const PRESET: &str = include_str!("../../../data/presets/default.ron");

    fn ax(s: &str) -> AxisId {
        AxisId(s.into())
    }

    fn pid(s: &str) -> ProvinceId {
        ProvinceId(s.into())
    }

    fn game(data: Data, seed: u64) -> Game {
        let preset = Preset::load(PRESET, &data).unwrap();
        Game::new(data, &preset, seed)
    }

    /// No content and no quiet weight: every tick is idle unless a test says otherwise.
    fn bare() -> Data {
        let mut data = crate::data::load(RULES).unwrap();
        data.quiet_weight = 0;
        data
    }

    /// Out of the random pool (weight 0), always ready, one choice.
    fn event(id: &str, effects: Vec<Effect>) -> Event {
        Event {
            id: id.into(),
            title: id.into(),
            text: String::new(),
            when: Predicate::All(vec![]),
            weight: 0,
            once: false,
            cooldown_years: Years(0),
            importance: 1,
            sign: Sign::Bad,
            target: EventTarget::None,
            choices: vec![Choice {
                text: id.into(),
                effects,
                cause_tag: id.into(),
                hint: None,
            }],
        }
    }

    fn action(id: &str, target: ActionTarget, on_complete: Vec<Effect>) -> Action {
        Action {
            id: id.into(),
            name: id.into(),
            duration_years: Years(1),
            cost: Fx(0),
            requires: Predicate::All(vec![]),
            min_crown_power: Fx(0),
            target,
            on_complete,
            cause_tag: id.into(),
        }
    }

    fn fired(step: Step) -> Option<String> {
        match step {
            Step::Event(v) => Some(v.event_id),
            _ => None,
        }
    }

    /// Fires `id` now, as if the scheduler had picked it, and takes choice 0.
    fn force(g: &mut Game, id: &str) {
        g.pending_event = Some(PendingEvent {
            event_id: id.into(),
            target: None,
        });
        g.choose(0).unwrap();
    }

    fn play(seed: u64) -> Game {
        let mut data = crate::data::load(RULES).unwrap();
        data.add_events(include_str!("../../../data/events/test.ron"))
            .unwrap();
        data.add_actions(include_str!("../../../data/actions.ron"))
            .unwrap();
        let mut g = game(data, seed);
        for i in 0..20 {
            if let Some((id, targets)) = g.available_actions().into_iter().next() {
                let _ = g.start_action(&id, targets.first().cloned());
            }
            if let Step::Event(v) = g.wait().unwrap() {
                g.choose(i % v.choices.len()).unwrap();
            }
        }
        g
    }

    #[test]
    fn replay_is_deterministic() {
        let a = play(42);
        let b = play(42);
        assert_eq!(a.world, b.world);
        assert_eq!(a.decisions, b.decisions);
        assert_eq!(a.queue, b.queue);
        assert_eq!(a.world.tick, Tick(20));
        let kinds = |k: fn(&DecisionKind) -> bool| a.decisions.iter().any(|d| k(&d.kind));
        assert!(kinds(|k| matches!(k, DecisionKind::EventChoice { .. })));
        assert!(kinds(|k| matches!(k, DecisionKind::ActionStarted { .. })));
        assert_ne!(play(43).decisions, a.decisions);
    }

    #[test]
    fn min_crown_power_filters_actions() {
        // Crown power: capital 90, holm 25, nordheim 0.
        let mut data = bare();
        let mut tax = action(
            "tax",
            ActionTarget::Province(ProvinceFilter::default()),
            vec![],
        );
        tax.min_crown_power = Fx::from_int(50);
        let mut decree = action("decree", ActionTarget::None, vec![]);
        decree.min_crown_power = Fx::from_int(91);
        let mut edict = action("edict", ActionTarget::None, vec![]);
        edict.min_crown_power = Fx::from_int(90);
        data.actions = vec![tax, decree, edict];
        let mut g = game(data, 1);
        let capital = Target::Province(pid("capital"));
        assert_eq!(
            g.available_actions(),
            [("tax".into(), vec![capital]), ("edict".into(), vec![])]
        );
        let holm = Some(Target::Province(pid("holm")));
        assert_eq!(g.start_action("tax", holm), Err(GameError::Unavailable));
        assert_eq!(g.start_action("decree", None), Err(GameError::Unavailable));
    }

    #[test]
    fn action_slots_follow_bureaucracy() {
        let mut data = bare();
        data.action_slots.steps = vec![(Fx(0), 1), (Fx::from_int(40), 2)];
        data.actions = vec![
            action("a", ActionTarget::None, vec![]),
            action("b", ActionTarget::None, vec![]),
        ];
        // Bureaucracy 20: one slot.
        let mut g = game(data.clone(), 1);
        assert_eq!(g.start_action("a", None), Ok(()));
        assert_eq!(g.start_action("b", None), Err(GameError::NoSlot));
        // Bureaucracy 40: two slots.
        let mut g = game(data, 1);
        g.world.axes.insert(ax("bureaucracy"), Fx::from_int(40));
        assert_eq!(g.start_action("a", None), Ok(()));
        assert_eq!(g.start_action("b", None), Ok(()));
    }

    #[test]
    fn start_action_checks() {
        let mut data = bare();
        let mut costly = action("costly", ActionTarget::None, vec![]);
        costly.cost = Fx::from_int(100); // treasury 150
        let mut barred = action("barred", ActionTarget::None, vec![]);
        barred.requires = Predicate::Flag("never".into());
        data.actions = vec![
            costly,
            barred,
            action("envoy", ActionTarget::Neighbour, vec![]),
            action("tutor", ActionTarget::Heir, vec![]),
        ];
        data.events = vec![event("e", vec![])];
        let mut g = game(data, 1);
        g.world.axes.insert(ax("bureaucracy"), Fx::from_int(100));
        let nordmark = Target::Neighbour(NeighbourId("nordmark".into()));
        assert_eq!(
            g.available_actions(),
            [
                ("costly".into(), vec![]),
                ("envoy".into(), vec![nordmark.clone()]),
                ("tutor".into(), vec![Target::Heir(0), Target::Heir(1)]),
            ]
        );
        assert_eq!(g.start_action("nothing", None), Err(GameError::Unknown));
        assert_eq!(g.start_action("barred", None), Err(GameError::Unavailable));
        assert_eq!(g.start_action("envoy", None), Err(GameError::Unavailable));
        assert_eq!(
            g.start_action("costly", Some(nordmark.clone())),
            Err(GameError::Unavailable)
        );
        g.start_action("costly", None).unwrap();
        assert_eq!(g.world.axes[&ax("treasury")], Fx::from_int(50));
        // Unaffordable now, and running on the same target anyway.
        g.start_action("tutor", Some(Target::Heir(1))).unwrap();
        assert_eq!(
            g.available_actions(),
            [
                ("envoy".into(), vec![nordmark.clone()]),
                ("tutor".into(), vec![Target::Heir(0)]),
            ]
        );
        assert_eq!(
            g.decisions.last().unwrap().kind,
            DecisionKind::ActionStarted {
                action_id: "tutor".into(),
                target: Some(Target::Heir(1))
            }
        );
        g.pending_event = Some(PendingEvent {
            event_id: "e".into(),
            target: None,
        });
        assert_eq!(
            g.start_action("envoy", Some(nordmark)),
            Err(GameError::EventPending)
        );
    }

    #[test]
    fn on_complete_applies_in_the_end_tick() {
        let mut data = bare();
        let mut a = action(
            "build",
            ActionTarget::Province(ProvinceFilter::default()),
            vec![
                Effect::Axis(ax("legitimacy"), Fx::from_int(5)),
                Effect::Province(
                    ProvinceTarget::EventTarget,
                    ProvinceField::Income,
                    Fx::from_int(3),
                ),
            ],
        );
        a.duration_years = Years(2);
        data.actions = vec![a];
        let mut g = game(data, 1);
        g.start_action("build", Some(Target::Province(pid("holm"))))
            .unwrap();
        assert_eq!(g.world.active_actions[0].ends_at, Tick(2));
        g.wait().unwrap();
        assert_eq!(g.world.axes[&ax("legitimacy")], Fx::from_int(60));
        assert_eq!(g.world.provinces[&pid("holm")].income, Fx::from_int(6));
        g.wait().unwrap();
        assert_eq!(g.world.tick, Tick(2));
        assert_eq!(g.world.axes[&ax("legitimacy")], Fx::from_int(65));
        assert_eq!(g.world.provinces[&pid("holm")].income, Fx::from_int(9));
        assert!(g.world.active_actions.is_empty());
    }

    #[test]
    fn spawn_event_fires_exactly_two_years_later() {
        let mut data = bare();
        data.time_unit = TimeUnit { ticks_per_year: 4 };
        let spawn = Effect::SpawnEvent("next".into(), Years(2));
        data.events = vec![event("first", vec![spawn]), event("next", vec![])];
        let mut g = game(data, 1);
        g.wait().unwrap();
        force(&mut g, "first");
        for _ in 0..7 {
            assert_eq!(g.wait().unwrap(), Step::Idle);
        }
        assert_eq!(fired(g.wait().unwrap()).as_deref(), Some("next"));
        assert_eq!(g.world.tick, Tick(9));
    }

    #[test]
    fn deferred_events() {
        let mut data = bare();
        let spawn = |id: &str| Effect::SpawnEvent(id.into(), Years(1));
        let mut pool = event("pool", vec![]);
        pool.weight = 1;
        let mut blocked = event("blocked", vec![]);
        blocked.when = Predicate::Flag("never".into());
        let mut once = event("once", vec![]);
        once.once = true;
        data.events = vec![
            event(
                "start",
                vec![spawn("a"), spawn("blocked"), spawn("unknown"), spawn("b")],
            ),
            event("a", vec![]),
            event("b", vec![spawn("once"), spawn("once")]),
            pool,
            blocked,
            once,
        ];
        let mut g = game(data, 1);
        force(&mut g, "start");
        // Deferred beat the pool; a not-ready or unknown one is dropped; one per tick;
        // the second `once` is dropped because the first fired.
        let mut next = || {
            let id = fired(g.wait().unwrap());
            g.choose(0).unwrap();
            id.unwrap()
        };
        assert_eq!(
            [next(), next(), next(), next(), next()],
            ["a", "b", "once", "pool", "pool"]
        );
    }

    #[test]
    fn once_event_fires_once() {
        let mut data = bare();
        let mut e = event("once", vec![]);
        (e.weight, e.once) = (1, true);
        data.events = vec![e];
        let mut g = game(data, 1);
        assert_eq!(fired(g.wait().unwrap()).as_deref(), Some("once"));
        g.choose(0).unwrap();
        for _ in 0..20 {
            assert_eq!(g.wait().unwrap(), Step::Idle);
        }
    }

    #[test]
    fn cooldown_in_years() {
        let mut data = bare();
        data.time_unit = TimeUnit { ticks_per_year: 2 };
        let mut e = event("e", vec![]);
        (e.weight, e.cooldown_years) = (1, Years(2));
        data.events = vec![e];
        let mut g = game(data, 1);
        let mut ticks = vec![];
        for _ in 0..12 {
            if fired(g.wait().unwrap()).is_some() {
                ticks.push(g.world.tick.0);
                g.choose(0).unwrap();
            }
        }
        assert_eq!(ticks, [1, 5, 9]);
    }

    #[test]
    fn quiet_weight_shares_the_pick() {
        let mut data = bare();
        let mut e = event("e", vec![]);
        e.weight = 1;
        data.events = vec![e];
        data.quiet_weight = 1;
        let mut g = game(data.clone(), 1);
        let mut hits = 0;
        for _ in 0..200 {
            if fired(g.wait().unwrap()).is_some() {
                hits += 1;
                g.choose(0).unwrap();
            }
        }
        assert!((70..130).contains(&hits), "{hits}");
        data.quiet_weight = 0;
        let mut g = game(data, 1);
        assert!(fired(g.wait().unwrap()).is_some());
    }

    #[test]
    fn targets_and_text() {
        let mut data = bare();
        let mut local = event(
            "local",
            vec![Effect::Province(
                ProvinceTarget::EventTarget,
                ProvinceField::Loyalty,
                Fx::from_int(10),
            )],
        );
        local.weight = 1;
        local.title = "{ruler}: {province}".into();
        local.choices[0].hint = Some("{province}, {neighbour}".into());
        let filter = "(holder: Vassal)";
        local.target = EventTarget::RandomProvince(crate::data::parse(filter).unwrap());
        let mut envoy = event("envoy", vec![]);
        envoy.text = "Посол {neighbour}".into();
        envoy.target = EventTarget::Neighbour;
        data.events = vec![local, envoy];
        let mut g = game(data, 1);
        let Step::Event(v) = g.wait().unwrap() else {
            panic!()
        };
        assert_eq!(v.title, "Ульрих: Хольм");
        assert_eq!(v.choices[0].hint.as_deref(), Some("Хольм, {neighbour}"));
        assert_eq!(v.target, Some(Target::Province(pid("holm"))));
        g.choose(0).unwrap();
        // Holm loyalty 40 drifted to 41, then +10.
        assert_eq!(g.world.provinces[&pid("holm")].loyalty, Fx::from_int(51));

        g.pending_event = Some(PendingEvent {
            event_id: "envoy".into(),
            target: Some(Target::Neighbour(NeighbourId("nordmark".into()))),
        });
        let Step::Event(v) = g.wait().unwrap() else {
            panic!()
        };
        assert_eq!(v.text, "Посол Нордмарк");

        // No vassal province left: the event cannot fire.
        let mut g = game(g.data.clone(), 1);
        g.world.provinces.get_mut(&pid("holm")).unwrap().holder = Holder::Crown;
        assert_eq!(g.wait().unwrap(), Step::Idle);
    }

    #[test]
    fn choose_and_pending() {
        let mut data = bare();
        let mut e = event("e", vec![Effect::SetFlag("chosen".into())]);
        e.weight = 1;
        data.events = vec![e];
        let mut g = game(data, 1);
        assert_eq!(g.choose(0), Err(GameError::NoEvent));
        let step = g.wait().unwrap();
        assert!(fired(step.clone()).is_some());
        // While an event waits, `wait` repeats it and time stands still.
        assert_eq!(g.wait().unwrap(), step);
        assert_eq!(g.world.tick, Tick(1));
        assert_eq!(g.choose(1), Err(GameError::BadChoice));
        assert_eq!(g.choose(0), Ok(()));
        assert!(g.world.flags.contains("chosen"));
        assert_eq!(g.world.last_fired["e"], Tick(1));
        assert_eq!(
            g.decisions,
            [Decision {
                tick: Tick(1),
                kind: DecisionKind::EventChoice {
                    event_id: "e".into(),
                    choice_idx: 0,
                    target: None
                },
                cause_tag: "e".into(),
            }]
        );
        assert_eq!(g.choose(0), Err(GameError::NoEvent));
    }

    #[test]
    fn passive_tick() {
        let mut data = bare();
        data.time_unit = TimeUnit { ticks_per_year: 4 };
        data.economy.flows = vec![(ax("income"), Fx::from_int(1)), (ax("army"), Fx(-100))];
        data.drift.step = Fx::from_int(1);
        data.drift.province_loyalty = Fx::from_int(50);
        let mut g = game(data, 1);
        g.world.crown_modifiers.insert(pid("holm"), Fx(500));
        g.wait().unwrap();
        assert_eq!(g.world.crown_modifiers[&pid("holm")], Fx(250));
        g.wait().unwrap();
        g.wait().unwrap();
        assert!(g.world.crown_modifiers.is_empty());
        assert_eq!(g.world.ruler.age, 30);
        g.wait().unwrap();
        let w = &g.world;
        // Per year: capital income 12 + income 10 - army 50 * 0.1 = 17, in quarters.
        assert_eq!(w.axes[&ax("treasury")], Fx::from_int(167));
        assert_eq!((w.ruler.age, w.heirs[0].age), (31, 9));
        // Toward the axis default 50, toward province_loyalty 50, a year's step of 1.
        assert_eq!(w.axes[&ax("loyalty_nobles")], Fx::from_int(41));
        assert_eq!(w.axes[&ax("loyalty")], Fx(45_500)); // recomputed: (82 + 50 + 50) / 4
        assert_eq!(w.provinces[&pid("holm")].loyalty, Fx::from_int(41));
        assert_eq!(w.provinces[&pid("capital")].loyalty, Fx::from_int(69));
        assert_eq!(w.provinces[&pid("nordheim")].loyalty, Fx::from_int(50));
    }
}
