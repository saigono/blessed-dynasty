use crate::fx::Fx;
use crate::rules::{Action, Event};
use crate::state::{AxisId, Heir, Stance, Vassal, World};
use crate::time::TimeUnit;
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
    /// What `HeirOp::Add` pushes.
    pub new_heir: Heir,
    pub neighbour_ai: NeighbourAi,
    pub grant: GrantRules,
    /// From `add_events`, not from `rules.ron`.
    #[serde(default)]
    pub events: Vec<Event>,
    /// From `add_actions`, not from `rules.ron`.
    #[serde(default)]
    pub actions: Vec<Action>,
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
        let events: Vec<Event> = parse(text)?;
        let ids = self.events.iter().chain(&events).map(|e| e.id.as_str());
        unique(ids)?;
        for e in &events {
            e.check(self).map_err(|m| invalid(&e.id, m))?;
        }
        self.events.extend(events);
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
    pub expand: StanceRules,
    pub defend: StanceRules,
    pub trade: StanceRules,
    pub wait: StanceRules,
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

/// Who gets a province from `Effect::Grant`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GrantRules {
    /// The nearest vassal within this many border crossings takes the province.
    pub max_distance: u32,
    /// Otherwise the first of these not yet in the world is founded.
    pub new_vassals: Vec<Vassal>,
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
    for a in [&data.action_slots.axis, &data.economy.treasury]
        .into_iter()
        .chain(flows)
    {
        if !is_axis(a) {
            return Err(DataError::Invalid(format!("unknown axis {}", a.0)));
        }
    }
    if data.is_derived(&data.economy.treasury) {
        return Err(DataError::Invalid("economy.treasury is derived".into()));
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
            (r#"("army", -0.1)"#, r#"("nothing", -0.1)"#),
        ] {
            assert!(
                matches!(broken(from, to), Err(DataError::Invalid(_))),
                "{to}"
            );
        }
        assert!(matches!(
            broken(r#"("neighbour_raid", 25)"#, r#"("neighbour_raid", 86)"#),
            Err(DataError::Invalid(_))
        ));
        assert!(broken(r#"("neighbour_raid", 25)"#, r#"("neighbour_raid", 85)"#).is_ok());
        let no_weight = RULES
            .replace("weight: 2", "weight: 0")
            .replace("weight: 1", "weight: 0");
        assert!(matches!(load(&no_weight), Err(DataError::Invalid(_))));
    }

    const EVENTS: &str = include_str!("../../../data/events/test.ron");
    const ACTIONS: &str = include_str!("../../../data/actions.ron");
    const NEIGHBOUR_EVENTS: &str = include_str!("../../../data/events/neighbours.ron");

    #[test]
    fn content_loads() {
        let mut data = load(RULES).unwrap();
        data.add_events(EVENTS).unwrap();
        data.add_events(NEIGHBOUR_EVENTS).unwrap();
        data.add_actions(ACTIONS).unwrap();
        assert_eq!(data.events.len(), 8);
        assert_eq!(data.actions.len(), 7);
        // Ids must be unique across files.
        assert!(matches!(
            data.add_events(EVENTS),
            Err(DataError::Invalid(_))
        ));
        assert!(matches!(
            data.add_actions(ACTIONS),
            Err(DataError::Invalid(_))
        ));
        assert_eq!(data.events.len(), 8);
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
