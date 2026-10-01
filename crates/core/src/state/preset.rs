use super::{Axes, Capital, Heir, Holder, Neighbour, Province, ProvinceId, Ruler, Vassal};
use crate::data::{Data, DataError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A starting position: everything `World::from_preset` needs besides `rules.ron`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Preset {
    pub start_year: u32,
    /// Overrides of the rule defaults; axes not listed start at their default.
    pub axes: Axes,
    /// Empty when the map lives in its own file, see `load_with_map`.
    #[serde(default)]
    pub map: Map,
    pub capital: Capital,
    pub vassals: Vec<Vassal>,
    pub ruler: Ruler,
    pub heirs: Vec<Heir>,
    pub neighbours: Vec<Neighbour>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Map {
    pub provinces: Vec<Province>,
    /// UI outlines, points in map coordinates. Static, so kept out of `World`.
    pub polygons: BTreeMap<ProvinceId, Vec<(i32, i32)>>,
}

impl Preset {
    /// Parses a preset and checks that its references hold against `data`. No file I/O.
    pub fn load(text: &str, data: &Data) -> Result<Preset, DataError> {
        let preset: Preset = ron::from_str(text).map_err(DataError::Parse)?;
        preset.check(data).map_err(DataError::Invalid)?;
        Ok(preset)
    }

    /// A preset whose map is a separate text (`data/maps/*.ron`); it replaces `map`.
    pub fn load_with_map(text: &str, map: &str, data: &Data) -> Result<Preset, DataError> {
        let mut preset: Preset = ron::from_str(text).map_err(DataError::Parse)?;
        preset.map = ron::from_str(map).map_err(DataError::Parse)?;
        preset.check(data).map_err(DataError::Invalid)?;
        Ok(preset)
    }

    fn check(&self, data: &Data) -> Result<(), String> {
        for (id, v) in &self.axes {
            let def = data.axes.iter().find(|a| a.id == *id);
            let def = def.ok_or(format!("unknown axis {}", id.0))?;
            if data.is_derived(id) {
                return Err(format!("axis {} is derived, a preset cannot set it", id.0));
            }
            if *v < def.min || *v > def.max {
                return Err(format!("axis {} = {v} is out of bounds", id.0));
            }
        }
        let provinces: BTreeMap<_, _> = self.map.provinces.iter().map(|p| (&p.id, p)).collect();
        if !provinces.contains_key(&self.capital.province) {
            return Err(format!(
                "capital {} is not on the map",
                self.capital.province.0
            ));
        }
        for p in &self.map.provinces {
            for n in &p.neighbours {
                // Asymmetric edges would make path costs depend on direction.
                if !provinces
                    .get(n)
                    .is_some_and(|q| q.neighbours.contains(&p.id))
                {
                    return Err(format!("{} -> {}: no edge back", p.id.0, n.0));
                }
            }
            let holder_ok = match &p.holder {
                Holder::Crown => true,
                Holder::Vassal(v) => self.vassals.iter().any(|x| x.id == *v),
                Holder::Foreign(n) => self.neighbours.iter().any(|x| x.id == *n),
            };
            if !holder_ok {
                return Err(format!("{}: unknown holder {:?}", p.id.0, p.holder));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRESET: &str = include_str!("../../../../data/presets/default.ron");
    const MAP: &str = include_str!("../../../../data/maps/default.ron");

    #[test]
    fn every_province_has_an_outline_on_the_map() {
        let data = crate::data::load(include_str!("../../../../data/rules.ron")).unwrap();
        let map = Preset::load_with_map(PRESET, MAP, &data).unwrap().map;
        let ids: Vec<_> = map.provinces.iter().map(|p| &p.id).collect();
        assert_eq!(map.polygons.len(), ids.len());
        for id in ids {
            let poly = &map.polygons[id];
            assert!(poly.len() >= 3, "{}", id.0);
            let inside = |&(x, y): &(i32, i32)| (0..=400).contains(&x) && (0..=300).contains(&y);
            assert!(poly.iter().all(inside), "{}", id.0);
        }
    }

    #[test]
    fn broken_presets_are_rejected() {
        let data = crate::data::load(include_str!("../../../../data/rules.ron")).unwrap();
        assert!(Preset::load_with_map(PRESET, MAP, &data).is_ok());
        // Without its map the default preset has no capital.
        assert!(matches!(
            Preset::load(PRESET, &data),
            Err(DataError::Invalid(_))
        ));
        assert!(matches!(
            Preset::load("()", &data),
            Err(DataError::Parse(_))
        ));
        assert!(matches!(
            Preset::load_with_map(PRESET, "()", &data),
            Err(DataError::Parse(_))
        ));
        for (from, to) in [
            (r#""legitimacy": 60"#, r#""legitimacy": 101"#),
            (r#""legitimacy": 60"#, r#""no_such_axis": 60"#),
            (r#"province: "capital""#, r#"province: "nowhere""#),
            (
                r#""gart", "frostad", "nordheim"]"#,
                r#""gart", "nordheim"]"#,
            ),
            (r#"Vassal("weir")"#, r#"Vassal("nobody")"#),
            (r#"Foreign("nordmark")"#, r#"Foreign("nobody")"#),
            (r#""legitimacy": 60"#, r#""loyalty": 60"#),
        ] {
            let (preset, map) = (PRESET.replacen(from, to, 1), MAP.replacen(from, to, 1));
            assert!(preset != PRESET || map != MAP, "{from}");
            let res = Preset::load_with_map(&preset, &map, &data);
            assert!(matches!(res, Err(DataError::Invalid(_))), "{to}: {res:?}");
        }
    }
}
