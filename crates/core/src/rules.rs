//! Content schema read by `Game`: predicates, effects, events, actions.

use crate::data::Data;
use crate::fx::Fx;
use crate::state::{AxisId, HeirStatus, Holder, NeighbourId, Province, ProvinceId, World};
use crate::time::{Tick, Years};
use serde::{Deserialize, Serialize};

/// Province loyalty, ruler health and heir ability live in 0..=100, relations in -100..=100.
const PERCENT: Fx = Fx::from_int(100);

/// What an event or an action is about.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Province(ProvinceId),
    Neighbour(NeighbourId),
    /// Index into `World.heirs`.
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
    /// Inclusive range.
    RulerAge(u32, u32),
    AtWar,
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
            Predicate::RulerAge(lo, hi) => (*lo..=*hi).contains(&w.ruler.age),
            // There is no war state before stage 4.
            Predicate::AtWar => false,
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
}

impl ProvinceFilter {
    pub fn matches(&self, p: &Province, w: &World) -> bool {
        let kind = match p.holder {
            Holder::Crown => HolderKind::Crown,
            Holder::Vassal(_) => HolderKind::Vassal,
            Holder::Foreign(_) => HolderKind::Foreign,
        };
        let borders = p.neighbours.iter().any(|n| {
            w.provinces
                .get(n)
                .is_some_and(|q| matches!(q.holder, Holder::Foreign(_)) && q.holder != p.holder)
        });
        self.holder.is_none_or(|h| h == kind)
            && self.loyalty_below.is_none_or(|v| p.loyalty < v)
            && self.loyalty_above.is_none_or(|v| p.loyalty > v)
            && self
                .building
                .as_ref()
                .is_none_or(|b| p.buildings.contains(b))
            && (self.without_building.as_ref()).is_none_or(|b| !p.buildings.contains(b))
            && self.borders_foreign.is_none_or(|b| b == borders)
            && (self.capital).is_none_or(|c| c == (p.id == w.capital.province))
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
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NeighbourTarget {
    ById(NeighbourId),
    /// The neighbour of the event or action; no-op if it has none.
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
    /// Pushes `Data.new_heir`.
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
}

/// What an effect may touch besides the world.
pub struct Ctx<'a> {
    pub data: &'a Data,
    /// Deferred events: `(due tick, event id)`.
    pub queue: &'a mut Vec<(Tick, String)>,
    pub target: Option<&'a Target>,
}

impl Effect {
    /// The caller recomputes loyalty and crown power after a batch.
    pub fn apply(&self, w: &mut World, ctx: &mut Ctx) {
        let pct = |v: Fx| v.clamp(Fx(0), PERCENT);
        match self {
            Effect::Axis(a, d) => add_axis(w, ctx.data, a, *d),
            Effect::Province(t, field, d) => {
                let id = t.resolve(w, ctx.target);
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
                ctx.queue.push((due, id.clone()));
            }
            Effect::RulerHealth(d) => w.ruler.health = pct(w.ruler.health + *d),
            Effect::Relation(t, d) => {
                let id = match (t, ctx.target) {
                    (NeighbourTarget::ById(id), _) => Some(id),
                    (NeighbourTarget::EventTarget, Some(Target::Neighbour(id))) => Some(id),
                    _ => None,
                };
                if let Some(n) = id.and_then(|id| w.neighbours.get_mut(id)) {
                    n.relation = (n.relation + *d).clamp(Fx(0) - PERCENT, PERCENT);
                }
            }
            Effect::CrownPower(t, d) => {
                if let Some(id) = t.resolve(w, ctx.target) {
                    let m = w.crown_modifiers.entry(id).or_default();
                    *m = *m + *d;
                }
            }
            Effect::HeirOp(op) => {
                let heir = |i: &u32| *i as usize;
                // Out of range, so a no-op, when there is no target heir.
                let t = match ctx.target {
                    Some(Target::Heir(i)) => *i,
                    _ => u32::MAX,
                };
                let op = match op.clone() {
                    HeirOp::TargetStatus(s) => HeirOp::SetStatus(t, s),
                    HeirOp::TargetAbility(d) => HeirOp::Ability(t, d),
                    HeirOp::TargetClaim(d) => HeirOp::Claim(t, d),
                    HeirOp::TargetRemove => HeirOp::Remove(t),
                    op => op,
                };
                match &op {
                    HeirOp::Add => w.heirs.push(ctx.data.new_heir.clone()),
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
                    HeirOp::Remove(_) => {}
                    HeirOp::TargetStatus(_)
                    | HeirOp::TargetAbility(_)
                    | HeirOp::TargetClaim(_)
                    | HeirOp::TargetRemove => {
                        unreachable!("resolved above")
                    }
                }
            }
            Effect::RulerDies(_) | Effect::Abdicate | Effect::Chance(_) => {
                unreachable!("Game applies these")
            }
            Effect::Build(t, b) => {
                let id = t.resolve(w, ctx.target);
                if let Some(p) = id.and_then(|id| w.provinces.get_mut(&id)) {
                    p.buildings.insert(b.clone());
                }
            }
            Effect::Grant(t) => {
                let Some(id) = t.resolve(w, ctx.target) else {
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
                let fresh = (g.new_vassals.iter()).find(|v| !w.vassals.contains_key(&v.id));
                let vassal = match (nearest, fresh) {
                    (Some((d, v)), _) if d <= g.max_distance => v,
                    (_, Some(v)) => {
                        w.vassals.insert(v.id.clone(), v.clone());
                        v.id.clone()
                    }
                    (Some((_, v)), None) => v,
                    (None, None) => return,
                };
                w.provinces.get_mut(&id).expect("checked above").holder = Holder::Vassal(vassal);
            }
            Effect::Revoke(t) => {
                let id = t.resolve(w, ctx.target);
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
            _ => Ok(()),
        }
    }
}

impl ProvinceTarget {
    fn resolve(&self, w: &World, target: Option<&Target>) -> Option<ProvinceId> {
        match (self, target) {
            (ProvinceTarget::Capital, _) => Some(w.capital.province.clone()),
            (ProvinceTarget::ById(id), _) => Some(id.clone()),
            (ProvinceTarget::EventTarget, Some(Target::Province(id))) => Some(id.clone()),
            _ => None,
        }
    }
}

/// Adds `d` to the axis, clamped to its bounds from `rules.ron`.
pub(crate) fn add_axis(w: &mut World, data: &Data, id: &AxisId, d: Fx) {
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

/// Texts may contain `{province}`, `{neighbour}`, `{heir}`, `{ruler}`; `Game` fills them in for display.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub text: String,
    pub when: Predicate,
    /// 0 keeps the event out of the random pool: it only fires via `SpawnEvent`.
    pub weight: u32,
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
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Choice {
    pub text: String,
    pub effects: Vec<Effect>,
    pub cause_tag: String,
    pub hint: Option<String>,
}

impl Event {
    pub(crate) fn check(&self, data: &Data) -> Result<(), String> {
        if self.choices.is_empty() {
            return Err("an event needs at least one choice".into());
        }
        self.when.check(data)?;
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
    pub cause_tag: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ActionTarget {
    None,
    Province(ProvinceFilter),
    Neighbour,
    Heir,
}

impl Action {
    pub(crate) fn check(&self, data: &Data) -> Result<(), String> {
        self.requires.check(data)?;
        self.on_complete.iter().try_for_each(|e| e.check(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Preset, VassalId};

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
        // legitimacy is 60.
        assert!(p(r#"AxisAbove("legitimacy", 59.999)"#));
        assert!(!p(r#"AxisAbove("legitimacy", 60)"#));
        assert!(p(r#"AxisBelow("legitimacy", 60.001)"#));
        assert!(!p(r#"AxisBelow("legitimacy", 60)"#));
        assert!(p(r#"Flag("married")"#) && !p(r#"Flag("plague")"#));
        assert!(p(r#"NotFlag("plague")"#) && !p(r#"NotFlag("married")"#));
        assert!(p("ProvinceWhere((holder: Vassal))"));
        assert!(!p("ProvinceWhere((holder: Vassal, loyalty_above: 45))"));
        // Two heirs, ruler 30.
        assert!(p("HeirCount(2, 2)") && !p("HeirCount(3, 9)") && !p("HeirCount(0, 1)"));
        assert!(p("RulerAge(30, 30)") && !p("RulerAge(31, 99)"));
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
                },
            );
        };
        run(&mut w, r#"Axis("legitimacy", 5.5)"#, None);
        assert_eq!(w.axes[&ax("legitimacy")], Fx(65_500));
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
        assert_eq!(relation(&w), Fx::from_int(50));
        run(&mut w, "Relation(EventTarget, -900)", Some(&nordmark));
        assert_eq!(relation(&w), Fx::from_int(-100));
        run(&mut w, "Relation(EventTarget, 20)", Some(&holm));
        assert_eq!(relation(&w), Fx::from_int(-100));

        run(&mut w, "CrownPower(EventTarget, 7)", Some(&holm));
        assert_eq!(w.crown_modifiers[&pid("holm")], Fx::from_int(7));

        run(&mut w, "HeirOp(Add)", None);
        assert_eq!(w.heirs.len(), 3);
        assert_eq!(w.heirs[2], data.new_heir);
        run(&mut w, "HeirOp(Remove(0))", None);
        assert_eq!(w.heirs[0].name, "Агнесса");
        run(&mut w, r#"HeirOp(SetStatus(0, Hostage("nordmark")))"#, None);
        assert_eq!(
            w.heirs[0].status,
            HeirStatus::Hostage(NeighbourId("nordmark".into()))
        );
        run(&mut w, "HeirOp(Ability(0, 10))", None);
        assert_eq!(w.heirs[0].ability, Fx::from_int(65));
        let before = w.clone();
        run(&mut w, "HeirOp(Remove(9))", None);
        run(&mut w, "HeirOp(Ability(9, 1))", None);
        run(&mut w, "HeirOp(SetStatus(9, Home))", None);
        assert_eq!(w, before);

        w.tick = Tick(3);
        run(&mut w, r#"SpawnEvent("next", 2)"#, None);
        drop(run);
        assert_eq!(queue, [(Tick(5), "next".to_string())]);
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
            };
            e.apply(w, &mut ctx);
        };
        let holder = |w: &World, p: &str| w.provinces[&pid(p)].holder.clone();
        let vassal = |v: &str| Holder::Vassal(VassalId(v.into()));

        run(&mut w, &data, r#"Build(EventTarget, "market")"#, "berg");
        assert!(w.provinces[&pid("berg")].buildings.contains("market"));

        // Next to holm (weir) at one crossing.
        run(&mut w, &data, "Grant(EventTarget)", "gart");
        assert_eq!(holder(&w, "gart"), vassal("weir"));
        // Arden and Weir both border the capital: the smaller id wins the tie.
        run(&mut w, &data, "Grant(ById(\"capital\"))", "gart");
        assert_eq!(holder(&w, "capital"), vassal("arden"));
        // Not a crown province: no-op.
        run(&mut w, &data, "Grant(EventTarget)", "nordheim");
        assert_eq!(
            holder(&w, "nordheim"),
            Holder::Foreign(NeighbourId("nordmark".into()))
        );

        // Nobody close enough: a new house from the list, then the next one.
        data.grant.max_distance = 0;
        run(&mut w, &data, "Grant(EventTarget)", "sol");
        assert_eq!(holder(&w, "sol"), vassal("rosten"));
        assert_eq!(w.vassals[&VassalId("rosten".into())].name, "Ростен");
        run(&mut w, &data, "Grant(EventTarget)", "berg");
        assert_eq!(holder(&w, "berg"), vassal("olbek"));
        // The list is used up: the nearest vassal at any distance.
        data.grant.new_vassals.clear();
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
