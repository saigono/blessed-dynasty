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
            Predicate::All(ps) => ps.iter().all(|p| p.eval(w)),
            Predicate::Any(ps) => ps.iter().any(|p| p.eval(w)),
            Predicate::Not(p) => !p.eval(w),
        }
    }

    fn check(&self, data: &Data) -> Result<(), String> {
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
                match op {
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
                    HeirOp::Remove(_) => {}
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

/// Texts may contain `{province}`, `{neighbour}`, `{ruler}`; `Game` fills them in for display.
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
    pub target: EventTarget,
    pub choices: Vec<Choice>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum EventTarget {
    None,
    /// The event cannot fire while no province matches.
    RandomProvince(ProvinceFilter),
    Neighbour,
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
    use crate::state::Preset;

    fn world() -> (Data, World) {
        let data = crate::data::load(include_str!("../../../data/rules.ron")).unwrap();
        let preset = include_str!("../../../data/presets/default.ron");
        let preset = Preset::load(preset, &data).unwrap();
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
        assert!(!p("ProvinceWhere((holder: Vassal, loyalty_above: 40))"));
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
        assert_eq!(matching("()"), ["capital", "holm", "nordheim"]);
        assert_eq!(matching("(holder: Crown)"), ["capital"]);
        assert_eq!(matching("(holder: Foreign)"), ["nordheim"]);
        assert_eq!(matching("(loyalty_below: 50)"), ["holm"]);
        assert_eq!(matching("(loyalty_above: 50)"), ["capital"]);
        assert_eq!(matching(r#"(building: "fort")"#), ["capital"]);
        assert_eq!(
            matching(r#"(without_building: "fort")"#),
            ["holm", "nordheim"]
        );
        // holm borders nordheim; nordheim borders no other foreign state.
        assert_eq!(matching("(borders_foreign: true)"), ["holm"]);
        assert_eq!(
            matching("(borders_foreign: false)"),
            ["capital", "nordheim"]
        );
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
}
