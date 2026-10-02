use crate::fx::Fx;
use crate::rules::{Action, Event, Predicate};
use crate::sim::FallReason;
use crate::state::{AxisId, Heir, Holder, Stance, World};
use crate::time::TimeUnit;
use crate::war::WarOutcome;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, BTreeSet};

/// Everything loaded from `rules.ron`. Grows stage by stage.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Data {
    pub time_unit: TimeUnit,
    pub axes: Vec<AxisDef>,
    pub crown_power: CrownPowerRules,
    pub factions: Vec<Faction>,
    /// Derived axis: weighted mean of the faction axes, see `World::recompute_loyalty`.
    pub loyalty_axis: AxisId,
    pub action_slots: ActionSlots,
    pub economy: Economy,
    pub drift: Drift,
    /// Weight of "nothing happens" in the random event pick.
    pub quiet_weight: u32,
    /// Province loyalty below this shows as unrest. Display only.
    #[serde(default)]
    pub unrest_below: Fx,
    /// What `HeirOp::Add` and a birth push, named by `Data::newborn`.
    pub new_heir: Heir,
    pub death: Death,
    pub abdication: Abdication,
    pub heirs: HeirRules,
    pub neighbour_ai: NeighbourAi,
    pub crown_capacity: CrownCapacity,
    pub grant: GrantRules,
    pub war: WarRules,
    pub sim: SimRules,
    /// From `add_events`, not from `rules.ron`.
    #[serde(default)]
    pub events: Vec<Event>,
    /// From `add_sim_events`: events only the simulation draws (`data/events/sim/`).
    #[serde(default)]
    pub sim_events: Vec<Event>,
    /// From `add_hints`: chronicle hint per `cause_tag`.
    #[serde(default)]
    pub hints: BTreeMap<String, String>,
    /// From `add_actions`, not from `rules.ron`.
    #[serde(default)]
    pub actions: Vec<Action>,
    /// From `add_names`, not from `rules.ron`.
    #[serde(default)]
    pub names: Names,
}

/// Name pools (`data/names.ron`). A vassal house founded by `Effect::Grant` takes the first
/// name of `vassals` not yet in the world; the name is also its id.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Names {
    pub rulers: Vec<String>,
    pub heirs: Vec<String>,
    pub vassals: Vec<String>,
}

/// Concurrent actions: the largest `slots` whose `threshold` the axis has reached.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ActionSlots {
    pub axis: AxisId,
    pub steps: Vec<(Fx, u32)>,
}

impl ActionSlots {
    pub fn slots(&self, w: &World) -> u32 {
        let v = w.axes[&self.axis];
        let open = self.steps.iter().filter(|(threshold, _)| v >= *threshold);
        open.map(|(_, slots)| *slots).max().unwrap_or(0)
    }
}

/// Per year: `treasury += crown province income + sum(axis * coefficient over flows)`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Economy {
    pub treasury: AxisId,
    pub flows: Vec<(AxisId, Fx)>,
}

impl Economy {
    /// What the treasury gains in a year.
    pub fn yearly_income(&self, w: &World) -> Fx {
        let crown = w.provinces.values().filter(|p| p.holder == Holder::Crown);
        let income = crown.fold(Fx(0), |sum, p| sum + p.income);
        (self.flows.iter()).fold(income, |sum, (a, k)| sum + w.axes[a] * *k)
    }
}

/// Per year, faction axes move by `step` toward their default, province loyalty toward
/// `province_loyalty`, crown power modifiers toward 0.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Drift {
    pub step: Fx,
    pub province_loyalty: Fx,
}

impl Data {
    /// Derived axes are recomputed from others; nothing may write to them directly.
    pub fn is_derived(&self, axis: &AxisId) -> bool {
        *axis == self.loyalty_axis
    }

    /// Appends a list of events (`data/events/*.ron`). Ids are unique across all files.
    pub fn add_events(&mut self, text: &str) -> Result<(), DataError> {
        let events = self.parse_events(text)?;
        self.events.extend(events);
        Ok(())
    }

    /// Appends a list of simulation events (`data/events/sim/*.ron`), kept out of the reign.
    pub fn add_sim_events(&mut self, text: &str) -> Result<(), DataError> {
        let events = self.parse_events(text)?;
        self.sim_events.extend(events);
        Ok(())
    }

    fn parse_events(&self, text: &str) -> Result<Vec<Event>, DataError> {
        let events: Vec<Event> = parse(text)?;
        let all = self.events.iter().chain(&self.sim_events).chain(&events);
        unique(all.map(|e| e.id.as_str()))?;
        for e in &events {
            e.check(self).map_err(|m| invalid(&e.id, m))?;
        }
        Ok(events)
    }

    /// Sets the chronicle hints (`data/hints.ron`).
    pub fn add_hints(&mut self, text: &str) -> Result<(), DataError> {
        self.hints = parse(text)?;
        Ok(())
    }

    /// Appends a list of actions (`data/actions.ron`).
    pub fn add_actions(&mut self, text: &str) -> Result<(), DataError> {
        let actions: Vec<Action> = parse(text)?;
        unique(self.actions.iter().chain(&actions).map(|a| a.id.as_str()))?;
        for a in &actions {
            a.check(self).map_err(|m| invalid(&a.id, m))?;
        }
        self.actions.extend(actions);
        Ok(())
    }

    /// `new_heir` under the name `names.heirs[id % len]`, or its own name without a pool.
    pub fn newborn(&self, id: u32) -> Heir {
        let pool = &self.names.heirs;
        let name = match pool.is_empty() {
            true => self.new_heir.name.clone(),
            false => pool[id as usize % pool.len()].clone(),
        };
        Heir {
            name,
            ..self.new_heir.clone()
        }
    }

    /// Sets the name pools (`data/names.ron`). Names are unique within a pool.
    pub fn add_names(&mut self, text: &str) -> Result<(), DataError> {
        let names: Names = parse(text)?;
        for pool in [&names.rulers, &names.heirs, &names.vassals] {
            unique(pool.iter().map(|n| n.as_str()))?;
        }
        self.names = names;
        Ok(())
    }
}

/// RON with `implicit_some`, so data writes `holder: Crown` instead of `Some(Crown)`.
pub(crate) fn parse<T: DeserializeOwned>(text: &str) -> Result<T, DataError> {
    let options =
        ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    options.from_str(text).map_err(DataError::Parse)
}

fn unique<'a>(ids: impl Iterator<Item = &'a str>) -> Result<(), DataError> {
    let mut seen = BTreeSet::new();
    match ids.into_iter().find(|id| !seen.insert(*id)) {
        Some(id) => Err(invalid(id, "duplicate id".into())),
        None => Ok(()),
    }
}

fn invalid(id: &str, m: String) -> DataError {
    DataError::Invalid(format!("{id}: {m}"))
}

/// Yearly ruler death risk in per mille: the `base` row of the largest `age_from <= age`,
/// plus `(100 - health) * health_k`, plus every risk whose `when` holds. A hit spawns a
/// death event; while that event is on cooldown, its risk does not count.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Death {
    pub base: Vec<(u32, Fx)>,
    pub health_k: Fx,
    /// The event for the base and health risk.
    pub event: String,
    pub risks: Vec<Risk>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Risk {
    pub when: Predicate,
    pub per_mille: Fx,
    pub event: String,
}

/// `Game::abdicate` fires `event`. Its `Effect::Abdicate` ends the reign; unless the
/// `institutions` axis and the first heir's ability are above their thresholds, it also
/// takes `claim_drop` off the first heir's claim and sets `contested_flag`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Abdication {
    pub event: String,
    pub institutions: (AxisId, Fx),
    pub heir_ability: Fx,
    pub claim_drop: Fx,
    pub contested_flag: String,
}

/// Yearly heir rules.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HeirRules {
    /// Ability grows by status until this age.
    pub adult_age: u32,
    pub growth_home: Fx,
    pub growth_studying: Fx,
    pub growth_hostage: Fx,
    /// Claim change of a hostage, on top of the law.
    pub hostage_claim: Fx,
    /// Claims move by `claim_step` toward the target of the law whose flag is set.
    pub claim_step: Fx,
    pub laws: Vec<Law>,
    /// Birth chance in percent: the row of the largest `age_from <= ruler age`,
    /// times `unmarried` without `married_flag`.
    pub birth: Vec<(u32, Fx)>,
    pub married_flag: String,
    pub unmarried: Fx,
    /// Yearly death risk of every heir in per mille: the row of the largest
    /// `age_from <= heir age`. The dead leave the list.
    pub death: Vec<(u32, Fx)>,
}

impl HeirRules {
    /// The succession law in force: the first whose flag is set.
    pub fn law(&self, w: &World) -> Option<&Law> {
        self.laws.iter().find(|l| w.flags.contains(&l.flag))
    }
}

/// Claim target: `eldest` for heir 0, `others` for the rest, plus `ability * ability_k`.
/// A new ruler's claim below `crisis_claim` contests the succession.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Law {
    pub flag: String,
    pub eldest: Fx,
    pub others: Fx,
    pub ability_k: Fx,
    pub crisis_claim: Fx,
    /// Chance in percent per heir left after the coronation that the succession is
    /// contested anyway: more heirs, more quarrels.
    #[serde(default)]
    pub dispute_per_heir: Fx,
    /// Display name and a plain-words explanation for the UI. `{eldest}`, `{others}`,
    /// `{ability_k}`, `{crisis_claim}`, `{dispute_per_heir}` stand for the numbers above.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
}

impl Law {
    /// `description` with the numbers of this law filled in.
    pub fn text(&self) -> String {
        let numbers = [
            ("{eldest}", self.eldest),
            ("{others}", self.others),
            ("{ability_k}", self.ability_k),
            ("{crisis_claim}", self.crisis_claim),
            ("{dispute_per_heir}", self.dispute_per_heir),
        ];
        (numbers.iter()).fold(self.description.clone(), |s, (k, v)| {
            s.replace(k, &v.to_string())
        })
    }
}

/// The row of the largest `from <= at`; 0 below the first row.
pub fn by_age(table: &[(u32, Fx)], at: u32) -> Fx {
    let rows = table.iter().filter(|(from, _)| *from <= at);
    rows.max_by_key(|(from, _)| *from)
        .map_or(Fx(0), |(_, v)| *v)
}

/// Piecewise linear through `points` (sorted by x); flat beyond the first and the last.
pub fn curve(points: &[(Fx, Fx)], x: Fx) -> Fx {
    let Some(i) = points.iter().position(|(px, _)| *px > x) else {
        return points.last().map_or(Fx(0), |p| p.1);
    };
    if i == 0 {
        return points[0].1;
    }
    let ((x0, y0), (x1, y1)) = (points[i - 1], points[i]);
    y0 + (y1 - y0) * ((x - x0) / (x1 - x0))
}

/// A faction's loyalty lives in its `axis`; `weight` is its share in `loyalty_axis`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Faction {
    pub id: String,
    pub axis: AxisId,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AxisDef {
    pub id: AxisId,
    pub min: Fx,
    pub max: Fx,
    pub default: Fx,
    /// Display name; the UI shows the id when it is empty.
    #[serde(default)]
    pub name: String,
    /// A change of at least this much shows in the year's summary (`World::changes`);
    /// 0: never.
    #[serde(default)]
    pub notable: Fx,
}

/// Coefficients of `World::recompute_crown_power`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CrownPowerRules {
    pub base: HolderBase,
    /// Loyalty below this costs `(threshold - loyalty) * loyalty_penalty`.
    pub loyalty_threshold: Fx,
    pub loyalty_penalty: Fx,
    /// Per unit of path cost from the capital. A step into an own province costs 1,
    /// into a neighbour's `max(1, 1 + foreign_step_penalty - relation * foreign_step_relation)`.
    pub distance_penalty: Fx,
    pub foreign_step_penalty: Fx,
    pub foreign_step_relation: Fx,
    /// Bonus per building id. Buildings not listed give nothing.
    pub buildings: BTreeMap<String, Fx>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HolderBase {
    pub crown: Fx,
    pub vassal: Fx,
}

/// Yearly neighbour behaviour, see `neighbour::neighbour_tick`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NeighbourAi {
    /// Relation below this is hostile: Expand or Defend.
    pub hostile_below: Fx,
    /// Relation above this: Trade. In between: Wait.
    pub friendly_above: Fx,
    /// Expand needs strength >= the weakest border crown power + this.
    pub expand_margin: Fx,
    /// Every year the relation moves this much toward 0, after the stance's own change.
    pub drift: Fx,
    /// Every year the strength moves this much toward `Neighbour.per_province` times the
    /// provinces the state holds now.
    pub recover: Fx,
    pub expand: StanceRules,
    pub defend: StanceRules,
    pub trade: StanceRules,
    pub wait: StanceRules,
    /// Weight of an event a neighbour starts in the random event pick, next to the pool
    /// events and `quiet_weight`. Not picked, it is dropped.
    pub weight: u32,
}

impl NeighbourAi {
    pub fn stance(&self, s: &Stance) -> &StanceRules {
        match s {
            Stance::Expand => &self.expand,
            Stance::Defend => &self.defend,
            Stance::Trade => &self.trade,
            Stance::Wait => &self.wait,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StanceRules {
    /// Added to the relation every year.
    pub relation: Fx,
    /// `(event id, chance in percent per year)`; at most one fires, chances sum to <= 100.
    pub events: Vec<(String, u32)>,
}

/// Yearly limit of direct rule, see `Game::overreach`: the crown holds at most
/// `capital crown power * per_power` provinces; each of its weakest beyond that loses
/// `loyalty`, and goes to a vassal below `grant_below`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CrownCapacity {
    pub per_power: Fx,
    pub loyalty: Fx,
    pub grant_below: Fx,
}

/// Who gets a province from `Effect::Grant`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GrantRules {
    /// The nearest vassal within this many border crossings takes the province.
    pub max_distance: u32,
    /// Otherwise a new house from `Data.names.vassals` is founded with these.
    pub new_loyalty: Fx,
    pub new_strength: Fx,
}

/// War, see `war::strengths` and `war::clash`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WarRules {
    /// `StartWar` queues it at once, with the enemy as its target.
    pub start_event: String,
    pub army: AxisId,
    /// `(building, bonus)` per own province with it on the border with the enemy.
    pub fort: (String, Fx),
    /// `sum(axis * k)`, added to the multiplier with the fort bonus.
    pub bonus: Vec<(AxisId, Fx)>,
    /// The treasury factor runs from `treasury_poor` at 0 up to 1 at `treasury_full`.
    pub treasury_full: Fx,
    pub treasury_poor: Fx,
    /// Each side's strength is multiplied by a roll in this inclusive range.
    pub roll: (Fx, Fx),
    pub score_k: Fx,
    pub max_score: Fx,
    /// `Tribute(v)`: the treasury gets `v`, the neighbour loses `v * tribute_strength` strength.
    pub tribute_strength: Fx,
    /// `EndWar(outcome)` adds this to the enemy's strength.
    pub end_strength: Vec<(WarOutcome, Fx)>,
    /// While at war the crown provinces bring this share of their income less.
    #[serde(default)]
    pub income_penalty: Fx,
    /// Yearly upkeep by army size, `curve` points `(army, upkeep)`; see `war::yearly_income`.
    #[serde(default)]
    pub army_upkeep: Vec<(Fx, Fx)>,
    /// Every year the treasury is below 0, or a year of peace whose income (`war::yearly_income`)
    /// is below 0, this share of the army deserts.
    #[serde(default)]
    pub desertion: Fx,
}

/// The dynasty simulation after the reign, see `sim::run`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SimRules {
    /// The dynasty counts as alive after this many years from tick 0.
    pub max_years: u32,
    /// Events of this importance and above go to the chronicle.
    pub threshold: u32,
    /// Importance of the entries made always: a new ruler, a province lost or gained.
    pub notable: u32,
    /// Mark weights are multiplied by this every year.
    pub decay: Fx,
    /// An entry tells the hint of its main cause only from this weight on.
    pub hint_weight: Fx,
    /// The dynasty falls once this flag is set.
    pub usurped_flag: String,
    /// A ruler younger than `regency_age` reigns under `regency_flag` until he reaches it.
    pub regency_flag: String,
    pub regency_age: u32,
    /// Flags that belong to one reign and go when the next ruler is crowned.
    #[serde(default)]
    pub reign_flags: Vec<String>,
    /// The death of the first heir (`texts.heir_died`) is told from this age on; younger,
    /// only when he was the last heir and the dynasty ends without one.
    pub heir_death_age: u32,
    pub ruler_health: Fx,
    /// Relation of a state born of a vassal revolt (`Effect::Secede`) with the kingdom.
    pub secession_relation: Fx,
    pub traits: Vec<TraitRule>,
    pub auto: AutoRules,
    pub texts: SimTexts,
}

/// A new ruler has the trait with chance `percent + ability * ability_k` in percent, plus
/// `studying` or `hostage` when the heir was studying or a hostage.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TraitRule {
    pub id: String,
    pub percent: Fx,
    pub ability_k: Fx,
    pub studying: Fx,
    pub hostage: Fx,
}

/// Weights of `sim::AutoChooser`: `base` plus `traits[t]` of every trait of the ruler.
/// Keys are axis ids, flag ids and effect kinds, see `sim::features`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AutoRules {
    /// Each option gets a roll in `0..=noise` on top of its score.
    pub noise: Fx,
    pub base: BTreeMap<String, Fx>,
    pub traits: BTreeMap<String, BTreeMap<String, Fx>>,
}

/// `(title, text)` of the chronicle entries made by the simulation itself; `{ruler}`,
/// `{province}`, `{neighbour}`, `{heir}`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SimTexts {
    pub crowned: (String, String),
    pub province_lost: (String, String),
    pub province_gained: (String, String),
    /// The heir first in line died (`heirs.death`); `{heir}`.
    pub heir_died: (String, String),
    /// Display text per reign end cause (`Game.ended`: a `RulerDies` cause or the
    /// abdication event id).
    #[serde(default)]
    pub reign_ends: BTreeMap<String, String>,
    /// Display text per fall reason.
    #[serde(default)]
    pub falls: Vec<(FallReason, String)>,
}

#[derive(Debug)]
pub enum DataError {
    Parse(ron::error::SpannedError),
    Invalid(String),
}

/// Parses `rules.ron` contents. The caller does the file I/O.
pub fn load(rules: &str) -> Result<Data, DataError> {
    let data: Data = parse(rules)?;
    if data.time_unit.ticks_per_year == 0 {
        return Err(DataError::Invalid(
            "time_unit.ticks_per_year must be > 0".into(),
        ));
    }
    for a in &data.axes {
        if !(a.min <= a.default && a.default <= a.max) {
            return Err(DataError::Invalid(format!(
                "axis {}: needs min <= default <= max",
                a.id.0
            )));
        }
    }
    let is_axis = |id: &AxisId| data.axes.iter().any(|a| a.id == *id);
    if !is_axis(&data.loyalty_axis) {
        return Err(DataError::Invalid("loyalty_axis is not an axis".into()));
    }
    for f in &data.factions {
        if !is_axis(&f.axis) || data.is_derived(&f.axis) {
            return Err(DataError::Invalid(format!(
                "faction {}: axis must be a plain axis",
                f.id
            )));
        }
    }
    if data.factions.iter().map(|f| f.weight).sum::<u32>() == 0 {
        return Err(DataError::Invalid("faction weights sum to 0".into()));
    }
    let flows = data.economy.flows.iter().map(|(a, _)| a);
    let war = data.war.bonus.iter().map(|(a, _)| a);
    for a in [
        &data.action_slots.axis,
        &data.economy.treasury,
        &data.war.army,
    ]
    .into_iter()
    .chain(flows)
    .chain(war)
    {
        if !is_axis(a) {
            return Err(DataError::Invalid(format!("unknown axis {}", a.0)));
        }
    }
    let w = &data.war;
    if w.treasury_full <= Fx(0) || w.roll.0 > w.roll.1 {
        return Err(DataError::Invalid(
            "war: needs treasury_full > 0 and roll lo <= hi".into(),
        ));
    }
    if data.is_derived(&data.economy.treasury) {
        return Err(DataError::Invalid("economy.treasury is derived".into()));
    }
    for r in &data.death.risks {
        r.when.check(&data).map_err(|m| invalid("death.risks", m))?;
    }
    if !is_axis(&data.abdication.institutions.0) {
        return Err(DataError::Invalid(
            "abdication.institutions: unknown axis".into(),
        ));
    }
    let ai = &data.neighbour_ai;
    for s in [&ai.expand, &ai.defend, &ai.trade, &ai.wait] {
        if s.events.iter().map(|(_, c)| c).sum::<u32>() > 100 {
            return Err(DataError::Invalid(
                "neighbour_ai: chances sum over 100".into(),
            ));
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = include_str!("../../../data/rules.ron");

    #[test]
    fn curve_is_linear_between_points_and_flat_beyond() {
        let pts = [
            (Fx::from_int(1), Fx::from_int(10)),
            (Fx::from_int(3), Fx::from_int(30)),
        ];
        assert_eq!(curve(&pts, Fx(0)), Fx::from_int(10));
        assert_eq!(curve(&pts, Fx::from_int(1)), Fx::from_int(10));
        assert_eq!(curve(&pts, Fx::from_int(2)), Fx::from_int(20));
        assert_eq!(curve(&pts, Fx::from_int(3)), Fx::from_int(30));
        assert_eq!(curve(&pts, Fx::from_int(9)), Fx::from_int(30));
        assert_eq!(curve(&[], Fx::from_int(9)), Fx(0));
    }

    #[test]
    fn rules_ron_loads() {
        let data = load(RULES).unwrap();
        assert_eq!(data.time_unit, TimeUnit { ticks_per_year: 1 });
        assert_eq!(data.axes.len(), 11);
        assert!(data.is_derived(&AxisId("loyalty".into())));
        assert!(!data.is_derived(&AxisId("loyalty_nobles".into())));
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(matches!(load("(time_unit: ())"), Err(DataError::Parse(_))));
        let broken = |from: &str, to: &str| {
            assert!(RULES.contains(from), "{from}");
            load(&RULES.replacen(from, to, 1))
        };
        assert!(matches!(
            broken("ticks_per_year: 1", "ticks_per_year: 0"),
            Err(DataError::Invalid(_))
        ));
        assert!(matches!(
            broken(
                "min: 0, max: 100, default: 50",
                "min: 0, max: 100, default: 101"
            ),
            Err(DataError::Invalid(_))
        ));
        for (from, to) in [
            (r#"loyalty_axis: "loyalty""#, r#"loyalty_axis: "nothing""#),
            (r#"axis: "loyalty_nobles""#, r#"axis: "nothing""#),
            (r#"axis: "loyalty_nobles""#, r#"axis: "loyalty""#),
        ] {
            assert!(
                matches!(broken(from, to), Err(DataError::Invalid(_))),
                "{to}"
            );
        }
        for (from, to) in [
            (r#"axis: "bureaucracy""#, r#"axis: "nothing""#),
            (r#"treasury: "treasury""#, r#"treasury: "nothing""#),
            (r#"treasury: "treasury""#, r#"treasury: "loyalty""#),
            (r#"("income", 1)"#, r#"("nothing", 1)"#),
            (r#"army: "army""#, r#"army: "nothing""#),
            (r#"("loyalty_nobles", 0.005)"#, r#"("nothing", 0.005)"#),
            ("treasury_full: 100", "treasury_full: 0"),
            ("roll: (0.5, 1.5)", "roll: (1.5, 0.5)"),
        ] {
            assert!(
                matches!(broken(from, to), Err(DataError::Invalid(_))),
                "{to}"
            );
        }
        assert!(matches!(
            broken(r#"("neighbour_raid", 25)"#, r#"("neighbour_raid", 76)"#),
            Err(DataError::Invalid(_))
        ));
        assert!(broken(r#"("neighbour_raid", 25)"#, r#"("neighbour_raid", 75)"#).is_ok());
        let no_weight = RULES
            .replace("weight: 2", "weight: 0")
            .replace("weight: 1", "weight: 0");
        assert!(matches!(load(&no_weight), Err(DataError::Invalid(_))));
    }

    const EVENTS: &str = include_str!("../../../data/events/reign.ron");
    const ACTIONS: &str = include_str!("../../../data/actions.ron");
    const NEIGHBOUR_EVENTS: &str = include_str!("../../../data/events/neighbours.ron");

    #[test]
    fn content_loads() {
        let mut data = load(RULES).unwrap();
        data.add_events(EVENTS).unwrap();
        data.add_events(NEIGHBOUR_EVENTS).unwrap();
        data.add_actions(ACTIONS).unwrap();
        assert_eq!(data.events.len(), 38);
        assert_eq!(data.actions.len(), 15);
        // Ids must be unique across files.
        assert!(matches!(
            data.add_events(EVENTS),
            Err(DataError::Invalid(_))
        ));
        assert!(matches!(
            data.add_actions(ACTIONS),
            Err(DataError::Invalid(_))
        ));
        assert_eq!(data.events.len(), 38);
    }

    #[test]
    fn newborns_are_named_by_id() {
        let mut data = load(RULES).unwrap();
        assert_eq!(data.newborn(3).name, data.new_heir.name); // no pool
        data.add_names(r#"(rulers: [], heirs: ["Ада", "Бруно"], vassals: [])"#)
            .unwrap();
        let names = [0, 1, 2, 3].map(|id| data.newborn(id).name);
        assert_eq!(names, ["Ада", "Бруно", "Ада", "Бруно"]);
        assert_eq!(data.newborn(1).ability, data.new_heir.ability);
    }

    #[test]
    fn sim_events_stay_apart() {
        let mut data = load(RULES).unwrap();
        data.add_events(EVENTS).unwrap();
        let sim = include_str!("../../../data/events/sim/sim.ron");
        data.add_sim_events(sim).unwrap();
        assert_eq!(data.events.len(), 34);
        assert!(data.sim_events.iter().any(|e| e.id == "vassal_revolt"));
        // Ids are unique across both lists, whichever comes first.
        assert!(matches!(
            data.add_sim_events(sim),
            Err(DataError::Invalid(_))
        ));
        assert!(matches!(
            data.add_sim_events(EVENTS),
            Err(DataError::Invalid(_))
        ));
        let mut data = load(RULES).unwrap();
        data.add_sim_events(sim).unwrap();
        assert!(matches!(data.add_events(sim), Err(DataError::Invalid(_))));
        data.add_hints(r#"{"a": "b"}"#).unwrap();
        assert_eq!(data.hints["a"], "b");
    }

    #[test]
    fn bad_content_is_rejected() {
        let data = load(RULES).unwrap();
        let event = |when: &str, effect: &str, choices: bool| {
            let choices = match choices {
                true => format!("[(text: \"\", effects: [{effect}], cause_tag: \"t\")]"),
                false => "[]".into(),
            };
            format!(
                "[(id: \"e\", title: \"\", text: \"\", when: {when}, weight: 1, once: false, \
                 cooldown_years: 0, importance: 0, target: None, choices: {choices})]"
            )
        };
        let add = |text: String| data.clone().add_events(&text);
        assert!(add(event("All([])", r#"Axis("legitimacy", 1)"#, true)).is_ok());
        for bad in [
            event("All([])", r#"Axis("loyalty", 1)"#, true), // derived axis
            event("All([])", r#"Axis("nothing", 1)"#, true),
            event(r#"Not(Any([AxisAbove("nothing", 1)]))"#, "", true),
            event(r#"AxisBelow("nothing", 1)"#, "", true),
            event("All([])", "", false),
        ] {
            assert!(
                matches!(add(bad.clone()), Err(DataError::Invalid(_))),
                "{bad}"
            );
        }
        assert!(matches!(add("[(id: 1)]".into()), Err(DataError::Parse(_))));

        let action = |requires: &str, effect: &str| {
            format!(
                "[(id: \"a\", name: \"\", duration_years: 1, cost: 0, requires: {requires}, \
                 min_crown_power: 0, target: None, on_complete: [{effect}], cause_tag: \"t\")]"
            )
        };
        let add = |text: String| data.clone().add_actions(&text);
        assert!(add(action("All([])", r#"Axis("legitimacy", 1)"#)).is_ok());
        assert!(matches!(
            add(action("All([])", r#"Axis("loyalty", 1)"#)),
            Err(DataError::Invalid(_))
        ));
        assert!(matches!(
            add(action(r#"AxisAbove("nothing", 1)"#, "")),
            Err(DataError::Invalid(_))
        ));
    }
}
