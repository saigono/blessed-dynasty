//! Content schema read by `Game`: predicates, effects, events, actions.

use crate::data::Data;
use crate::fx::Fx;
use crate::game::PendingEvent;
use crate::state::{
    AxisId, HeirStatus, Holder, Neighbour, NeighbourId, Province, ProvinceId, Stance, Vassal,
    VassalId, World,
};
use crate::time::{Tick, Years};
use crate::war::{War, WarOutcome, WarStage};
use serde::{Deserialize, Serialize};

/// Province loyalty, ruler health and heir ability live in 0..=100, relations in -100..=100.
const PERCENT: Fx = Fx::from_int(100);

/// What an event or an action is about.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Province(ProvinceId),
    Neighbour(NeighbourId),
    /// `Heir.id`, stable while heirs are born and die.
    Heir(u32),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Predicate {
    /// Strictly above.
    AxisAbove(AxisId, Fx),
    /// Strictly below.
    AxisBelow(AxisId, Fx),
    Flag(String),
    NotFlag(String),
    /// At least one province matches.
    ProvinceWhere(ProvinceFilter),
    /// Inclusive range.
    HeirCount(u32, u32),
    /// Unrecognized bastards (`World.bastards`), inclusive range.
    BastardCount(u32, u32),
    /// Inclusive range.
    RulerAge(u32, u32),
    AtWar,
    /// Strictly above; false without a war.
    WarScoreAbove(Fx),
    /// Strictly below; false without a war.
    WarScoreBelow(Fx),
    /// False without a war.
    WarStage(WarStage),
    /// Age of the heir at this index within an inclusive range; false without that heir.
    HeirAge(u32, u32, u32),
    /// Claims of heirs 0 and 1 differ by less than this; false with fewer than two heirs.
    ClaimGapBelow(Fx),
    /// `All([])` is always true.
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
    Not(Box<Predicate>),
}

impl Predicate {
    pub fn eval(&self, w: &World) -> bool {
        match self {
            Predicate::AxisAbove(a, v) => w.axes[a] > *v,
            Predicate::AxisBelow(a, v) => w.axes[a] < *v,
            Predicate::Flag(f) => w.flags.contains(f),
            Predicate::NotFlag(f) => !w.flags.contains(f),
            Predicate::ProvinceWhere(f) => w.provinces.values().any(|p| f.matches(p, w)),
            Predicate::HeirCount(lo, hi) => (*lo..=*hi).contains(&(w.heirs.len() as u32)),
            Predicate::BastardCount(lo, hi) => (*lo..=*hi).contains(&(w.bastards.len() as u32)),
            Predicate::RulerAge(lo, hi) => (*lo..=*hi).contains(&w.ruler.age),
            Predicate::AtWar => w.war.is_some(),
            Predicate::WarScoreAbove(v) => w.war.as_ref().is_some_and(|x| x.war_score > *v),
            Predicate::WarScoreBelow(v) => w.war.as_ref().is_some_and(|x| x.war_score < *v),
            Predicate::WarStage(s) => w.war.as_ref().is_some_and(|x| x.stage == *s),
            Predicate::HeirAge(i, lo, hi) => {
                (w.heirs.get(*i as usize)).is_some_and(|h| (*lo..=*hi).contains(&h.age))
            }
            Predicate::ClaimGapBelow(v) => match w.heirs.as_slice() {
                [a, b, ..] => (a.claim - b.claim).max(b.claim - a.claim) < *v,
                _ => false,
            },
            Predicate::All(ps) => ps.iter().all(|p| p.eval(w)),
            Predicate::Any(ps) => ps.iter().any(|p| p.eval(w)),
            Predicate::Not(p) => !p.eval(w),
        }
    }

    pub(crate) fn check(&self, data: &Data) -> Result<(), String> {
        match self {
            Predicate::AxisAbove(a, _) | Predicate::AxisBelow(a, _) => known_axis(data, a),
            Predicate::All(ps) | Predicate::Any(ps) => ps.iter().try_for_each(|p| p.check(data)),
            Predicate::Not(p) => p.check(data),
            _ => Ok(()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum HolderKind {
    Crown,
    Vassal,
    Foreign,
}

/// Every set field must match; `()` matches any province.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ProvinceFilter {
    pub holder: Option<HolderKind>,
    pub loyalty_below: Option<Fx>,
    pub loyalty_above: Option<Fx>,
    pub building: Option<String>,
    pub without_building: Option<String>,
    /// Borders a province of a foreign state (other than its own holder).
    pub borders_foreign: Option<bool>,
    /// Is the capital.
    pub capital: Option<bool>,
    /// Held by a vassal whose strength times the number of his provinces exceeds the crown
    /// power here.
    pub vassal_stronger: Option<bool>,
    /// Held by a vassal whose `World::vassal_ratio` here is above this.
    pub vassal_ratio_above: Option<Fx>,
    /// Borders a province of the crown or a vassal; for foreign land, what a war can take.
    pub borders_realm: Option<bool>,
}

impl ProvinceFilter {
    pub fn matches(&self, p: &Province, w: &World) -> bool {
        let kind = match p.holder {
            Holder::Crown => HolderKind::Crown,
            Holder::Vassal(_) => HolderKind::Vassal,
            Holder::Foreign(_) => HolderKind::Foreign,
        };
        let stronger = || match &p.holder {
            Holder::Vassal(v) => w.vassals.get(v).is_some_and(|x| {
                let held = w
                    .provinces
                    .values()
                    .filter(|q| q.holder == p.holder)
                    .count();
                x.strength * Fx::from_int(held as i64) > p.crown_power
            }),
            _ => false,
        };
        self.holder.is_none_or(|h| h == kind)
            && self.loyalty_below.is_none_or(|v| p.loyalty < v)
            && self.loyalty_above.is_none_or(|v| p.loyalty > v)
            && self
                .building
                .as_ref()
                .is_none_or(|b| p.buildings.contains(b))
            && (self.without_building.as_ref()).is_none_or(|b| !p.buildings.contains(b))
            && (self.borders_foreign).is_none_or(|b| b == w.foreign_of(p).next().is_some())
            && (self.capital).is_none_or(|c| c == (p.id == w.capital.province))
            && self.vassal_stronger.is_none_or(|b| b == stronger())
            && (self.vassal_ratio_above).is_none_or(|v| w.vassal_ratio(p).is_some_and(|r| r > v))
            && self.borders_realm.is_none_or(|b| {
                let own = |q: &ProvinceId| {
                    (w.provinces.get(q)).is_some_and(|q| !matches!(q.holder, Holder::Foreign(_)))
                };
                b == p.neighbours.iter().any(own)
            })
    }
}

/// Values are deltas; results are clamped to their range.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Effect {
    Axis(AxisId, Fx),
    Province(ProvinceTarget, ProvinceField, Fx),
    SetFlag(String),
    ClearFlag(String),
    /// Queues the event to fire this many years from now.
    SpawnEvent(String, Years),
    RulerHealth(Fx),
    Relation(NeighbourTarget, Fx),
    /// A lasting modifier on top of the crown power formula; drifts back to 0.
    CrownPower(ProvinceTarget, Fx),
    HeirOp(HeirOp),
    /// Adds the building to the province; its crown power bonus comes from `rules.ron`.
    Build(ProvinceTarget, String),
    /// A crown province goes to a vassal chosen by `Data.grant`; otherwise a no-op.
    Grant(ProvinceTarget),
    /// A vassal province goes back to the crown; otherwise a no-op.
    Revoke(ProvinceTarget),
    /// Ends the reign with this cause. Applied by `Game`.
    RulerDies(String),
    /// Ends the reign, see `Data.abdication`. Applied by `Game`.
    Abdicate,
    /// Applies `then` or `otherwise` by a roll. Applied by `Game`, which owns the rng.
    Chance(Chance),
    /// Starts a war on the neighbour and queues `Data.war.start_event` at it for now.
    /// The war is fought for the province of the event or action if the enemy holds it,
    /// else for `war::enemy_border`. A no-op while a war goes on.
    StartWar(NeighbourTarget),
    /// A battle of the current war, see `war::clash`. Applied by `Game`, which owns the rng.
    Clash,
    /// The province changes hands as a whole; crown power follows by the formula.
    TransferProvince(ProvinceTarget, NewHolder),
    /// The treasury gets this (pays it when negative); the neighbour of the event or
    /// action loses `Data.war.tribute_strength` strength per unit.
    Tribute(Fx),
    /// The heir at this index (0 is the first heir) goes to the neighbour as a hostage.
    TakeHostage(u32, NeighbourTarget),
    /// Ends the current war; the enemy's strength changes by `Data.war.end_strength`.
    EndWar(WarOutcome),
    /// Moves the current war to this stage; the chain's steps wait for their stage.
    SetWarStage(WarStage),
    /// Applies these only while the neighbour of the event or action is friendly (relation
    /// above `neighbour_ai.friendly_above`). Applied by `Game`.
    IfFriendly(Vec<Effect>),
    /// Relation change with every neighbour but the one of the event or action.
    OtherRelations(Fx),
    /// The vassal holding the province breaks away with all his provinces as a new foreign
    /// state of his name, strength `strength * provinces`, relation `sim.secession_relation`.
    Secede(ProvinceTarget),
    /// A suit to the neighbour of the event or action: with the chance of
    /// `MarriageRules::chance` the spouse it names weds into a union with that court
    /// (`World.unions`) and `then` applies, else `otherwise`. Applied by `Game`.
    Marry {
        then: Vec<Effect>,
        #[serde(default)]
        otherwise: Vec<Effect>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NewHolder {
    Crown,
    Foreign(NeighbourTarget),
}

/// Success chance in percent: `percent + sum(axis * k) + bonus of every predicate that holds`,
/// clamped to 0..=100.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chance {
    pub percent: Fx,
    #[serde(default)]
    pub axes: Vec<(AxisId, Fx)>,
    #[serde(default)]
    pub bonus: Vec<(Predicate, Fx)>,
    pub then: Vec<Effect>,
    #[serde(default)]
    pub otherwise: Vec<Effect>,
}

impl Chance {
    pub fn percent(&self, w: &World) -> Fx {
        let axes = self.axes.iter().map(|(a, k)| w.axes[a] * *k);
        let bonus = self
            .bonus
            .iter()
            .filter(|(p, _)| p.eval(w))
            .map(|(_, b)| *b);
        let p = axes.chain(bonus).fold(self.percent, |sum, v| sum + v);
        p.clamp(Fx(0), PERCENT)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ProvinceTarget {
    Capital,
    ById(ProvinceId),
    /// The province of the event or action; no-op if it has none.
    EventTarget,
    /// `World::weakest_border` with the neighbour of the event or action.
    OwnBorder,
    /// That neighbour's province next to the kingdom closest to the capital, smallest id
    /// on a tie (`war::enemy_border`).
    EnemyBorder,
    /// The target of the current war while the enemy holds it (`War.target`).
    WarTarget,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NeighbourTarget {
    ById(NeighbourId),
    /// The neighbour of the event or action (its target, or the neighbour behind its
    /// province target); no-op if it has none.
    EventTarget,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ProvinceField {
    Income,
    /// The delta's whole part, in people.
    Population,
    Loyalty,
}

/// Indices into `World.heirs`; out of range is a no-op.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum HeirOp {
    /// Pushes `Data::newborn`.
    Add,
    Remove(u32),
    SetStatus(u32, HeirStatus),
    Ability(u32, Fx),
    Claim(u32, Fx),
    /// The same on the heir of the event or action; no-op if it has none.
    TargetStatus(HeirStatus),
    TargetAbility(Fx),
    TargetClaim(Fx),
    TargetRemove,
    /// The heir of the event or action is married from now on (`Heir.married`).
    TargetMarry,
    /// The ruler names this heir to succeed him (`World.designated`); one not rightful
    /// (`sim::rightful`) costs `heirs.designate_penalty`. Naming the named again is nothing.
    Designate(u32),
    TargetDesignate,
    /// The eldest bastard joins the line with `heirs.bastard_claim`; no-op without one.
    Recognize,
}

/// What an effect may touch besides the world.
pub struct Ctx<'a> {
    pub data: &'a Data,
    /// Deferred events with their due tick.
    pub queue: &'a mut Vec<(Tick, PendingEvent)>,
    pub target: Option<&'a Target>,
    /// The neighbour behind a province target, see `PendingEvent.neighbour`.
    pub neighbour: Option<&'a NeighbourId>,
}

impl<'a> Ctx<'a> {
    /// What `NeighbourTarget::EventTarget` means here.
    pub fn neighbour(&self) -> Option<&'a NeighbourId> {
        match self.target {
            Some(Target::Neighbour(n)) => Some(n),
            _ => self.neighbour,
        }
    }

    fn resolve<'b>(&self, t: &'b NeighbourTarget) -> Option<&'b NeighbourId>
    where
        'a: 'b,
    {
        match t {
            NeighbourTarget::ById(id) => Some(id),
            NeighbourTarget::EventTarget => self.neighbour(),
        }
    }
}

impl Effect {
    /// The caller recomputes loyalty and crown power after a batch.
    pub fn apply(&self, w: &mut World, ctx: &mut Ctx) {
        let pct = |v: Fx| v.clamp(Fx(0), PERCENT);
        match self {
            Effect::Axis(a, d) => add_axis(w, ctx.data, a, *d),
            Effect::Province(t, field, d) => {
                let id = t.resolve(w, ctx);
                let Some(p) = id.and_then(|id| w.provinces.get_mut(&id)) else {
                    return;
                };
                match field {
                    ProvinceField::Income => p.income = (p.income + *d).max(Fx(0)),
                    ProvinceField::Population => {
                        p.population = (p.population + (d.0 / Fx::SCALE) as i32).max(0)
                    }
                    ProvinceField::Loyalty => p.loyalty = pct(p.loyalty + *d),
                }
            }
            Effect::SetFlag(f) => {
                w.flags.insert(f.clone());
            }
            Effect::ClearFlag(f) => {
                w.flags.remove(f);
            }
            Effect::SpawnEvent(id, years) => {
                let due = Tick(w.tick.0 + years.ticks(w.time_unit).0);
                let p = PendingEvent {
                    event_id: id.clone(),
                    target: ctx.target.cloned(),
                    neighbour: ctx.neighbour.cloned(),
                };
                ctx.queue.push((due, p));
            }
            Effect::RulerHealth(d) => w.ruler.health = pct(w.ruler.health + *d),
            Effect::Relation(t, d) => {
                if let Some(n) = ctx.resolve(t).and_then(|id| w.neighbours.get_mut(id)) {
                    n.relation = (n.relation + *d).clamp(Fx(0) - PERCENT, PERCENT);
                }
            }
            Effect::CrownPower(t, d) => {
                if let Some(id) = t.resolve(w, ctx) {
                    let m = w.crown_modifiers.entry(id).or_default();
                    *m = *m + *d;
                }
            }
            Effect::HeirOp(op) => {
                let heir = |i: &u32| *i as usize;
                // Out of range, so a no-op, when there is no target heir.
                let t = match ctx.target {
                    Some(Target::Heir(id)) => w.heir_index(*id).map_or(u32::MAX, |i| i as u32),
                    _ => u32::MAX,
                };
                let op = match op.clone() {
                    HeirOp::TargetStatus(s) => HeirOp::SetStatus(t, s),
                    HeirOp::TargetAbility(d) => HeirOp::Ability(t, d),
                    HeirOp::TargetClaim(d) => HeirOp::Claim(t, d),
                    HeirOp::TargetRemove => HeirOp::Remove(t),
                    HeirOp::TargetDesignate => HeirOp::Designate(t),
                    op => op,
                };
                match &op {
                    HeirOp::Add => w.add_heir(ctx.data.newborn(w.next_heir_id)),
                    HeirOp::TargetMarry => {
                        let year = w.year();
                        if let Some(h) = w.heirs.get_mut(heir(&t)) {
                            (h.married, h.married_in) = (true, Some(year));
                        }
                    }
                    HeirOp::Recognize if !w.bastards.is_empty() => {
                        let mut h = w.bastards.remove(0);
                        h.claim = ctx.data.heirs.bastard_claim;
                        w.insert_heir(h);
                    }
                    HeirOp::Recognize => {}
                    HeirOp::Remove(i) if heir(i) < w.heirs.len() => {
                        w.heirs.remove(heir(i));
                    }
                    HeirOp::SetStatus(i, s) => {
                        if let Some(h) = w.heirs.get_mut(heir(i)) {
                            h.status = s.clone();
                        }
                    }
                    HeirOp::Ability(i, d) => {
                        if let Some(h) = w.heirs.get_mut(heir(i)) {
                            h.ability = pct(h.ability + *d);
                        }
                    }
                    HeirOp::Claim(i, d) => {
                        if let Some(h) = w.heirs.get_mut(heir(i)) {
                            h.claim = pct(h.claim + *d);
                        }
                    }
                    HeirOp::Designate(i) => {
                        let Some(id) = w.heirs.get(heir(i)).map(|h| h.id) else {
                            return;
                        };
                        if w.designated == Some(id) {
                            return;
                        }
                        w.designated = Some(id);
                        if crate::sim::rightful(w, ctx.data) != Some(heir(i)) {
                            for (a, v) in &ctx.data.heirs.designate_penalty {
                                add_axis(w, ctx.data, a, *v);
                            }
                        }
                    }
                    HeirOp::Remove(_) => {}
                    HeirOp::TargetStatus(_)
                    | HeirOp::TargetAbility(_)
                    | HeirOp::TargetClaim(_)
                    | HeirOp::TargetRemove
                    | HeirOp::TargetDesignate => {
                        unreachable!("resolved above")
                    }
                }
            }
            Effect::RulerDies(_)
            | Effect::Abdicate
            | Effect::Marry { .. }
            | Effect::Chance(_)
            | Effect::Clash
            | Effect::IfFriendly(_) => {
                unreachable!("Game applies these")
            }
            Effect::StartWar(t) => {
                let Some(enemy) = ctx.resolve(t).filter(|n| w.neighbours.contains_key(*n)) else {
                    return;
                };
                if w.war.is_some() {
                    return;
                }
                let (ours, theirs) = crate::war::strengths(w, ctx.data, enemy);
                let theirs_here = |id: &&ProvinceId| {
                    (w.provinces.get(*id))
                        .is_some_and(|p| p.holder == Holder::Foreign(enemy.clone()))
                };
                let target = match ctx.target {
                    Some(Target::Province(id)) => Some(id).filter(theirs_here).cloned(),
                    _ => None,
                };
                let target = target.or_else(|| crate::war::enemy_border(w, enemy));
                // A war ends the union with that court.
                w.unions.remove(enemy);
                w.war = Some(War {
                    enemy: enemy.clone(),
                    stage: WarStage::Declared,
                    our_strength: ours,
                    their_strength: theirs,
                    war_score: Fx(0),
                    started: w.tick,
                    target,
                    battles: Vec::new(),
                });
                let p = PendingEvent {
                    event_id: ctx.data.war.start_event.clone(),
                    target: Some(Target::Neighbour(enemy.clone())),
                    neighbour: None,
                };
                ctx.queue.push((w.tick, p));
            }
            Effect::TransferProvince(t, to) => {
                let holder = match to {
                    NewHolder::Crown => Some(Holder::Crown),
                    NewHolder::Foreign(n) => ctx.resolve(n).map(|n| Holder::Foreign(n.clone())),
                };
                let id = t.resolve(w, ctx);
                if let (Some(p), Some(h)) = (id.and_then(|id| w.provinces.get_mut(&id)), holder) {
                    p.holder = h;
                }
            }
            Effect::Tribute(v) => {
                add_axis(w, ctx.data, &ctx.data.economy.treasury, *v);
                let n = ctx.neighbour().and_then(|n| w.neighbours.get_mut(n));
                if let Some(n) = n {
                    n.strength = (n.strength - *v * ctx.data.war.tribute_strength).max(Fx(0));
                }
            }
            Effect::TakeHostage(i, t) => {
                let n = ctx.resolve(t).filter(|n| w.neighbours.contains_key(*n));
                if let (Some(h), Some(n)) = (w.heirs.get_mut(*i as usize), n) {
                    h.status = HeirStatus::Hostage(n.clone());
                }
            }
            Effect::OtherRelations(d) => {
                let target = ctx.neighbour();
                for n in w.neighbours.values_mut().filter(|n| Some(&n.id) != target) {
                    n.relation = (n.relation + *d).clamp(Fx(0) - PERCENT, PERCENT);
                }
            }
            Effect::SetWarStage(stage) => {
                if let Some(war) = &mut w.war {
                    war.stage = stage.clone();
                }
            }
            Effect::EndWar(outcome) => {
                let Some(war) = w.war.take() else {
                    return;
                };
                let mut ends = ctx.data.war.end_strength.iter();
                let d = ends.find(|(o, _)| o == outcome).map_or(Fx(0), |(_, d)| *d);
                if let Some(n) = w.neighbours.get_mut(&war.enemy) {
                    n.strength = (n.strength + d).max(Fx(0));
                }
            }
            Effect::Build(t, b) => {
                let id = t.resolve(w, ctx);
                if let Some(p) = id.and_then(|id| w.provinces.get_mut(&id)) {
                    p.buildings.insert(b.clone());
                }
            }
            Effect::Grant(t) => {
                let Some(id) = t.resolve(w, ctx) else {
                    return;
                };
                if w.provinces
                    .get(&id)
                    .is_none_or(|p| p.holder != Holder::Crown)
                {
                    return;
                }
                let g = &ctx.data.grant;
                let hops = w.hops(&id);
                let nearest = (w.provinces.values())
                    .filter_map(|p| match &p.holder {
                        Holder::Vassal(v) => Some((*hops.get(&p.id)?, v.clone())),
                        _ => None,
                    })
                    .min();
                let names = ctx.data.names.vassals.iter();
                let fresh = names.map(|n| VassalId(n.clone())).find(|id| {
                    !w.vassals.contains_key(id) && w.vassals.values().all(|v| v.name != id.0)
                });
                // A house that gets more land grows stronger by `strength`.
                let grow = |w: &mut World, v: VassalId| {
                    if let Some(house) = w.vassals.get_mut(&v) {
                        house.strength = house.strength + g.strength;
                    }
                    v
                };
                let vassal = match (nearest, fresh) {
                    (Some((d, v)), _) if d <= g.max_distance => grow(w, v),
                    (_, Some(id)) => {
                        let v = Vassal {
                            id: id.clone(),
                            name: id.0.clone(),
                            loyalty: g.new_loyalty,
                            strength: g.new_strength,
                        };
                        w.vassals.insert(id.clone(), v);
                        id
                    }
                    (Some((_, v)), None) => grow(w, v),
                    (None, None) => return,
                };
                w.provinces.get_mut(&id).expect("checked above").holder = Holder::Vassal(vassal);
            }
            Effect::Secede(t) => {
                let held = t.resolve(w, ctx).and_then(|id| w.provinces.get(&id));
                let Some(Holder::Vassal(v)) = held.map(|p| p.holder.clone()) else {
                    return;
                };
                let holder = Holder::Vassal(v.clone());
                let Some(vassal) = w.vassals.remove(&v) else {
                    return;
                };
                // ponytail: a later house of the same name would merge into this state.
                let id = NeighbourId(v.0);
                let mut count = 0;
                for p in w.provinces.values_mut().filter(|p| p.holder == holder) {
                    p.holder = Holder::Foreign(id.clone());
                    count += 1;
                }
                let n = Neighbour {
                    id: id.clone(),
                    name: vassal.name,
                    relation: ctx.data.sim.secession_relation,
                    strength: vassal.strength * Fx::from_int(count),
                    stance: Stance::Defend,
                    per_province: vassal.strength,
                    ordinal: (w.neighbours.values().map(|n| n.ordinal + 1).max()).unwrap_or(0),
                };
                w.neighbours.insert(id, n);
            }
            Effect::Revoke(t) => {
                let id = t.resolve(w, ctx);
                if let Some(p) = id.and_then(|id| w.provinces.get_mut(&id))
                    && matches!(p.holder, Holder::Vassal(_))
                {
                    p.holder = Holder::Crown;
                }
            }
        }
    }

    fn check(&self, data: &Data) -> Result<(), String> {
        match self {
            Effect::Axis(a, _) if data.is_derived(a) => {
                Err(format!("axis {} is derived, effects cannot write it", a.0))
            }
            Effect::Axis(a, _) => known_axis(data, a),
            Effect::Chance(c) => {
                c.axes.iter().try_for_each(|(a, _)| known_axis(data, a))?;
                c.bonus.iter().try_for_each(|(p, _)| p.check(data))?;
                c.then
                    .iter()
                    .chain(&c.otherwise)
                    .try_for_each(|e| e.check(data))
            }
            Effect::IfFriendly(es) => es.iter().try_for_each(|e| e.check(data)),
            Effect::Marry { then, otherwise } => {
                then.iter().chain(otherwise).try_for_each(|e| e.check(data))
            }
            _ => Ok(()),
        }
    }
}

impl ProvinceTarget {
    fn resolve(&self, w: &World, ctx: &Ctx) -> Option<ProvinceId> {
        match (self, ctx.target, ctx.neighbour()) {
            (ProvinceTarget::Capital, ..) => Some(w.capital.province.clone()),
            (ProvinceTarget::ById(id), ..) => Some(id.clone()),
            (ProvinceTarget::EventTarget, Some(Target::Province(id)), _) => Some(id.clone()),
            (ProvinceTarget::OwnBorder, _, Some(n)) => w.weakest_border(n).map(|p| p.id.clone()),
            (ProvinceTarget::EnemyBorder, _, Some(n)) => crate::war::enemy_border(w, n),
            (ProvinceTarget::WarTarget, ..) => {
                let war = w.war.as_ref()?;
                let id = war.target.as_ref()?;
                let held = w.provinces.get(id)?.holder == Holder::Foreign(war.enemy.clone());
                held.then(|| id.clone())
            }
            _ => None,
        }
    }
}

/// Adds `d` to the axis, clamped to its bounds from `rules.ron`. The derived stability
/// takes it as a shock (`graph::Stability`).
pub(crate) fn add_axis(w: &mut World, data: &Data, id: &AxisId, d: Fx) {
    if let Some(s) = data.stability.as_ref().filter(|s| s.axis == *id) {
        add_axis(w, data, &s.shocks, d);
        return crate::graph::recompute_stability(data, w);
    }
    let def = data
        .axes
        .iter()
        .find(|a| a.id == *id)
        .expect("axes are checked on load");
    let v = w.axes.get_mut(id).expect("the world has every axis");
    *v = (*v + d).clamp(def.min, def.max);
}

fn known_axis(data: &Data, a: &AxisId) -> Result<(), String> {
    match data.axes.iter().any(|d| d.id == *a) {
        true => Ok(()),
        false => Err(format!("unknown axis {}", a.0)),
    }
}

/// Texts may contain `{province}`, `{neighbour}`, `{heir}`, `{ruler}`, `{vassal}`,
/// `{war_target}`; `Game` fills them in for display.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub text: String,
    pub when: Predicate,
    /// 0 keeps the event out of the random pool: it only fires via `SpawnEvent`.
    pub weight: u32,
    /// Added to the weight in the pick while the predicate holds (also to an offer's weight
    /// from `neighbour_ai`): risks that grow with a bad state. Weight 0 stays out of the pool.
    #[serde(default)]
    pub weight_bonus: Vec<(Predicate, u32)>,
    /// Also added to the weight: this curve (`data::curve`, points `(ratio, weight)`) at the
    /// highest `World::vassal_ratio` of any vassal province. Revolt grows as a vassal nears
    /// the crown's strength.
    #[serde(default)]
    pub vassal_weight: Vec<(Fx, Fx)>,
    pub once: bool,
    pub cooldown_years: Years,
    pub importance: u32,
    /// Good or bad for the dynasty; the score reads it. Defaults to `Bad`.
    #[serde(default)]
    pub sign: Sign,
    pub target: EventTarget,
    pub choices: Vec<Choice>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub enum Sign {
    Good,
    #[default]
    Bad,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum EventTarget {
    None,
    /// The event cannot fire while no province matches.
    RandomProvince(ProvinceFilter),
    Neighbour,
    /// A random heir aged within this inclusive range; cannot fire while there is none.
    Heir(u32, u32),
    /// The same among the heirs not yet married.
    UnmarriedHeir(u32, u32),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Choice {
    pub text: String,
    pub effects: Vec<Effect>,
    pub cause_tag: String,
    pub hint: Option<String>,
}

impl Event {
    /// Sum of the `weight_bonus` that hold now.
    pub fn bonus(&self, w: &World) -> u32 {
        let holding = self.weight_bonus.iter().filter(|(p, _)| p.eval(w));
        let flat: u32 = holding.map(|(_, b)| b).sum();
        if self.vassal_weight.is_empty() {
            return flat;
        }
        let ratios = w.provinces.values().filter_map(|p| w.vassal_ratio(p));
        let Some(top) = ratios.max() else {
            return flat;
        };
        let curve = crate::data::curve(&self.vassal_weight, top);
        flat + (curve.0 / Fx::SCALE).max(0) as u32
    }

    pub(crate) fn check(&self, data: &Data) -> Result<(), String> {
        if self.choices.is_empty() {
            return Err("an event needs at least one choice".into());
        }
        self.when.check(data)?;
        self.weight_bonus
            .iter()
            .try_for_each(|(p, _)| p.check(data))?;
        let mut effects = self.choices.iter().flat_map(|c| &c.effects);
        effects.try_for_each(|e| e.check(data))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Action {
    pub id: String,
    pub name: String,
    pub duration_years: Years,
    /// Paid from the treasury on start.
    pub cost: Fx,
    pub requires: Predicate,
    /// Against the target province, or the capital for actions without one.
    pub min_crown_power: Fx,
    pub target: ActionTarget,
    pub on_complete: Vec<Effect>,
    /// Applied once a year while the action runs, its last year included, without a target:
    /// e.g. a faction's resistance to a new law.
    #[serde(default)]
    pub yearly: Vec<Effect>,
    pub cause_tag: String,
    /// What the action does, in plain words, for the UI tooltip.
    #[serde(default)]
    pub description: String,
    /// For an action that marries (`Effect::Marry`): what a union is to the crown
    /// (`Game::bonds`), e.g. «брачный союз». Empty: no bond.
    #[serde(default)]
    pub bond: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ActionTarget {
    None,
    Province(ProvinceFilter),
    Neighbour,
    Heir,
    /// The enemy of the current war; no target, so unavailable, in peace.
    Enemy,
}

impl Action {
    /// A suit: its `on_complete` has `Effect::Marry`.
    pub fn marries(&self) -> bool {
        (self.on_complete.iter()).any(|e| matches!(e, Effect::Marry { .. }))
    }

    pub(crate) fn check(&self, data: &Data) -> Result<(), String> {
        self.requires.check(data)?;
        let effects = self.on_complete.iter().chain(&self.yearly);
        effects.into_iter().try_for_each(|e| e.check(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Heir, Preset};

    fn world() -> (Data, World) {
        let data = crate::data::load(include_str!("../../../data/rules.ron")).unwrap();
        let preset = include_str!("../../../data/presets/default.ron");
        let map = include_str!("../../../data/maps/default.ron");
        let preset = Preset::load_with_map(preset, map, &data).unwrap();
        let world = World::from_preset(&data, &preset);
        (data, world)
    }

    fn ax(s: &str) -> AxisId {
        AxisId(s.into())
    }

    fn pid(s: &str) -> ProvinceId {
        ProvinceId(s.into())
    }

    fn filter(text: &str) -> ProvinceFilter {
        crate::data::parse(text).unwrap()
    }

    #[test]
    fn predicates() {
        let (_, mut w) = world();
        w.flags.insert("married".into());
        let p = |text: &str| crate::data::parse::<Predicate>(text).unwrap().eval(&w);
        // legitimacy is 45.
        assert!(p(r#"AxisAbove("legitimacy", 44.999)"#));
        assert!(!p(r#"AxisAbove("legitimacy", 45)"#));
        assert!(p(r#"AxisBelow("legitimacy", 45.001)"#));
        assert!(!p(r#"AxisBelow("legitimacy", 45)"#));
        assert!(p(r#"Flag("married")"#) && !p(r#"Flag("plague")"#));
        assert!(p(r#"NotFlag("plague")"#) && !p(r#"NotFlag("married")"#));
        assert!(p("ProvinceWhere((holder: Vassal))"));
        assert!(!p("ProvinceWhere((holder: Vassal, loyalty_above: 45))"));
        // One heir, ruler 32.
        assert!(p("HeirCount(1, 1)") && !p("HeirCount(2, 9)") && !p("HeirCount(0, 0)"));
        assert!(p("RulerAge(32, 32)") && !p("RulerAge(33, 99)"));
        assert!(!p("AtWar"));
        assert!(p("All([])") && !p(r#"All([Flag("married"), Flag("plague")])"#));
        assert!(p(r#"Any([Flag("plague"), Flag("married")])"#) && !p("Any([])"));
        assert!(p(r#"Not(Flag("plague"))"#) && !p("Not(All([]))"));
    }

    #[test]
    fn province_filter() {
        let (_, w) = world();
        let matching = |text: &str| {
            let f = filter(text);
            let ids = w.provinces.values().filter(|p| f.matches(p, &w));
            ids.map(|p| p.id.0.as_str()).collect::<Vec<_>>()
        };
        let crown = ["berg", "capital", "gart", "lugovo", "ostwick", "sol"];
        assert_eq!(matching("()").len(), 20);
        assert_eq!(matching("(holder: Crown)"), crown);
        assert_eq!(matching("(holder: Foreign)").len(), 10);
        assert_eq!(
            matching("(loyalty_below: 50)"),
            ["arden", "holm", "mar", "weir"]
        );
        assert_eq!(matching("(loyalty_above: 50)"), crown);
        assert_eq!(matching(r#"(building: "fort")"#), ["capital"]);
        assert_eq!(matching(r#"(without_building: "fort")"#).len(), 19);
        // Own border provinces, plus where two neighbours touch: skala and porfir,
        // frostad and vestburg.
        assert_eq!(
            matching("(borders_foreign: true)"),
            [
                "arden", "berg", "frostad", "gart", "holm", "lugovo", "mar", "ostwick", "porfir",
                "skala", "sol", "vestburg", "weir"
            ]
        );
        let inner = matching("(borders_foreign: false)");
        assert!(
            ["capital", "kirm", "nordheim"]
                .iter()
                .all(|p| inner.contains(p))
        );
        assert_eq!(matching("(capital: true)"), ["capital"]);
        assert_eq!(matching("(capital: false)").len(), 19);
    }

    #[test]
    fn effects() {
        let (data, mut w) = world();
        let mut queue = Vec::new();
        let holm = Target::Province(pid("holm"));
        let nordmark = Target::Neighbour(NeighbourId("nordmark".into()));
        let mut run = |w: &mut World, text: &str, target: Option<&Target>| {
            let e: Effect = crate::data::parse(text).unwrap();
            e.apply(
                w,
                &mut Ctx {
                    data: &data,
                    queue: &mut queue,
                    target,
                    neighbour: None,
                },
            );
        };
        run(&mut w, r#"Axis("legitimacy", 5.5)"#, None);
        assert_eq!(w.axes[&ax("legitimacy")], Fx(50_500));
        run(&mut w, r#"Axis("legitimacy", 1000)"#, None);
        assert_eq!(w.axes[&ax("legitimacy")], PERCENT); // clamped to the axis max

        run(&mut w, "Province(Capital, Income, -100)", None);
        assert_eq!(w.provinces[&pid("capital")].income, Fx(0));
        run(
            &mut w,
            r#"Province(ById("holm"), Population, -1000.5)"#,
            None,
        );
        assert_eq!(w.provinces[&pid("holm")].population, 8000);
        run(&mut w, "Province(EventTarget, Loyalty, 100)", Some(&holm));
        assert_eq!(w.provinces[&pid("holm")].loyalty, PERCENT);
        let before = w.clone();
        run(&mut w, "Province(EventTarget, Loyalty, 5)", None);
        run(&mut w, "Province(EventTarget, Loyalty, 5)", Some(&nordmark));
        assert_eq!(w, before); // no province target: no-op

        run(&mut w, r#"SetFlag("plague")"#, None);
        assert!(w.flags.contains("plague"));
        run(&mut w, r#"ClearFlag("plague")"#, None);
        assert!(!w.flags.contains("plague"));

        run(&mut w, "RulerHealth(-500)", None);
        assert_eq!(w.ruler.health, Fx(0));

        let relation = |w: &World| w.neighbours[&NeighbourId("nordmark".into())].relation;
        run(&mut w, r#"Relation(ById("nordmark"), 30)"#, None);
        run(&mut w, "Relation(EventTarget, 20)", Some(&nordmark));
        assert_eq!(relation(&w), Fx::from_int(10)); // from -40
        run(&mut w, "Relation(EventTarget, -900)", Some(&nordmark));
        assert_eq!(relation(&w), Fx::from_int(-100));
        run(&mut w, "Relation(EventTarget, 20)", Some(&holm));
        assert_eq!(relation(&w), Fx::from_int(-100));

        run(&mut w, "CrownPower(EventTarget, 7)", Some(&holm));
        assert_eq!(w.crown_modifiers[&pid("holm")], Fx::from_int(7));

        run(&mut w, "HeirOp(Add)", None);
        assert_eq!(w.heirs.len(), 2);
        // The newborn gets the next id; the preset's heir has 0.
        assert_eq!((w.heirs[0].id, w.next_heir_id), (0, 2));
        assert_eq!(
            w.heirs[1],
            Heir {
                id: 1,
                ..data.new_heir.clone()
            }
        );
        run(&mut w, "HeirOp(Remove(0))", None);
        assert_eq!(w.heirs[0].name, "Младенец");
        run(&mut w, r#"HeirOp(SetStatus(0, Hostage("nordmark")))"#, None);
        assert_eq!(
            w.heirs[0].status,
            HeirStatus::Hostage(NeighbourId("nordmark".into()))
        );
        run(&mut w, "HeirOp(Ability(0, 10))", None);
        assert_eq!(w.heirs[0].ability, Fx::from_int(60));
        let before = w.clone();
        run(&mut w, "HeirOp(Remove(9))", None);
        run(&mut w, "HeirOp(Ability(9, 1))", None);
        run(&mut w, "HeirOp(SetStatus(9, Home))", None);
        assert_eq!(w, before);

        w.tick = Tick(3);
        run(&mut w, r#"SpawnEvent("next", 2)"#, None);
        drop(run);
        let next = PendingEvent {
            event_id: "next".into(),
            target: None,
            neighbour: None,
        };
        assert_eq!(queue, [(Tick(5), next)]);
    }

    #[test]
    fn map_effects() {
        let (mut data, mut w) = world();
        let run = |w: &mut World, data: &Data, text: &str, province: &str| {
            let e: Effect = crate::data::parse(text).unwrap();
            let target = Target::Province(pid(province));
            let mut queue = Vec::new();
            let mut ctx = Ctx {
                data,
                queue: &mut queue,
                target: Some(&target),
                neighbour: None,
            };
            e.apply(w, &mut ctx);
        };
        let holder = |w: &World, p: &str| w.provinces[&pid(p)].holder.clone();
        let vassal = |v: &str| Holder::Vassal(VassalId(v.into()));

        run(&mut w, &data, r#"Build(EventTarget, "market")"#, "berg");
        assert!(w.provinces[&pid("berg")].buildings.contains("market"));

        // Next to holm (weir) at one crossing; stage 15: the house grows by grant.strength.
        let weir = |w: &World| w.vassals[&VassalId("weir".into())].strength;
        let before = weir(&w);
        run(&mut w, &data, "Grant(EventTarget)", "gart");
        assert_eq!(holder(&w, "gart"), vassal("weir"));
        assert_eq!(weir(&w), before + data.grant.strength);
        assert!(data.grant.strength > Fx(0));
        // Arden and Weir both border the capital: the smaller id wins the tie.
        run(&mut w, &data, "Grant(ById(\"capital\"))", "gart");
        assert_eq!(holder(&w, "capital"), vassal("arden"));
        // Not a crown province: no-op.
        run(&mut w, &data, "Grant(EventTarget)", "nordheim");
        assert_eq!(
            holder(&w, "nordheim"),
            Holder::Foreign(NeighbourId("nordmark".into()))
        );

        // Nobody close enough: a new house from the name pool, then the next one.
        // Вейр is taken (by name, under the id "weir"), so it is skipped.
        data.grant.max_distance = 0;
        data.names.vassals = ["Вейр", "Ростен", "Ольбек"].map(String::from).to_vec();
        run(&mut w, &data, "Grant(EventTarget)", "sol");
        assert_eq!(holder(&w, "sol"), vassal("Ростен"));
        let rosten = &w.vassals[&VassalId("Ростен".into())];
        assert_eq!(rosten.name, "Ростен");
        assert_eq!(rosten.loyalty, data.grant.new_loyalty);
        assert_eq!(rosten.strength, data.grant.new_strength);
        run(&mut w, &data, "Grant(EventTarget)", "berg");
        assert_eq!(holder(&w, "berg"), vassal("Ольбек"));
        // The pool is used up: the nearest vassal at any distance.
        data.names.vassals.clear();
        run(&mut w, &data, "Grant(EventTarget)", "lugovo");
        assert_eq!(holder(&w, "lugovo"), vassal("arden")); // capital and arden at 1
        // No vassal anywhere: no-op.
        for p in w.provinces.values_mut() {
            if matches!(p.holder, Holder::Vassal(_)) {
                p.holder = Holder::Crown;
            }
        }
        run(&mut w, &data, "Grant(EventTarget)", "ostwick");
        assert_eq!(holder(&w, "ostwick"), Holder::Crown);

        w.provinces.get_mut(&pid("holm")).unwrap().holder = vassal("weir");
        run(&mut w, &data, "Revoke(EventTarget)", "holm");
        assert_eq!(holder(&w, "holm"), Holder::Crown);
        run(&mut w, &data, "Revoke(EventTarget)", "nordheim");
        assert_eq!(
            holder(&w, "nordheim"),
            Holder::Foreign(NeighbourId("nordmark".into()))
        );
    }

    #[test]
    fn vassal_stronger_than_the_crown() {
        let (data, mut w) = world();
        let f = filter("(vassal_stronger: true)");
        let stronger = |w: &World| {
            let ps = w.provinces.values().filter(|p| f.matches(p, w));
            ps.map(|p| p.id.0.clone()).collect::<Vec<_>>()
        };
        // Weir holds holm (crown power 25) and weir: his strength counts twice.
        let weir = VassalId("weir".into());
        w.vassals.get_mut(&weir).unwrap().strength = Fx::from_int(12);
        assert!(!stronger(&w).contains(&"holm".to_string())); // 24 vs 25
        w.vassals.get_mut(&weir).unwrap().strength = Fx(12_501);
        assert!(stronger(&w).contains(&"holm".to_string())); // 25.002 vs 25
        // A third province makes him stronger still.
        w.vassals.get_mut(&weir).unwrap().strength = Fx::from_int(9);
        assert!(!stronger(&w).contains(&"holm".to_string())); // 18 vs 25
        w.provinces.get_mut(&pid("gart")).unwrap().holder = Holder::Vassal(weir);
        w.recompute_crown_power(&data);
        assert!(stronger(&w).contains(&"holm".to_string())); // 27 vs 25
        // Crown land never matches; `false` matches the rest.
        assert!(!stronger(&w).contains(&"capital".to_string()));
        let weaker = filter("(vassal_stronger: false)");
        assert!(weaker.matches(&w.provinces[&pid("capital")], &w));
    }

    #[test]
    fn vassal_ratio_grows_with_the_house() {
        let (_, mut w) = world();
        // The houses at strength 20, as in the preset before stage 15.
        w.vassals
            .values_mut()
            .for_each(|v| v.strength = Fx::from_int(20));
        let ratio = |w: &World, id: &str| w.vassal_ratio(&w.provinces[&pid(id)]);
        let above = |w: &World, v: &str, id: &str| {
            filter(&format!("(vassal_ratio_above: {v})")).matches(&w.provinces[&pid(id)], w)
        };
        // Weir (strength 20) holds holm (crown power 25) and weir: 20 * 2 / 25.
        assert_eq!(ratio(&w, "holm"), Some(Fx(1_600)));
        assert!(above(&w, "1.599", "holm") && !above(&w, "1.6", "holm"));
        w.vassals
            .get_mut(&VassalId("weir".into()))
            .unwrap()
            .strength = Fx::from_int(10);
        assert_eq!(ratio(&w, "holm"), Some(Fx(800)));
        // Crown land has no ratio and never matches.
        assert_eq!(ratio(&w, "capital"), None);
        assert!(!above(&w, "0", "capital"));
        // The revolt weight follows the curve at the highest ratio: mar, 20 * 2 / 12.5 = 3.2.
        let mut e: Event = crate::data::parse(
            r#"(id: "e", title: "", text: "", when: All([]), weight: 1, once: false,
            cooldown_years: 0, importance: 0, target: None, choices: [],
            weight_bonus: [(Flag("plague"), 2)], vassal_weight: [(1, 0), (3, 20)])"#,
        )
        .unwrap();
        assert_eq!(e.bonus(&w), 20); // flat beyond the last point
        w.vassals
            .get_mut(&VassalId("arden".into()))
            .unwrap()
            .strength = Fx::from_int(10);
        assert_eq!(e.bonus(&w), 6); // mar 1.6: 20 * 0.6 / 2
        w.flags.insert("plague".into());
        assert_eq!(e.bonus(&w), 8);
        e.vassal_weight.clear();
        assert_eq!(e.bonus(&w), 2);
        // No vassal left: only the flat bonus.
        e.vassal_weight = vec![(Fx(0), Fx::from_int(50))];
        for p in w.provinces.values_mut() {
            if matches!(p.holder, Holder::Vassal(_)) {
                p.holder = Holder::Crown;
            }
        }
        assert_eq!(e.bonus(&w), 2);
    }

    #[test]
    fn secede() {
        let (data, mut w) = world();
        w.vassals
            .values_mut()
            .for_each(|v| v.strength = Fx::from_int(20));
        let mut queue = Vec::new();
        apply(
            &mut w,
            &data,
            &mut queue,
            "Secede(ById(\"holm\"))",
            None,
            None,
        );
        let weir = NeighbourId("weir".into());
        for p in ["holm", "weir"] {
            assert_eq!(
                w.provinces[&pid(p)].holder,
                Holder::Foreign(weir.clone()),
                "{p}"
            );
        }
        assert!(!w.vassals.contains_key(&VassalId("weir".into())));
        let n = &w.neighbours[&weir];
        assert_eq!((n.name.as_str(), n.strength), ("Вейр", Fx::from_int(40)));
        // Two provinces of a house of strength 20: it recovers toward 20 per province held.
        assert_eq!(n.per_province, Fx::from_int(20));
        assert_eq!(
            (n.relation, &n.stance),
            (data.sim.secession_relation, &Stance::Defend)
        );
        // Crown or foreign land: no-op.
        let before = w.clone();
        apply(&mut w, &data, &mut queue, "Secede(Capital)", None, None);
        apply(
            &mut w,
            &data,
            &mut queue,
            "Secede(ById(\"holm\"))",
            None,
            None,
        );
        assert_eq!(w, before);
    }

    #[test]
    fn weight_bonus_counts_while_it_holds() {
        let (_, mut w) = world();
        let e: Event = crate::data::parse(
            r#"(id: "e", title: "", text: "", when: All([]), weight: 2, once: false,
            cooldown_years: 0, importance: 0, target: None, choices: [],
            weight_bonus: [(AxisBelow("stability", 50), 3), (Flag("plague"), 10)])"#,
        )
        .unwrap();
        assert_eq!(e.bonus(&w), 0); // stability 55
        w.axes.insert(ax("stability"), Fx::from_int(40));
        assert_eq!(e.bonus(&w), 3);
        w.flags.insert("plague".into());
        assert_eq!(e.bonus(&w), 13);
    }

    fn apply(
        w: &mut World,
        data: &Data,
        queue: &mut Vec<(Tick, PendingEvent)>,
        text: &str,
        target: Option<&Target>,
        neighbour: Option<&NeighbourId>,
    ) {
        let e: Effect = crate::data::parse(text).unwrap();
        e.apply(
            w,
            &mut Ctx {
                data,
                queue,
                target,
                neighbour,
            },
        );
    }

    #[test]
    fn war_predicates() {
        let (_, mut w) = world();
        let p = |w: &World, text: &str| crate::data::parse::<Predicate>(text).unwrap().eval(w);
        let all = [
            "AtWar",
            "WarScoreAbove(-100)",
            "WarScoreBelow(100)",
            "WarStage(Declared)",
        ];
        assert!(all.iter().all(|t| !p(&w, t)), "no war: all false");
        w.war = Some(War {
            enemy: NeighbourId("nordmark".into()),
            stage: WarStage::Fighting,
            our_strength: Fx(0),
            their_strength: Fx(0),
            war_score: Fx::from_int(10),
            started: Tick(0),
            target: None,
            battles: vec![],
        });
        assert!(p(&w, "AtWar") && p(&w, "WarStage(Fighting)") && !p(&w, "WarStage(Peace)"));
        assert!(p(&w, "WarScoreAbove(9.999)") && !p(&w, "WarScoreAbove(10)"));
        assert!(p(&w, "WarScoreBelow(10.001)") && !p(&w, "WarScoreBelow(10)"));
    }

    #[test]
    fn war_effects() {
        let (data, mut w) = world();
        let mut queue = Vec::new();
        let nordmark = NeighbourId("nordmark".into());
        let at_nordmark = Target::Neighbour(nordmark.clone());
        let holm = Target::Province(pid("holm"));
        let holder = |w: &World, p: &str| w.provinces[&pid(p)].holder.clone();
        let strength = |w: &World| w.neighbours[&nordmark].strength;
        // No neighbour to act on: no-ops.
        let before = w.clone();
        for text in [
            "StartWar(EventTarget)",
            "TransferProvince(EnemyBorder, Crown)",
            "TransferProvince(OwnBorder, Crown)",
            "TakeHostage(0, EventTarget)",
            "EndWar(Victory)",
            "SetWarStage(Peace)",
        ] {
            apply(&mut w, &data, &mut queue, text, Some(&holm), None);
        }
        assert_eq!(w, before);
        assert!(queue.is_empty());

        apply(
            &mut w,
            &data,
            &mut queue,
            "StartWar(EventTarget)",
            Some(&at_nordmark),
            None,
        );
        let war = w.war.clone().unwrap();
        assert_eq!(
            (&war.enemy, &war.stage, war.war_score),
            (&nordmark, &WarStage::Declared, Fx(0))
        );
        assert_eq!(
            (war.our_strength, war.their_strength),
            (Fx::from_int(60), Fx::from_int(60))
        );
        // One war at a time: the second is refused and queues nothing.
        apply(
            &mut w,
            &data,
            &mut queue,
            r#"StartWar(ById("purpur"))"#,
            None,
            None,
        );
        assert_eq!(w.war, Some(war));
        let declared = PendingEvent {
            event_id: "war_declared".into(),
            target: Some(at_nordmark.clone()),
            neighbour: None,
        };
        assert_eq!(queue, [(Tick(0), declared)]);

        apply(&mut w, &data, &mut queue, "SetWarStage(Peace)", None, None);
        assert_eq!(w.war.as_ref().unwrap().stage, WarStage::Peace);

        // Nordmark's provinces next to the kingdom are all two crossings away: frostad by id.
        apply(
            &mut w,
            &data,
            &mut queue,
            "TransferProvince(EnemyBorder, Crown)",
            Some(&at_nordmark),
            None,
        );
        assert_eq!(holder(&w, "frostad"), Holder::Crown);
        w.recompute_crown_power(&data);
        assert_eq!(w.provinces[&pid("frostad")].crown_power, Fx::from_int(50)); // 60 - 2 * 5
        // The war, declared at Nordmark itself, is for that same default province; the enemy
        // no longer holds it, so the war's target is nothing to take.
        assert_eq!(w.war.as_ref().unwrap().target, Some(pid("frostad")));
        let before = w.clone();
        let text = "TransferProvince(WarTarget, Foreign(EventTarget))";
        apply(&mut w, &data, &mut queue, text, Some(&at_nordmark), None);
        assert_eq!(w, before);
        // The neighbour behind a province target counts: arden is the weakest on its border.
        let text = "TransferProvince(OwnBorder, Foreign(EventTarget))";
        apply(
            &mut w,
            &data,
            &mut queue,
            text,
            Some(&holm),
            Some(&nordmark),
        );
        assert_eq!(holder(&w, "arden"), Holder::Foreign(nordmark.clone()));
        let text = r#"TransferProvince(ById("berg"), Foreign(ById("purpur")))"#;
        apply(&mut w, &data, &mut queue, text, None, None);
        assert_eq!(
            holder(&w, "berg"),
            Holder::Foreign(NeighbourId("purpur".into()))
        );

        // Paying 50 makes Nordmark 5 stronger.
        apply(
            &mut w,
            &data,
            &mut queue,
            "Tribute(-50)",
            Some(&at_nordmark),
            None,
        );
        assert_eq!(w.axes[&ax("treasury")], Fx::from_int(100));
        assert_eq!(strength(&w), Fx::from_int(65));

        apply(
            &mut w,
            &data,
            &mut queue,
            "TakeHostage(0, EventTarget)",
            Some(&at_nordmark),
            None,
        );
        assert_eq!(w.heirs[0].status, HeirStatus::Hostage(nordmark.clone()));
        let before = w.clone();
        apply(
            &mut w,
            &data,
            &mut queue,
            "TakeHostage(1, EventTarget)",
            Some(&at_nordmark),
            None,
        );
        assert_eq!(w, before, "no second heir");

        // Victory: Nordmark loses 10.
        apply(&mut w, &data, &mut queue, "EndWar(Victory)", None, None);
        assert_eq!((w.war.clone(), strength(&w)), (None, Fx::from_int(55)));
        // Strength never goes below 0.
        apply(
            &mut w,
            &data,
            &mut queue,
            "Tribute(1000)",
            Some(&at_nordmark),
            None,
        );
        assert_eq!(strength(&w), Fx(0));

        // A deferred event keeps both targets of what spawned it.
        queue.clear();
        apply(
            &mut w,
            &data,
            &mut queue,
            r#"SpawnEvent("next", 1)"#,
            Some(&holm),
            Some(&nordmark),
        );
        let next = PendingEvent {
            event_id: "next".into(),
            target: Some(holm),
            neighbour: Some(nordmark),
        };
        assert_eq!(queue, [(Tick(1), next)]);
    }

    #[test]
    fn sign_defaults_to_bad() {
        let event = |sign: &str| {
            let text = format!(
                r#"(id: "e", title: "", text: "", when: All([]), weight: 1, once: false,
                cooldown_years: 0, importance: 0, {sign} target: None, choices: [])"#
            );
            crate::data::parse::<Event>(&text).unwrap().sign
        };
        assert_eq!(event(""), Sign::Bad);
        assert_eq!(event("sign: Good,"), Sign::Good);
        assert_eq!(event("sign: Bad,"), Sign::Bad);
    }
}
