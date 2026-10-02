//! A reign without UI: action -> tick -> event -> choice.

use crate::data::{Data, by_age};
use crate::fx::Fx;
use crate::neighbour::neighbour_tick;
use crate::rng::Rng;
use crate::rules::{
    ActionTarget, Choice, Ctx, Effect, Event, EventTarget, ProvinceField, Target, add_axis,
};
use crate::state::{
    ActiveAction, CauseTag, HeirStatus, Holder, MarkKey, NeighbourId, Preset, ProvinceId, World,
};
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
    /// `Game::abdicate`; the confirm or cancel choice follows as an `EventChoice`.
    Abdicate,
}

/// The event waiting for `choose`, or deferred in `Game.queue`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PendingEvent {
    pub event_id: String,
    pub target: Option<Target>,
    /// The neighbour behind a province target, e.g. the raider of a raided province;
    /// fills `{neighbour}` and is what `NeighbourTarget::EventTarget` means then.
    #[serde(default)]
    pub neighbour: Option<NeighbourId>,
}

/// An event with `{province}`, `{neighbour}`, `{ruler}`, `{heir}`, `{vassal}` (the holder of
/// a target province) filled in.
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
    /// Deferred events with their due tick, from `SpawnEvent` and `StartWar`. They keep the
    /// targets of what spawned them.
    pub queue: Vec<(Tick, PendingEvent)>,
    /// Cause of the reign end, set by `RulerDies` or `Abdicate`.
    pub ended: Option<String>,
    /// `wait` has returned `Step::ReignEnded`; every call errs from now on.
    pub reported: bool,
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
            ended: None,
            reported: false,
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
                // The same action on the same target runs once at a time.
                let busy = |t: Option<&Target>| {
                    let same = |x: &ActiveAction| x.target == t.map(target_key);
                    (w.active_actions.iter()).any(|x| x.id == a.id && same(x))
                };
                let free = |t: Target| (!busy(Some(&t))).then_some(t);
                let targets: Vec<Target> = match &a.target {
                    ActionTarget::Province(f) => (w.provinces.values())
                        .filter(|p| f.matches(p, w) && p.crown_power >= a.min_crown_power)
                        .filter_map(|p| free(Target::Province(p.id.clone())))
                        .collect(),
                    _ if capital_power < a.min_crown_power => return None,
                    ActionTarget::None => return (!busy(None)).then(|| (a.id.clone(), vec![])),
                    ActionTarget::Neighbour => (w.neighbours.keys())
                        .filter_map(|n| free(Target::Neighbour(n.clone())))
                        .collect(),
                    ActionTarget::Heir => (w.heirs.iter())
                        .filter_map(|h| free(Target::Heir(h.id)))
                        .collect(),
                };
                (!targets.is_empty()).then(|| (a.id.clone(), targets))
            })
            .collect()
    }

    pub fn start_action(&mut self, id: &str, target: Option<Target>) -> Result<(), GameError> {
        self.start(id, target, true)
    }

    /// `record`: a player decision, logged in `decisions` and marking what it touches.
    /// The simulation's automaton passes false.
    pub(crate) fn start(
        &mut self,
        id: &str,
        target: Option<Target>,
        record: bool,
    ) -> Result<(), GameError> {
        if self.ended.is_some() {
            return Err(GameError::ReignEnded);
        }
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
        if !record {
            return Ok(());
        }
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
        if self.ended.is_some() {
            return self.report_end();
        }
        if let Some(p) = &self.pending_event {
            return Ok(Step::Event(self.view(p)));
        }
        self.world.tick.0 += 1;
        self.passive();
        self.complete_actions();
        self.world.recompute_loyalty(&self.data);
        self.world.recompute_crown_power(&self.data);
        // Events neighbours start this year; they join the random pick.
        let mut offers = Vec::new();
        if (self.world.tick.0).is_multiple_of(self.data.time_unit.ticks_per_year) {
            let ids: Vec<_> = self.world.neighbours.keys().cloned().collect();
            let relations = |w: &World| w.neighbours.values().map(|n| n.relation).collect::<Vec<_>>();
            let before = relations(&self.world);
            for id in ids {
                offers.extend(neighbour_tick(
                    &mut self.world,
                    &self.data,
                    &mut self.rng,
                    id,
                ));
            }
            // Relations price the paths through foreign land; mostly they stay put.
            if relations(&self.world) != before {
                self.world.recompute_crown_power(&self.data);
            }
        }
        if self.ended.is_some() {
            return self.report_end();
        }
        let death = self.death_roll();
        let picked = death.or_else(|| {
            pick_event(
                &self.data,
                &self.world,
                &mut self.rng,
                &mut self.queue,
                offers,
            )
        });
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
        self.resolve(idx, true)
    }

    /// `record` as in `start`.
    pub(crate) fn resolve(&mut self, idx: usize, record: bool) -> Result<(), GameError> {
        if self.ended.is_some() {
            return Err(GameError::ReignEnded);
        }
        let p = self.pending_event.clone().ok_or(GameError::NoEvent)?;
        let event = find_event(&self.data, &p.event_id).expect("pending events exist");
        let choice = event.choices.get(idx).ok_or(GameError::BadChoice)?.clone();
        let before = record.then(|| self.world.without_marks());
        self.apply(&choice.effects, p.target.as_ref(), p.neighbour.as_ref());
        self.pending_event = None;
        if let Some(before) = &before {
            self.mark(self.decisions.len(), &choice.cause_tag, before);
            self.decisions.push(Decision {
                tick: self.world.tick,
                kind: DecisionKind::EventChoice {
                    event_id: p.event_id,
                    choice_idx: idx,
                    target: p.target,
                },
                cause_tag: choice.cause_tag,
            });
        }
        self.world.recompute_loyalty(&self.data);
        if moves_crown_power(&choice.effects) {
            self.world.recompute_crown_power(&self.data);
        }
        Ok(())
    }

    /// Marks what changed since `before` (axes, provinces with their crown modifiers, flags,
    /// neighbours, heirs) as touched by decision `idx`, weight 1.
    fn mark(&mut self, idx: usize, cause_tag: &str, before: &World) {
        let w = &self.world;
        let axes = (w.axes.iter()).filter(|(a, v)| before.axes.get(*a) != Some(*v));
        let provinces = w.provinces.iter().filter(|(id, p)| {
            before.provinces.get(*id) != Some(*p)
                || before.crown_modifiers.get(*id) != w.crown_modifiers.get(*id)
        });
        let neighbours = w.neighbours.iter();
        let neighbours = neighbours.filter(|(id, n)| before.neighbours.get(*id) != Some(*n));
        let heirs = w.heirs.iter().filter(|h| !before.heirs.contains(h));
        let keys: Vec<MarkKey> = (axes.map(|(a, _)| MarkKey::Axis(a.clone())))
            .chain(provinces.map(|(id, _)| MarkKey::Province(id.clone())))
            .chain(
                w.flags
                    .symmetric_difference(&before.flags)
                    .map(|f| MarkKey::Flag(f.clone())),
            )
            .chain(neighbours.map(|(id, _)| MarkKey::Neighbour(id.clone())))
            .chain(heirs.map(|h| MarkKey::Heir(h.id)))
            .collect();
        for k in keys {
            self.world.marks.entry(k).or_default().push(CauseTag {
                decision_idx: idx,
                cause_tag: cause_tag.into(),
                weight: Fx::from_int(1),
            });
        }
    }

    /// Treasury, aging, drift. Yearly amounts are spread over the ticks of a year.
    fn passive(&mut self) {
        let (d, w) = (&self.data, &mut self.world);
        let per_tick = |v: Fx| v / Fx::from_int(d.time_unit.ticks_per_year as i64);
        let income = d.economy.yearly_income(w);
        add_axis(w, d, &d.economy.treasury, per_tick(income));

        if w.tick.0 % d.time_unit.ticks_per_year == 0 {
            w.ruler.age += 1;
            w.heirs.iter_mut().for_each(|h| h.age += 1);
            heirs_year(d, w, &mut self.rng);
            for tags in w.marks.values_mut() {
                tags.iter_mut()
                    .for_each(|t| t.weight = t.weight * d.sim.decay);
                tags.retain(|t| t.weight > Fx(0));
            }
            w.marks.retain(|_, tags| !tags.is_empty());
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
            let target = a.target.clone().map(|key| match def.target {
                ActionTarget::Province(_) => Target::Province(ProvinceId(key)),
                ActionTarget::Neighbour => Target::Neighbour(NeighbourId(key)),
                ActionTarget::Heir => Target::Heir(key.parse().expect("written by target_key")),
                ActionTarget::None => unreachable!("untargeted actions store no target"),
            });
            let (effects, tag) = (def.on_complete.clone(), def.cause_tag.clone());
            // The decision that started it; none for the simulation's automaton.
            let started = self.decisions.iter().rposition(|d| match &d.kind {
                DecisionKind::ActionStarted { action_id, target } => {
                    *action_id == a.id && target.as_ref().map(target_key) == a.target
                }
                _ => false,
            });
            let before = started.map(|_| self.world.without_marks());
            self.apply(&effects, target.as_ref(), None);
            if let (Some(idx), Some(before)) = (started, before) {
                self.mark(idx, &tag, &before);
            }
        }
    }

    /// Fires the abdication event from `Data.abdication`; its choices confirm or cancel.
    pub fn abdicate(&mut self) -> Result<(), GameError> {
        if self.ended.is_some() {
            return Err(GameError::ReignEnded);
        }
        if self.pending_event.is_some() {
            return Err(GameError::EventPending);
        }
        let id = &self.data.abdication.event;
        find_event(&self.data, id).ok_or(GameError::Unknown)?;
        self.pending_event = Some(PendingEvent {
            event_id: id.clone(),
            target: None,
            neighbour: None,
        });
        self.decisions.push(Decision {
            tick: self.world.tick,
            kind: DecisionKind::Abdicate,
            cause_tag: id.clone(),
        });
        Ok(())
    }

    /// Applies effects in order. Those needing the rng or ending the reign are done here.
    fn apply(&mut self, effects: &[Effect], target: Option<&Target>, nb: Option<&NeighbourId>) {
        for e in effects {
            match e {
                Effect::Chance(c) => {
                    let hit = self.rng.range(0, Fx::from_int(100).0) < c.percent(&self.world).0;
                    self.apply(if hit { &c.then } else { &c.otherwise }, target, nb);
                }
                Effect::Clash => crate::war::clash(&mut self.world, &self.data, &mut self.rng),
                Effect::IfFriendly(es) => {
                    let n = match target {
                        Some(Target::Neighbour(n)) => Some(n),
                        _ => nb,
                    };
                    let n = n.and_then(|n| self.world.neighbours.get(n));
                    if n.is_some_and(|n| n.relation > self.data.neighbour_ai.friendly_above) {
                        self.apply(es, target, nb);
                    }
                }
                Effect::RulerDies(cause) => self.ended = Some(cause.clone()),
                Effect::Abdicate => {
                    let (a, w) = (&self.data.abdication, &mut self.world);
                    let (axis, min) = &a.institutions;
                    let calm = w.axes[axis] > *min
                        && w.heirs.first().is_some_and(|h| h.ability > a.heir_ability);
                    if !calm {
                        if let Some(h) = w.heirs.first_mut() {
                            h.claim = (h.claim - a.claim_drop).max(Fx(0));
                        }
                        w.flags.insert(a.contested_flag.clone());
                    }
                    self.ended = Some(a.event.clone());
                }
                e => {
                    let mut ctx = Ctx {
                        data: &self.data,
                        queue: &mut self.queue,
                        target,
                        neighbour: nb,
                    };
                    e.apply(&mut self.world, &mut ctx);
                }
            }
        }
    }

    /// Rolls the ruler's death risk (`Data.death`). A hit returns its death event, which
    /// goes before any other event this tick; one whose `when` is false is dropped.
    fn death_roll(&mut self) -> Option<PendingEvent> {
        let (d, w) = (&self.data, &self.world);
        let health = (Fx::from_int(100) - w.ruler.health) * d.death.health_k;
        let base = (by_age(&d.death.base, w.ruler.age) + health, &d.death.event);
        let risks = d.death.risks.iter().filter(|r| r.when.eval(w));
        let risks = risks.map(|r| (r.per_mille, &r.event));
        // Per mille per year, in Fx units, spread over the ticks of a year.
        let ticks = d.time_unit.ticks_per_year as i64;
        let mut roll = self.rng.range(0, 1000 * Fx::SCALE * ticks);
        for (risk, id) in std::iter::once(base).chain(risks) {
            let event = find_event(d, id);
            if event.is_some_and(|e| cooling(e, w)) {
                continue;
            }
            if roll < risk.0 {
                return event.filter(|e| e.when.eval(w)).map(|e| PendingEvent {
                    event_id: e.id.clone(),
                    target: None,
                    neighbour: None,
                });
            }
            roll -= risk.0;
        }
        None
    }

    fn report_end(&mut self) -> Result<Step, GameError> {
        if self.reported {
            return Err(GameError::ReignEnded);
        }
        self.reported = true;
        Ok(Step::ReignEnded(ReignEnd {
            cause: self.ended.clone().expect("called once the reign ended"),
            tick: self.world.tick,
            world: self.world.snapshot(),
        }))
    }

    fn view(&self, p: &PendingEvent) -> EventView {
        let e = find_event(&self.data, &p.event_id).expect("pending events exist");
        let w = &self.world;
        let name = match &p.target {
            Some(Target::Province(id)) => w.provinces.get(id).map(|x| ("{province}", &x.name)),
            Some(Target::Neighbour(id)) => w.neighbours.get(id).map(|x| ("{neighbour}", &x.name)),
            Some(Target::Heir(id)) => (w.heir_index(*id)).map(|i| ("{heir}", &w.heirs[i].name)),
            _ => None,
        };
        let behind = (p.neighbour.as_ref()).and_then(|n| w.neighbours.get(n));
        let vassal = match &p.target {
            Some(Target::Province(id)) => match w.provinces.get(id).map(|p| &p.holder) {
                Some(Holder::Vassal(v)) => w.vassals.get(v),
                _ => None,
            },
            _ => None,
        };
        let names = [
            Some(("{ruler}", &w.ruler.name)),
            name,
            behind.map(|n| ("{neighbour}", &n.name)),
            vassal.map(|v| ("{vassal}", &v.name)),
        ];
        let fill = |s: &str| {
            names
                .iter()
                .flatten()
                .fold(s.to_string(), |s, (k, v)| match s.contains(k) {
                    true => s.replace(k, v),
                    false => s,
                })
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

/// Yearly: ability grows by status until adulthood, claims follow the succession law,
/// hostages lose claim, a child may be born.
fn heirs_year(d: &Data, w: &mut World, rng: &mut Rng) {
    let r = &d.heirs;
    let pct = |v: Fx| v.clamp(Fx(0), Fx::from_int(100));
    let law = r.laws.iter().find(|l| w.flags.contains(&l.flag));
    for (i, h) in w.heirs.iter_mut().enumerate() {
        let (growth, claim) = match h.status {
            HeirStatus::Home => (r.growth_home, Fx(0)),
            HeirStatus::Studying(_) => (r.growth_studying, Fx(0)),
            HeirStatus::Hostage(_) => (r.growth_hostage, r.hostage_claim),
        };
        if h.age < r.adult_age {
            h.ability = pct(h.ability + growth);
        }
        if let Some(l) = law {
            let base = if i == 0 { l.eldest } else { l.others };
            let target = base + h.ability * l.ability_k;
            h.claim = match h.claim < target {
                true => (h.claim + r.claim_step).min(target),
                false => (h.claim - r.claim_step).max(target),
            };
        }
        h.claim = pct(h.claim + claim);
    }
    let married = w.flags.contains(&r.married_flag);
    let factor = if married {
        Fx::from_int(1)
    } else {
        r.unmarried
    };
    let chance = by_age(&r.birth, w.ruler.age) * factor;
    if rng.range(0, Fx::from_int(100).0) < chance.0 {
        w.add_heir(d.newborn(w.next_heir_id));
    }
}

/// Whether the effects may change an input of `World::recompute_crown_power`: holders,
/// province loyalty, buildings, crown modifiers, relations. Most choices touch axes only,
/// and the recompute is a good part of a tick.
fn moves_crown_power(effects: &[Effect]) -> bool {
    effects.iter().any(|e| match e {
        Effect::Chance(c) => moves_crown_power(&c.then) || moves_crown_power(&c.otherwise),
        Effect::IfFriendly(es) => moves_crown_power(es),
        Effect::Province(_, field, _) => *field == ProvinceField::Loyalty,
        Effect::Axis(..)
        | Effect::SetFlag(_)
        | Effect::ClearFlag(_)
        | Effect::SpawnEvent(..)
        | Effect::RulerHealth(_)
        | Effect::HeirOp(_)
        | Effect::RulerDies(_)
        | Effect::Abdicate
        | Effect::StartWar(_)
        | Effect::Clash
        | Effect::Tribute(_)
        | Effect::TakeHostage(..)
        | Effect::EndWar(_)
        | Effect::SetWarStage(_) => false,
        _ => true,
    })
}

fn cooling(e: &Event, w: &World) -> bool {
    let cooldown = e.cooldown_years.ticks(w.time_unit).0;
    // Checked first: the lookup is the costly part of the event pick.
    cooldown > 0 && (w.last_fired.get(&e.id)).is_some_and(|t| w.tick.0 < t.0 + cooldown)
}

fn find_event<'a>(data: &'a Data, id: &str) -> Option<&'a Event> {
    data.events.iter().find(|e| e.id == id)
}

/// `None`: the event needs no target. `Some(empty)`: it cannot fire now. Lazy: the pick
/// asks every event of the pool each tick whether it has a target at all.
fn candidates<'a>(
    target: &'a EventTarget,
    w: &'a World,
) -> Option<Box<dyn Iterator<Item = Target> + 'a>> {
    match target {
        EventTarget::None => None,
        EventTarget::RandomProvince(f) => Some(Box::new(
            (w.provinces.values())
                .filter(|p| f.matches(p, w))
                .map(|p| Target::Province(p.id.clone())),
        )),
        EventTarget::Neighbour => Some(Box::new(
            w.neighbours.keys().map(|n| Target::Neighbour(n.clone())),
        )),
        EventTarget::Heir(lo, hi) => Some(Box::new(
            (w.heirs.iter())
                .filter(|h| (*lo..=*hi).contains(&h.age))
                .map(|h| Target::Heir(h.id)),
        )),
    }
}

/// At most one event per tick. Due deferred events go first, earliest due first, with the
/// target they were spawned with, or a fresh one if they had none. Then a weighted pick over
/// ready pool events off cooldown, the neighbours' `offers` (`neighbour_ai.weight` each, kept
/// targets) and `quiet_weight` for no event; events add their `Event::bonus`. Any event is dropped when its `when` is false,
/// it already fired `once`, or it has no target; unpicked offers are dropped too.
fn pick_event(
    data: &Data,
    w: &World,
    rng: &mut Rng,
    queue: &mut Vec<(Tick, PendingEvent)>,
    offers: Vec<PendingEvent>,
) -> Option<PendingEvent> {
    let ready = |e: &Event| e.when.eval(w) && !(e.once && w.last_fired.contains_key(&e.id));
    let targets = |e: &Event| candidates(&e.target, w).is_none_or(|mut t| t.next().is_some());
    let fire = |e: &Event, rng: &mut Rng| {
        let mut pick = |t: Vec<Target>| t[rng.range(0, t.len() as i64) as usize].clone();
        let target = candidates(&e.target, w).map(|t| pick(t.collect()));
        Some(PendingEvent {
            event_id: e.id.clone(),
            target,
            neighbour: None,
        })
    };

    let due = |q: &Vec<(Tick, PendingEvent)>| {
        (0..q.len())
            .filter(|&i| q[i].0 <= w.tick)
            .min_by_key(|&i| q[i].0)
    };
    while let Some(i) = due(queue) {
        let (_, p) = queue.remove(i);
        match find_event(data, &p.event_id).filter(|e| ready(e)) {
            Some(_) if p.target.is_some() => return Some(p),
            Some(e) if targets(e) => return fire(e, rng),
            _ => {}
        }
    }

    let pool: Vec<&Event> = (data.events.iter())
        .filter(|e| e.weight > 0 && !cooling(e, w) && ready(e) && targets(e))
        .collect();
    let pool: Vec<(&Event, u32)> = (pool.into_iter())
        .map(|e| (e, e.weight + e.bonus(w)))
        .collect();
    let offers: Vec<(PendingEvent, u32)> = (offers.into_iter())
        .filter_map(|p| {
            let e = find_event(data, &p.event_id).filter(|e| ready(e))?;
            Some((p, data.neighbour_ai.weight + e.bonus(w)))
        })
        .collect();
    let total = data.quiet_weight
        + pool.iter().map(|(_, wt)| wt).sum::<u32>()
        + offers.iter().map(|(_, wt)| wt).sum::<u32>();
    if total == 0 {
        return None;
    }
    let mut roll = rng.range(0, total as i64) as u32;
    for (e, wt) in pool {
        if roll < wt {
            return fire(e, rng);
        }
        roll -= wt;
    }
    for (p, wt) in offers {
        if roll < wt {
            return Some(p);
        }
        roll -= wt;
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
    const MAP: &str = include_str!("../../../data/maps/default.ron");

    fn ax(s: &str) -> AxisId {
        AxisId(s.into())
    }

    fn pid(s: &str) -> ProvinceId {
        ProvinceId(s.into())
    }

    fn game(data: Data, seed: u64) -> Game {
        let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
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
            weight_bonus: vec![],
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
            neighbour: None,
        });
        g.choose(0).unwrap();
    }

    fn play(seed: u64) -> Game {
        let mut data = crate::data::load(RULES).unwrap();
        data.add_events(include_str!("../../../data/events/reign.ron"))
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
        tax.min_crown_power = Fx::from_int(60); // provinces next to the capital have 55
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
        let neighbours: Vec<_> = (g.world.neighbours.keys())
            .map(|n| Target::Neighbour(n.clone()))
            .collect();
        assert_eq!(neighbours.len(), 3);
        assert_eq!(
            g.available_actions(),
            [
                ("costly".into(), vec![]),
                ("envoy".into(), neighbours.clone()),
                ("tutor".into(), vec![Target::Heir(0)]),
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
        g.start_action("tutor", Some(Target::Heir(0))).unwrap();
        assert_eq!(g.available_actions(), [("envoy".into(), neighbours)]);
        assert_eq!(
            g.decisions.last().unwrap().kind,
            DecisionKind::ActionStarted {
                action_id: "tutor".into(),
                target: Some(Target::Heir(0))
            }
        );
        g.pending_event = Some(PendingEvent {
            event_id: "e".into(),
            target: None,
            neighbour: None,
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
        assert_eq!(g.world.axes[&ax("legitimacy")], Fx::from_int(45));
        assert_eq!(g.world.provinces[&pid("holm")].income, Fx::from_int(6));
        g.wait().unwrap();
        assert_eq!(g.world.tick, Tick(2));
        assert_eq!(g.world.axes[&ax("legitimacy")], Fx::from_int(50));
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
        local.text = "{vassal}".into();
        local.choices[0].hint = Some("{province}, {neighbour}".into());
        let filter = r#"(holder: Vassal, building: "road")"#; // holm only
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
        assert_eq!(v.text, "Вейр"); // the holder of the target province
        assert_eq!(v.choices[0].hint.as_deref(), Some("Хольм, {neighbour}"));
        assert_eq!(v.target, Some(Target::Province(pid("holm"))));
        g.choose(0).unwrap();
        // Holm loyalty 40 drifted to 41, then +10.
        assert_eq!(g.world.provinces[&pid("holm")].loyalty, Fx::from_int(51));

        g.pending_event = Some(PendingEvent {
            event_id: "envoy".into(),
            target: Some(Target::Neighbour(NeighbourId("nordmark".into()))),
            neighbour: None,
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
    fn weight_bonus_joins_the_pick() {
        let mut data = bare();
        data.quiet_weight = 1;
        let mut e = event("risky", vec![]);
        e.weight = 1;
        e.weight_bonus = vec![(Predicate::Flag("plague".into()), 99)];
        data.events = vec![e];
        let hits = |plague: bool| {
            let mut g = game(data.clone(), 1);
            if plague {
                g.world.flags.insert("plague".into());
            }
            let mut hits = 0;
            for _ in 0..200 {
                if fired(g.wait().unwrap()).is_some() {
                    hits += 1;
                    g.choose(0).unwrap();
                }
            }
            hits
        };
        assert!((70..130).contains(&hits(false)), "{}", hits(false));
        assert!(hits(true) > 190, "{}", hits(true));
        // A neighbour's offer gets the bonus too.
        let mut offer = event("offer", vec![]);
        offer.weight_bonus = vec![(Predicate::All(vec![]), 5)];
        (data.events, data.quiet_weight, data.neighbour_ai.weight) = (vec![offer], 0, 0);
        let g = game(data.clone(), 1);
        let p = PendingEvent {
            event_id: "offer".into(),
            target: None,
            neighbour: None,
        };
        let mut rng = Rng::from_seed(1);
        let pick = pick_event(&data, &g.world, &mut rng, &mut vec![], vec![p.clone()]);
        assert_eq!(pick, Some(p.clone()));
        data.events[0].weight_bonus.clear();
        let pick = pick_event(&data, &g.world, &mut rng, &mut vec![], vec![p]);
        assert_eq!(pick, None);
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

    /// `bare()` plus the real actions and neighbour events.
    fn map_game() -> Game {
        let mut data = bare();
        data.add_actions(include_str!("../../../data/actions.ron"))
            .unwrap();
        data.add_events(include_str!("../../../data/events/neighbours.ron"))
            .unwrap();
        // Quiet neighbours unless a test says otherwise: hostile Nordmark made neutral,
        // no events from neutral or trading ones.
        data.neighbour_ai.wait.events.clear();
        data.neighbour_ai.trade.events.clear();
        let mut g = game(data, 1);
        g.world.axes.insert(ax("treasury"), Fx::from_int(1000));
        let nordmark = g.world.neighbours.get_mut(&NeighbourId("nordmark".into()));
        nordmark.unwrap().relation = Fx(0);
        g
    }

    fn targets(g: &Game, action: &str) -> Vec<Target> {
        let all = g.available_actions().into_iter();
        all.filter(|(id, _)| id == action)
            .flat_map(|(_, t)| t)
            .collect()
    }

    /// Runs `action` on `province` to completion next to an untouched game; returns
    /// (crown power, loyalty_nobles) of both.
    fn compare(action: &str, province: &str) -> ((Fx, Fx), (Fx, Fx)) {
        let mut base = map_game();
        let mut g = map_game();
        g.start_action(action, Some(Target::Province(pid(province))))
            .unwrap();
        base.wait().unwrap();
        g.wait().unwrap();
        let at = |g: &Game| {
            let power = g.world.provinces[&pid(province)].crown_power;
            (power, g.world.axes[&ax("loyalty_nobles")])
        };
        (at(&base), at(&g))
    }

    #[test]
    fn grant_and_revoke_province() {
        let ((power, nobles), (granted_power, granted_nobles)) = compare("grant_province", "berg");
        assert!(granted_power < power, "{granted_power} vs {power}");
        assert!(granted_nobles > nobles, "{granted_nobles} vs {nobles}");
        let ((power, nobles), (revoked_power, revoked_nobles)) = compare("revoke_province", "holm");
        assert!(revoked_power > power, "{revoked_power} vs {power}");
        assert!(revoked_nobles < nobles, "{revoked_nobles} vs {nobles}");
        // Grant goes to crown provinces but the capital, revoke to vassal ones.
        let g = map_game();
        let grant = targets(&g, "grant_province");
        assert!(grant.contains(&Target::Province(pid("berg"))));
        assert!(!grant.contains(&Target::Province(pid("capital"))));
        assert!(!grant.contains(&Target::Province(pid("holm"))));
        assert!(targets(&g, "revoke_province").contains(&Target::Province(pid("holm"))));
    }

    #[test]
    fn build_fort_only_on_crown_land() {
        let mut g = map_game();
        let forts = targets(&g, "build_fort");
        assert!(forts.contains(&Target::Province(pid("berg"))));
        for vassal in ["holm", "weir", "arden", "mar"] {
            assert!(!forts.contains(&Target::Province(pid(vassal))), "{vassal}");
        }
        let holm = Some(Target::Province(pid("holm")));
        assert_eq!(
            g.start_action("build_fort", holm),
            Err(GameError::Unavailable)
        );
        // Four years later the fort stands and adds its crown power.
        let before = g.world.provinces[&pid("berg")].crown_power;
        g.start_action("build_fort", Some(Target::Province(pid("berg"))))
            .unwrap();
        for _ in 0..4 {
            g.wait().unwrap();
        }
        let berg = &g.world.provinces[&pid("berg")];
        assert!(berg.buildings.contains("fort"));
        assert_eq!(berg.crown_power, before + Fx::from_int(10));
    }

    #[test]
    fn marriage_raises_relation_once() {
        let mut g = map_game();
        let vestrum = Target::Neighbour(NeighbourId("vestrum".into()));
        g.start_action("marry_neighbour", Some(vestrum.clone()))
            .unwrap();
        g.wait().unwrap();
        // 40 + 30 on completion; then Vestrum, friendly, trades: +1, and drifts toward 0: -1.
        let n = &g.world.neighbours[&NeighbourId("vestrum".into())];
        assert_eq!(n.relation, Fx::from_int(70));
        assert!(g.world.flags.contains("royal_marriage"));
        assert!(targets(&g, "marry_neighbour").is_empty());
    }

    #[test]
    fn neighbour_events_reach_the_player() {
        let mut g = map_game();
        let nordmark = NeighbourId("nordmark".into());
        let n = g.world.neighbours.get_mut(&nordmark).unwrap();
        (n.relation, n.strength) = (Fx::from_int(-80), Fx::from_int(100));
        let mut seen = vec![];
        for _ in 0..20 {
            if let Step::Event(v) = g.wait().unwrap() {
                seen.push((v.event_id, v.target, v.text));
                g.choose(0).unwrap();
            }
        }
        assert!(seen.iter().any(|(id, ..)| id == "neighbour_raid"));
        for (id, target, text) in seen {
            let expected = match id.as_str() {
                // The raid hits a province and names its raider.
                "neighbour_raid" => {
                    assert!(text.contains("Отряды Нордмарк"), "{text}");
                    Target::Province(pid("arden"))
                }
                "neighbour_ultimatum" | "neighbour_war_declared" => {
                    Target::Neighbour(nordmark.clone())
                }
                other => panic!("{other}"),
            };
            assert_eq!(target, Some(expected));
        }
    }

    #[test]
    fn repelled_raid_angers_the_raider() {
        let mut g = map_game();
        let nordmark = NeighbourId("nordmark".into());
        g.pending_event = Some(PendingEvent {
            event_id: "neighbour_raid".into(),
            target: Some(Target::Province(pid("arden"))),
            neighbour: Some(nordmark.clone()),
        });
        g.choose(0).unwrap();
        assert_eq!(g.world.neighbours[&nordmark].relation, Fx::from_int(-5));
    }

    #[test]
    fn heir_action_follows_the_heir_not_the_index() {
        let mut data = bare();
        let tutor = Effect::HeirOp(crate::rules::HeirOp::TargetAbility(Fx::from_int(10)));
        let mut a = action("tutor", ActionTarget::Heir, vec![tutor]);
        a.duration_years = Years(2);
        data.actions = vec![a];
        data.heirs.birth = vec![];
        let mut g = game(data.clone(), 1);
        g.world.add_heir(data.new_heir.clone()); // id 1, ability 50
        assert_eq!(
            g.available_actions(),
            [("tutor".into(), vec![Target::Heir(0), Target::Heir(1)])]
        );
        g.start_action("tutor", Some(Target::Heir(1))).unwrap();
        g.wait().unwrap();
        // The first heir dies; the newborn moves to index 0 but keeps its id.
        g.world.heirs.remove(0);
        g.wait().unwrap();
        assert_eq!(
            (g.world.heirs[0].id, g.world.heirs[0].ability),
            (1, Fx::from_int(64))
        );
    }

    #[test]
    fn deferred_event_keeps_the_spawner_target() {
        let mut data = bare();
        let mut first = event("first", vec![Effect::SpawnEvent("next".into(), Years(1))]);
        first.target = EventTarget::Neighbour;
        let mut next = event("next", vec![]);
        next.text = "Послы {neighbour}".into();
        data.events = vec![first, next];
        let mut g = game(data, 1);
        let vestrum = Target::Neighbour(NeighbourId("vestrum".into()));
        g.pending_event = Some(PendingEvent {
            event_id: "first".into(),
            target: Some(vestrum.clone()),
            neighbour: None,
        });
        g.choose(0).unwrap();
        let Step::Event(v) = g.wait().unwrap() else {
            panic!()
        };
        assert_eq!((v.event_id.as_str(), v.target), ("next", Some(vestrum)));
        assert_eq!(v.text, "Послы Веструм");
    }

    #[test]
    fn passive_tick() {
        let mut data = bare();
        data.time_unit = TimeUnit { ticks_per_year: 4 };
        data.economy.flows = vec![(ax("income"), Fx::from_int(1)), (ax("army"), Fx(-100))];
        data.drift.step = Fx::from_int(1);
        data.drift.province_loyalty = Fx::from_int(50);
        let mut g = game(data, 1);
        // The yearly sum below, as the UI shows it.
        assert_eq!(g.data.economy.yearly_income(&g.world), Fx::from_int(49));
        g.world.crown_modifiers.insert(pid("holm"), Fx(500));
        g.wait().unwrap();
        assert_eq!(g.world.crown_modifiers[&pid("holm")], Fx(250));
        g.wait().unwrap();
        g.wait().unwrap();
        assert!(g.world.crown_modifiers.is_empty());
        assert_eq!(g.world.ruler.age, 32);
        g.wait().unwrap();
        let w = &g.world;
        // Per year: crown provinces 12 + 7 + 5 + 8 + 6 + 6 + income 10 - army 50 * 0.1 = 49,
        // in quarters.
        assert_eq!(w.axes[&ax("treasury")], Fx::from_int(199));
        assert_eq!((w.ruler.age, w.heirs[0].age), (33, 7));
        // Toward the axis default 50, toward province_loyalty 50, a year's step of 1.
        assert_eq!(w.axes[&ax("loyalty_nobles")], Fx::from_int(41));
        assert_eq!(w.axes[&ax("loyalty")], Fx(47_750)); // recomputed: (82 + 59 + 50) / 4
        assert_eq!(w.provinces[&pid("holm")].loyalty, Fx::from_int(41));
        assert_eq!(w.provinces[&pid("capital")].loyalty, Fx::from_int(69));
        assert_eq!(w.provinces[&pid("nordheim")].loyalty, Fx::from_int(50));
    }
}
