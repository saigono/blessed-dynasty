use super::{
    Axes, Capital, Heir, Holder, Neighbour, NeighbourId, Province, ProvinceId, Ruler, Vassal,
    VassalId,
};
use crate::data::{Data, DataError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
    /// Flags the world starts with: the succession law (a flag of `heirs.laws`), `married`.
    #[serde(default)]
    pub flags: BTreeSet<String>,
    /// The backstory shown before the first move.
    #[serde(default)]
    pub intro: String,
    /// Stage 26: the neighbours as kingdoms of their own (`realm.rs`); None: numbers only.
    #[serde(default)]
    pub realms: Option<RealmsStart>,
}

/// The foreign kingdoms of a preset. Each starts as our preset does, but for what it sets:
/// the map is the same, its land its crown's (or its vassals', `fiefs`), ours `us`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RealmsStart {
    /// The holder of our land in their worlds.
    pub us: NeighbourId,
    pub kingdoms: Vec<RealmStart>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RealmStart {
    /// Its id among `Preset.neighbours`.
    pub id: NeighbourId,
    /// The ruling house; its cases go to `names.ron` `forms`.
    pub house: String,
    pub capital: ProvinceId,
    pub ruler: Ruler,
    pub heirs: Vec<Heir>,
    /// In place of the preset's: the succession law, `married`.
    #[serde(default)]
    pub flags: BTreeSet<String>,
    /// Over the preset's axes.
    #[serde(default)]
    pub axes: Axes,
    #[serde(default)]
    pub vassals: Vec<Vassal>,
    /// Which of its provinces its vassals hold.
    #[serde(default)]
    pub fiefs: BTreeMap<ProvinceId, VassalId>,
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

    /// The kingdom `r` sees itself: our preset with its ruler, house laws, axes and vassals,
    /// its land its own, ours `RealmsStart.us`. It knows no neighbours: wars, raids and
    /// marriages between kingdoms are for stage 27.
    pub fn realm(&self, r: &RealmStart) -> Preset {
        let realms = self.realms.as_ref().expect("a preset with realms");
        let me = Holder::Foreign(r.id.clone());
        let mut map = self.map.clone();
        for p in &mut map.provinces {
            p.holder = match &p.holder {
                h if *h == me => (r.fiefs.get(&p.id)).map_or(Holder::Crown, |v| Holder::Vassal(v.clone())),
                Holder::Foreign(n) => Holder::Foreign(n.clone()),
                _ => Holder::Foreign(realms.us.clone()),
            };
        }
        let mut axes = self.axes.clone();
        axes.extend(r.axes.clone());
        Preset {
            start_year: self.start_year,
            axes,
            map,
            capital: Capital {
                province: r.capital.clone(),
                ..self.capital.clone()
            },
            vassals: r.vassals.clone(),
            ruler: r.ruler.clone(),
            heirs: r.heirs.clone(),
            neighbours: vec![],
            flags: r.flags.clone(),
            intro: String::new(),
            realms: None,
        }
    }

    fn check(&self, data: &Data) -> Result<(), String> {
        check_axes(&self.axes, data)?;
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
        for r in self.realms.iter().flat_map(|r| &r.kingdoms) {
            let at = |e: String| format!("realm {}: {e}", r.id.0);
            if !self.neighbours.iter().any(|n| n.id == r.id) {
                return Err(at("not a neighbour".into()));
            }
            check_axes(&r.axes, data).map_err(at)?;
            let own = |id: &ProvinceId| {
                let p = self.map.provinces.iter().find(|p| p.id == *id);
                p.is_some_and(|p| p.holder == Holder::Foreign(r.id.clone()))
            };
            if !own(&r.capital) || r.fiefs.contains_key(&r.capital) {
                return Err(at(format!("capital {} is not its crown's", r.capital.0)));
            }
            for (p, v) in &r.fiefs {
                if !own(p) || !r.vassals.iter().any(|x| x.id == *v) {
                    return Err(at(format!("fief {}: not its land or no vassal {}", p.0, v.0)));
                }
            }
        }
        Ok(())
    }
}

fn check_axes(axes: &Axes, data: &Data) -> Result<(), String> {
    for (id, v) in axes {
        let def = data.axes.iter().find(|a| a.id == *id);
        let def = def.ok_or(format!("unknown axis {}", id.0))?;
        if data.is_derived(id) {
            return Err(format!("axis {} is derived, a preset cannot set it", id.0));
        }
        if *v < def.min || *v > def.max {
            return Err(format!("axis {} = {v} is out of bounds", id.0));
        }
    }
    Ok(())
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
            (r#""legitimacy": 45"#, r#""legitimacy": 101"#),
            (r#""legitimacy": 45"#, r#""no_such_axis": 45"#),
            (r#"province: "capital""#, r#"province: "nowhere""#),
            (
                r#""arden", "frostad", "nordheim"]"#,
                r#""arden", "nordheim"]"#,
            ),
            (r#"Vassal("weir")"#, r#"Vassal("nobody")"#),
            (r#"Foreign("nordmark")"#, r#"Foreign("nobody")"#),
            (r#""legitimacy": 45"#, r#""loyalty": 45"#),
        ] {
            let (preset, map) = (PRESET.replacen(from, to, 1), MAP.replacen(from, to, 1));
            assert!(preset != PRESET || map != MAP, "{from}");
            let res = Preset::load_with_map(&preset, &map, &data);
            assert!(matches!(res, Err(DataError::Invalid(_))), "{to}: {res:?}");
        }
    }
}
