use crate::fx::Fx;
use crate::state::AxisId;
use crate::time::TimeUnit;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Everything loaded from `rules.ron`. Grows stage by stage.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Data {
    pub time_unit: TimeUnit,
    pub axes: Vec<AxisDef>,
    pub crown_power: CrownPowerRules,
    pub factions: Vec<Faction>,
    /// Derived axis: weighted mean of the faction axes, see `World::recompute_loyalty`.
    pub loyalty_axis: AxisId,
}

impl Data {
    /// Derived axes are recomputed from others; nothing may write to them directly.
    pub fn is_derived(&self, axis: &AxisId) -> bool {
        *axis == self.loyalty_axis
    }
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

#[derive(Debug)]
pub enum DataError {
    Parse(ron::error::SpannedError),
    Invalid(String),
}

/// Parses `rules.ron` contents. The caller does the file I/O.
pub fn load(rules: &str) -> Result<Data, DataError> {
    let data: Data = ron::from_str(rules).map_err(DataError::Parse)?;
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
        let no_weight = RULES
            .replace("weight: 2", "weight: 0")
            .replace("weight: 1", "weight: 0");
        assert!(matches!(load(&no_weight), Err(DataError::Invalid(_))));
    }
}
