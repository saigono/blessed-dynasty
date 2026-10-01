use crate::time::TimeUnit;
use serde::Deserialize;

/// Everything loaded from `rules.ron`. Grows stage by stage.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Data {
    pub time_unit: TimeUnit,
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
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_ron_loads() {
        let data = load(include_str!("../../../data/rules.ron")).unwrap();
        assert_eq!(data.time_unit, TimeUnit { ticks_per_year: 1 });
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(matches!(load("(time_unit: ())"), Err(DataError::Parse(_))));
        assert!(matches!(
            load("(time_unit: (ticks_per_year: 0))"),
            Err(DataError::Invalid(_))
        ));
    }
}
