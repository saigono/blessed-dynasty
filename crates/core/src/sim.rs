//! The dynasty after the founder: the same `Game` year by year, choices by `AutoChooser`,
//! until the dynasty falls or `sim.max_years` pass. The result is a `Chronicle`.

use crate::data::Data;
use crate::fx::Fx;
use crate::game::{ActionId, Game, PendingEvent, ReignEnd, Step};
use crate::rng::Rng;
use crate::rules::{Choice, Effect, Event, HeirOp, NewHolder, Predicate, ProvinceField, Target};
use crate::state::{CauseTag, HeirStatus, Holder, MarkKey, ProvinceId, Ruler, World};
use crate::time::Tick;
use crate::war::WarStage;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chronicle {
    pub entries: Vec<ChronicleEntry>,
    pub fall: FallReason,
    /// From tick 0 (the founder's accession) to the fall, at most `sim.max_years`.
    pub years: u32,
    /// The founder first.
    pub rulers: Vec<RulerRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChronicleEntry {
    pub tick: Tick,
    /// The event behind the entry; None for a new ruler and a province lost or gained.
    pub event: Option<String>,
    pub title: String,
    pub text: String,
    /// The hint (`data/hints.ron`) of the main cause as a sentence, if that cause weighs at
    /// least `sim.hint_weight`.
    pub hint: Option<String>,
    pub importance: u32,
    /// The player's decisions behind the entry, one per decision, heaviest first.
    pub causes: Vec<CauseTag>,
    pub snapshot: World,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum FallReason {
    NoHeir,
    CapitalLost,
    /// `sim.usurped_flag` is set.
    Usurped,
    /// No province is left to the crown: the realm fell apart into appanages.
    NoCrownLand,
    /// Not a fall: the dynasty reached `sim.max_years`.
    Alive,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RulerRecord {
    pub name: String,
    pub traits: BTreeSet<String>,
    pub start: Tick,
    pub end: Tick,
    /// The cause of the reign end; None when the dynasty fell under him or lives on.
    pub cause: Option<String>,
}

/// Plays the dynasty from the end of the founder's reign. Simulation events
/// (`Data.sim_events`) join the pool; nothing the automaton does is a player decision, so
/// it marks nothing. An unfinished war starts over from its declaration under the new ruler,
/// since the deferred queue ends with the reign.
pub fn run(reign_end: ReignEnd, data: &Data, rng: Rng) -> Chronicle {
    let mut data = data.clone();
    data.events.extend(data.sim_events.clone());
    let s = data.sim.clone();
    let founder = record(&reign_end.world.ruler);
    let mut c = Chronicle {
        entries: Vec::new(),
        fall: FallReason::Alive,
        years: 0,
        rulers: vec![RulerRecord {
            end: reign_end.tick,
            cause: Some(reign_end.cause),
            ..founder
        }],
    };
    let mut g = Game {
        world: reign_end.world,
        rng,
        data,
        decisions: Vec::new(),
        pending_event: None,
        queue: Vec::new(),
        ended: None,
        reported: false,
    };
    if let Some(war) = &mut g.world.war {
        war.stage = WarStage::Declared;
        let p = PendingEvent {
            event_id: g.data.war.start_event.clone(),
            target: Some(Target::Neighbour(war.enemy.clone())),
            neighbour: None,
        };
        g.queue.push((g.world.tick, p));
    }
    let tpy = g.world.time_unit.ticks_per_year;
    // The death of a young first heir who was the last one, with its place in the entries:
    // told only if no heir comes after and the dynasty ends for want of one.
    let mut last_heir: Option<(usize, ChronicleEntry)> = None;
    c.fall = 'dynasty: loop {
        if !crown(&mut g, &mut c) {
            if let Some((i, e)) = last_heir {
                c.entries.insert(i, e);
            }
            break FallReason::NoHeir;
        }
        let auto = AutoChooser::for_ruler(&g.data, &g.world.ruler);
        loop {
            if let Some(fall) = fallen(&g) {
                break 'dynasty fall;
            }
            if g.world.tick.year(g.world.time_unit) >= s.max_years {
                break 'dynasty FallReason::Alive;
            }
            if g.world.ruler.age >= s.regency_age {
                g.world.flags.remove(&s.regency_flag);
            }
            if g.world.tick.0.is_multiple_of(tpy)
                && let Some((id, target)) = auto.action(&mut g)
            {
                // Listed by available_actions; only the slot may be gone.
                let _ = g.start(&id, target, false);
            }
            let holders: Vec<Holder> = g
                .world
                .provinces
                .values()
                .map(|p| p.holder.clone())
                .collect();
            let first = next_heir(&g.world).map(|i| g.world.heirs[i].clone());
            let step = g.wait().expect("the reign goes on");
            // Within a tick only the yearly age risk takes an heir; events do on resolve.
            if let Some(h) = first.filter(|h| g.world.heir_index(h.id).is_none()) {
                let (title, text) = &s.texts.heir_died;
                let told = (
                    title.replace("{heir}", &h.name),
                    text.replace("{heir}", &h.name),
                );
                let causes = causes(&g.world, [MarkKey::Heir(h.id)].into());
                let e = entry(&g, told, s.notable, causes);
                // `h` is from before the tick; heirs age a year before the death roll.
                if h.age + 1 >= s.heir_death_age {
                    c.entries.push(e);
                } else if g.world.heirs.is_empty() {
                    last_heir = Some((c.entries.len(), e));
                }
            }
            if !g.world.heirs.is_empty() {
                last_heir = None;
            }
            match step {
                Step::Idle => {}
                Step::Event(v) => {
                    // Causes as the world stood before the choice; only entries need them.
                    let told = v.importance >= s.threshold;
                    let causes = told.then(|| {
                        let p = g.pending_event.as_ref().expect("an event waits");
                        let e = g.data.events.iter().find(|e| e.id == p.event_id);
                        let keys = event_keys(&g.world, e.expect("pending events exist"), p);
                        causes(&g.world, keys)
                    });
                    let idx = auto.choose(&mut g, &v.choices);
                    g.resolve(idx, false).expect("a listed choice");
                    if let Some(causes) = causes {
                        let e = entry(&g, (v.title, v.text), v.importance, causes);
                        c.entries.push(ChronicleEntry {
                            event: Some(v.event_id),
                            ..e
                        });
                    }
                }
                Step::ReignEnded(end) => {
                    let last = c.rulers.last_mut().expect("a ruler reigned");
                    (last.end, last.cause) = (end.tick, Some(end.cause));
                    break;
                }
            }
            province_entries(&g, &holders, &mut c);
        }
    };
    let w = &g.world;
    c.years = w.tick.year(w.time_unit);
    let last = c.rulers.last_mut().expect("a ruler reigned");
    if last.cause.is_none() {
        last.end = w.tick;
    }
    c
}

/// The next ruler: the heir with the highest claim (claims follow the succession law), the
/// eldest on a tie. He keeps his name unless it is the newborn placeholder, then one from
/// `names.rulers`; traits roll by `sim.traits`; health `sim.ruler_health`. None: no heir.
pub fn succession(w: &World, data: &Data, rng: &mut Rng) -> Option<Ruler> {
    let heir = &w.heirs[next_heir(w)?];
    let pool = &data.names.rulers;
    let name = match heir.name == data.new_heir.name && !pool.is_empty() {
        true => pool[rng.range(0, pool.len() as i64) as usize].clone(),
        false => heir.name.clone(),
    };
    let traits = (data.sim.traits.iter())
        .filter(|t| {
            let status = match heir.status {
                HeirStatus::Home => Fx(0),
                HeirStatus::Studying(_) => t.studying,
                HeirStatus::Hostage(_) => t.hostage,
            };
            let chance = t.percent + heir.ability * t.ability_k + status;
            rng.range(0, Fx::from_int(100).0) < chance.0
        })
        .map(|t| t.id.clone())
        .collect();
    Some(Ruler {
        name,
        age: heir.age,
        health: data.sim.ruler_health,
        traits,
        reign_start: w.tick,
    })
}

fn next_heir(w: &World) -> Option<usize> {
    // max_by_key keeps the last of equals; going backwards, that is the eldest.
    (0..w.heirs.len()).rev().max_by_key(|&i| w.heirs[i].claim)
}

/// Crowns the next heir: a claim below the law's `crisis_claim` contests the succession
/// (`abdication.contested_flag`), a child reigns under the regency flag, the other heirs
/// become the collateral line. False: no heir.
fn crown(g: &mut Game, c: &mut Chronicle) -> bool {
    let Some(ruler) = succession(&g.world, &g.data, &mut g.rng) else {
        return false;
    };
    let (d, w) = (&g.data, &mut g.world);
    let heir = w.heirs.remove(next_heir(w).expect("succession found one"));
    // His brothers and sisters become the collateral line, behind his children to come.
    w.line_from = w.next_heir_id;
    if (d.heirs.law(w)).is_some_and(|l| heir.claim < l.crisis_claim) {
        w.flags.insert(d.abdication.contested_flag.clone());
    }
    if ruler.age < d.sim.regency_age {
        w.flags.insert(d.sim.regency_flag.clone());
    }
    let (title, text) = &d.sim.texts.crowned;
    let fill = |s: &String| s.replace("{ruler}", &ruler.name);
    let told = (fill(title), fill(text));
    c.rulers.push(record(&ruler));
    w.ruler = ruler;
    (g.ended, g.reported) = (None, false);
    let causes = causes(&g.world, [MarkKey::Heir(heir.id)].into());
    c.entries.push(entry(g, told, g.data.sim.notable, causes));
    true
}

fn record(r: &Ruler) -> RulerRecord {
    RulerRecord {
        name: r.name.clone(),
        traits: r.traits.clone(),
        start: r.reign_start,
        end: r.reign_start,
        cause: None,
    }
}

fn fallen(g: &Game) -> Option<FallReason> {
    let w = &g.world;
    let crown = |id: &ProvinceId| {
        w.provinces
            .get(id)
            .is_some_and(|p| p.holder == Holder::Crown)
    };
    if w.flags.contains(&g.data.sim.usurped_flag) {
        Some(FallReason::Usurped)
    } else if !w.provinces.keys().any(crown) {
        Some(FallReason::NoCrownLand)
    } else if !crown(&w.capital.province) {
        Some(FallReason::CapitalLost)
    } else {
        None
    }
}

/// An entry for every province that left the realm (crown and vassals) or joined it since
/// `holders` (in province order).
fn province_entries(g: &Game, holders: &[Holder], c: &mut Chronicle) {
    let t = &g.data.sim.texts;
    let w = &g.world;
    for (p, was) in w.provinces.values().zip(holders) {
        let ((title, text), foreign) = match (was, &p.holder) {
            (Holder::Foreign(_), Holder::Foreign(_)) => continue,
            (_, Holder::Foreign(n)) => (&t.province_lost, n),
            (Holder::Foreign(n), _) => (&t.province_gained, n),
            _ => continue,
        };
        let neighbour = w.neighbours.get(foreign).map_or("", |n| &n.name);
        let fill = |s: &String| {
            s.replace("{province}", &p.name)
                .replace("{neighbour}", neighbour)
        };
        let causes = causes(w, [MarkKey::Province(p.id.clone())].into());
        let told = (fill(title), fill(text));
        c.entries.push(entry(g, told, g.data.sim.notable, causes));
    }
}

fn entry(
    g: &Game,
    (title, text): (String, String),
    importance: u32,
    causes: Vec<CauseTag>,
) -> ChronicleEntry {
    let main = causes
        .first()
        .filter(|c| c.weight >= g.data.sim.hint_weight);
    // Hints are lowercase clauses; the entry tells one as a sentence of its own.
    let hint = main.and_then(|c| g.data.hints.get(&c.cause_tag)).map(|h| {
        let mut chars = h.chars();
        let first = chars.next().into_iter().flat_map(char::to_uppercase);
        format!("{}.", first.chain(chars).collect::<String>())
    });
    ChronicleEntry {
        tick: g.world.tick,
        event: None,
        title,
        text,
        hint,
        importance,
        causes,
        snapshot: g.world.snapshot(),
    }
}

/// The marks on `keys`, summed per decision, heaviest first, earlier decisions first on a tie.
fn causes(w: &World, keys: BTreeSet<MarkKey>) -> Vec<CauseTag> {
    let mut by_decision: BTreeMap<usize, CauseTag> = BTreeMap::new();
    for t in keys.iter().filter_map(|k| w.marks.get(k)).flatten() {
        let c = by_decision.entry(t.decision_idx).or_insert(CauseTag {
            weight: Fx(0),
            ..t.clone()
        });
        c.weight = c.weight + t.weight;
    }
    let mut causes: Vec<_> = by_decision.into_values().collect();
    causes.sort_by_key(|c| Reverse(c.weight));
    causes
}

/// What an event is about: the axes, flags and matching provinces of its `when` and of the
/// `weight_bonus` that hold, and its targets; a vassal's province brings all of his.
fn event_keys(w: &World, e: &Event, p: &PendingEvent) -> BTreeSet<MarkKey> {
    let mut keys = BTreeSet::new();
    let bonus = e.weight_bonus.iter().filter(|(b, _)| b.eval(w));
    for pred in std::iter::once(&e.when).chain(bonus.map(|(b, _)| b)) {
        predicate_keys(pred, w, &mut keys);
    }
    match &p.target {
        Some(Target::Province(id)) => {
            keys.insert(MarkKey::Province(id.clone()));
            if let Some(h @ Holder::Vassal(_)) = w.provinces.get(id).map(|q| &q.holder) {
                let domain = w.provinces.values().filter(|q| q.holder == *h);
                keys.extend(domain.map(|q| MarkKey::Province(q.id.clone())));
            }
        }
        Some(Target::Neighbour(n)) => {
            keys.insert(MarkKey::Neighbour(n.clone()));
        }
        Some(Target::Heir(id)) => {
            keys.insert(MarkKey::Heir(*id));
        }
        None => {}
    }
    keys.extend(p.neighbour.iter().map(|n| MarkKey::Neighbour(n.clone())));
    keys
}

fn predicate_keys(p: &Predicate, w: &World, keys: &mut BTreeSet<MarkKey>) {
    match p {
        Predicate::AxisAbove(a, _) | Predicate::AxisBelow(a, _) => {
            keys.insert(MarkKey::Axis(a.clone()));
        }
        Predicate::Flag(f) | Predicate::NotFlag(f) => {
            keys.insert(MarkKey::Flag(f.clone()));
        }
        Predicate::ProvinceWhere(f) => {
            let matching = w.provinces.values().filter(|q| f.matches(q, w));
            keys.extend(matching.map(|q| MarkKey::Province(q.id.clone())));
        }
        Predicate::All(ps) | Predicate::Any(ps) => {
            ps.iter().for_each(|p| predicate_keys(p, w, keys));
        }
        Predicate::Not(p) => predicate_keys(p, w, keys),
        _ => {}
    }
}

/// Chooses for the simulated rulers. An option scores `sum(weights[key] * amount)` over its
/// effects (see `worth`) plus a roll in `0..=noise`; the best wins, the first on a tie.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoChooser {
    pub weights: BTreeMap<String, Fx>,
    pub noise: Fx,
}

impl AutoChooser {
    /// `sim.auto.base` plus the weights of every trait the ruler has.
    pub fn for_ruler(data: &Data, ruler: &Ruler) -> AutoChooser {
        let a = &data.sim.auto;
        let mut weights = a.base.clone();
        for (k, v) in ruler
            .traits
            .iter()
            .filter_map(|t| a.traits.get(t))
            .flatten()
        {
            let w = weights.entry(k.clone()).or_default();
            *w = *w + *v;
        }
        AutoChooser {
            weights,
            noise: a.noise,
        }
    }

    /// Index of the best choice.
    pub fn choose(&self, g: &mut Game, choices: &[Choice]) -> usize {
        let scores = choices
            .iter()
            .map(|c| self.worth(&c.effects, &g.world, &g.data));
        let scores: Vec<Fx> = scores.collect();
        self.best(&scores, &mut g.rng)
    }

    /// The best action of `available_actions` (its cost counts as treasury spent), or None
    /// when no slot is free or doing nothing (score 0) wins.
    pub fn action(&self, g: &mut Game) -> Option<(ActionId, Option<Target>)> {
        if g.world.active_actions.len() as u32 >= g.data.action_slots.slots(&g.world) {
            return None;
        }
        let mut options = vec![None];
        let mut scores = vec![Fx(0)];
        let treasury = self.weight(&g.data.economy.treasury.0);
        let mut actions = g.available_actions();
        for (k, (id, targets)) in actions.iter().enumerate() {
            let a = g.data.actions.iter().find(|a| a.id == *id).expect("listed");
            let score = self.worth(&a.on_complete, &g.world, &g.data) - treasury * a.cost;
            for t in 0..targets.len().max(1) {
                options.push(Some((k, t)));
                scores.push(score);
            }
        }
        let (k, t) = options[self.best(&scores, &mut g.rng)]?;
        let (id, mut targets) = actions.swap_remove(k);
        Some((id, (t < targets.len()).then(|| targets.swap_remove(t))))
    }

    fn best(&self, scores: &[Fx], rng: &mut Rng) -> usize {
        let mut best: Option<(usize, Fx)> = None;
        for (i, s) in scores.iter().enumerate() {
            let s = *s + Fx(rng.range(0, self.noise.0 + 1));
            if best.is_none_or(|(_, b)| s > b) {
                best = Some((i, s));
            }
        }
        best.map_or(0, |(i, _)| i)
    }

    fn weight(&self, key: &str) -> Fx {
        self.weights.get(key).copied().unwrap_or_default()
    }

    /// Keys: axis ids (by the delta), flag ids (+1 set, -1 cleared), `province_income`,
    /// `province_population`, `province_loyalty`, `health`, `relation`, `crown_power`
    /// (by the delta), `build`, `grant`, `revoke`, `secede`, `war`, `hostage`, `death`,
    /// `abdicate` (+1 each), `province` (+1 gained, -1 given away), `heir` (+1 born, -1 lost),
    /// `heir_ability`, `heir_claim` (by the delta). A chance weighs both branches by its odds.
    fn worth(&self, effects: &[Effect], w: &World, data: &Data) -> Fx {
        let one = Fx::from_int(1);
        let mut sum = Fx(0);
        for e in effects {
            let (key, v): (&str, Fx) = match e {
                Effect::Axis(a, d) => (&a.0, *d),
                Effect::Tribute(v) => (&data.economy.treasury.0, *v),
                Effect::Province(_, ProvinceField::Income, d) => ("province_income", *d),
                Effect::Province(_, ProvinceField::Population, d) => ("province_population", *d),
                Effect::Province(_, ProvinceField::Loyalty, d) => ("province_loyalty", *d),
                Effect::SetFlag(f) => (f, one),
                Effect::ClearFlag(f) => (f, Fx::from_int(-1)),
                Effect::RulerHealth(d) => ("health", *d),
                Effect::Relation(_, d) | Effect::OtherRelations(d) => ("relation", *d),
                Effect::CrownPower(_, d) => ("crown_power", *d),
                Effect::Build(..) => ("build", one),
                Effect::Grant(_) => ("grant", one),
                Effect::Revoke(_) => ("revoke", one),
                Effect::Secede(_) => ("secede", one),
                Effect::StartWar(_) => ("war", one),
                Effect::TakeHostage(..) => ("hostage", one),
                Effect::RulerDies(_) => ("death", one),
                Effect::Abdicate => ("abdicate", one),
                Effect::TransferProvince(_, NewHolder::Crown) => ("province", one),
                Effect::TransferProvince(_, NewHolder::Foreign(_)) => {
                    ("province", Fx::from_int(-1))
                }
                Effect::HeirOp(HeirOp::Add) => ("heir", one),
                Effect::HeirOp(HeirOp::Remove(_) | HeirOp::TargetRemove) => {
                    ("heir", Fx::from_int(-1))
                }
                Effect::HeirOp(HeirOp::Ability(_, d) | HeirOp::TargetAbility(d)) => {
                    ("heir_ability", *d)
                }
                Effect::HeirOp(HeirOp::Claim(_, d) | HeirOp::TargetClaim(d)) => ("heir_claim", *d),
                Effect::Chance(c) => {
                    let hit = c.percent(w) / Fx::from_int(100);
                    sum = sum + self.worth(&c.then, w, data) * hit;
                    sum = sum + self.worth(&c.otherwise, w, data) * (one - hit);
                    continue;
                }
                Effect::IfFriendly(es) => {
                    sum = sum + self.worth(es, w, data);
                    continue;
                }
                Effect::HeirOp(HeirOp::SetStatus(..) | HeirOp::TargetStatus(_))
                | Effect::SpawnEvent(..)
                | Effect::Clash
                | Effect::EndWar(_)
                | Effect::SetWarStage(_) => continue,
            };
            sum = sum + self.weight(key) * v;
        }
        sum
    }
}
