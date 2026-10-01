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
    /// Per BFS step from the capital.
    pub distance_penalty: Fx,
    /// Bonus per building id. Buildings not listed give nothing.
    pub buildings: BTreeMap<String, Fx>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HolderBase {
    pub crown: Fx,
    pub vassal: Fx,
    pub foreign: Fx,
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
        assert_eq!(data.axes.len(), 10);
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
    }
}
