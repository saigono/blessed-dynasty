//! World state: plain data, constructors and reads. Changes come from the rules engine.

mod preset;

pub use preset::{Map, Preset};

use crate::data::Data;
use crate::fx::Fx;
use crate::time::{Tick, TimeUnit};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct AxisId(pub String);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct ProvinceId(pub String);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct VassalId(pub String);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct NeighbourId(pub String);

/// Axis values. Which axes exist and their bounds come from `rules.ron`.
pub type Axes = BTreeMap<AxisId, Fx>;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Province {
    pub id: ProvinceId,
    pub name: String,
    pub income: Fx,
    pub population: i32,
    pub loyalty: Fx,
    pub holder: Holder,
    pub buildings: BTreeSet<String>,
    pub neighbours: Vec<ProvinceId>,
    /// Derived by `World::recompute_crown_power`, so data may omit it.
    #[serde(default)]
    pub crown_power: Fx,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Holder {
    Crown,
    Vassal(VassalId),
    Foreign(NeighbourId),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Capital {
    pub province: ProvinceId,
    pub crown_bonus: Fx,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Faction {
    pub id: String,
    pub loyalty: Fx,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Vassal {
    pub id: VassalId,
    pub name: String,
    pub provinces: Vec<ProvinceId>,
    pub loyalty: Fx,
    pub strength: Fx,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Ruler {
    pub name: String,
    pub age: u32,
    pub health: Fx,
    pub traits: BTreeSet<String>,
    pub reign_start: Tick,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Heir {
    pub name: String,
    pub age: u32,
    pub ability: Fx,
    pub claim: Fx,
    pub status: HeirStatus,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum HeirStatus {
    Home,
    Hostage(NeighbourId),
    /// Where the heir studies.
    Studying(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Neighbour {
    pub id: NeighbourId,
    pub name: String,
    pub relation: Fx,
    pub strength: Fx,
    pub stance: Stance,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Stance {
    Expand,
    Defend,
    Trade,
    Wait,
}

/// Placeholder until the rules engine (stage 2) defines actions.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ActiveAction {
    pub id: String,
    pub target: Option<String>,
    pub ends_at: Tick,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct World {
    pub tick: Tick,
    pub time_unit: TimeUnit,
    /// Calendar year of tick 0, from the preset. For `Tick::date`.
    pub start_year: u32,
    pub axes: Axes,
    pub provinces: BTreeMap<ProvinceId, Province>,
    pub capital: Capital,
    pub vassals: BTreeMap<VassalId, Vassal>,
    pub factions: BTreeMap<String, Faction>,
    pub ruler: Ruler,
    pub heirs: Vec<Heir>,
    pub neighbours: BTreeMap<NeighbourId, Neighbour>,
    pub active_actions: Vec<ActiveAction>,
    pub flags: BTreeSet<String>,
}

impl World {
    /// Rule axes at their defaults overridden by the preset, crown power computed.
    /// Expects a preset that passed `Preset::load` against the same `data`.
    pub fn from_preset(data: &Data, preset: &Preset) -> World {
        let mut axes: Axes = data
            .axes
            .iter()
            .map(|a| (a.id.clone(), a.default))
            .collect();
        axes.extend(preset.axes.clone());
        let mut world = World {
            tick: Tick(0),
            time_unit: data.time_unit,
            start_year: preset.start_year,
            axes,
            provinces: by_id(&preset.map.provinces, |p| p.id.clone()),
            capital: preset.capital.clone(),
            vassals: by_id(&preset.vassals, |v| v.id.clone()),
            factions: by_id(&preset.factions, |f| f.id.clone()),
            ruler: preset.ruler.clone(),
            heirs: preset.heirs.clone(),
            neighbours: by_id(&preset.neighbours, |n| n.id.clone()),
            active_actions: Vec::new(),
            flags: BTreeSet::new(),
        };
        world.recompute_crown_power(data);
        world
    }

    /// A frozen copy for the chronicle.
    pub fn snapshot(&self) -> World {
        self.clone()
    }

    /// Recomputes `crown_power` of every province from scratch; formula in `rules.ron`.
    pub fn recompute_crown_power(&mut self, data: &Data) {
        let r = &data.crown_power;
        let dist = self.distances_from_capital();
        // Cut off from the capital counts as farther than any reachable province.
        let unreachable = self.provinces.len() as i64;
        for p in self.provinces.values_mut() {
            let base = match p.holder {
                Holder::Crown => r.base.crown,
                Holder::Vassal(_) => r.base.vassal,
                Holder::Foreign(_) => r.base.foreign,
            };
            let disloyalty = (r.loyalty_threshold - p.loyalty).max(Fx(0)) * r.loyalty_penalty;
            let steps = dist.get(&p.id).map_or(unreachable, |&d| d as i64);
            let distance = Fx::from_int(steps) * r.distance_penalty;
            let buildings = p.buildings.iter().filter_map(|b| r.buildings.get(b));
            let buildings = buildings.fold(Fx(0), |sum, &b| sum + b);
            let capital = if p.id == self.capital.province {
                self.capital.crown_bonus
            } else {
                Fx(0)
            };
            p.crown_power = (base - disloyalty - distance + buildings + capital).max(Fx(0));
        }
    }

    /// BFS steps from the capital over `neighbours`. Unreachable provinces are absent.
    fn distances_from_capital(&self) -> BTreeMap<ProvinceId, u32> {
        let start = self.capital.province.clone();
        let mut dist = BTreeMap::from([(start.clone(), 0)]);
        let mut queue = VecDeque::from([start]);
        while let Some(id) = queue.pop_front() {
            let Some(p) = self.provinces.get(&id) else {
                continue;
            };
            let d = dist[&id] + 1;
            for n in &p.neighbours {
                if !dist.contains_key(n) {
                    dist.insert(n.clone(), d);
                    queue.push_back(n.clone());
                }
            }
        }
        dist
    }
}

fn by_id<K: Ord, V: Clone>(items: &[V], key: impl Fn(&V) -> K) -> BTreeMap<K, V> {
    items.iter().map(|v| (key(v), v.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Data {
        crate::data::load(include_str!("../../../../data/rules.ron")).unwrap()
    }

    fn world() -> (Data, World) {
        let data = data();
        let text = include_str!("../../../../data/presets/default.ron");
        let preset = Preset::load(text, &data).unwrap();
        let world = World::from_preset(&data, &preset);
        (data, world)
    }

    fn pid(s: &str) -> ProvinceId {
        ProvinceId(s.into())
    }

    fn power(w: &World, id: &str) -> Fx {
        w.provinces[&pid(id)].crown_power
    }

    #[test]
    fn world_roundtrips_through_ron() {
        let (_, mut w) = world();
        // Exercise every enum variant and the optional fields.
        w.heirs[0].status = HeirStatus::Hostage(NeighbourId("nordmark".into()));
        w.heirs[1].status = HeirStatus::Studying("Монастырь".into());
        w.active_actions.push(ActiveAction {
            id: "build_fort".into(),
            target: Some("holm".into()),
            ends_at: Tick(4),
        });
        w.active_actions.push(ActiveAction {
            id: "wait".into(),
            target: None,
            ends_at: Tick(1),
        });
        w.flags.insert("married".into());
        w.axes.insert(AxisId("treasury".into()), Fx(-1_250));
        for (i, stance) in [Stance::Expand, Stance::Defend, Stance::Trade, Stance::Wait]
            .into_iter()
            .enumerate()
        {
            let mut n = w.neighbours[&NeighbourId("nordmark".into())].clone();
            n.id = NeighbourId(format!("n{i}"));
            n.stance = stance;
            w.neighbours.insert(n.id.clone(), n);
        }
        let text = ron::ser::to_string_pretty(&w, Default::default()).unwrap();
        let back: World = ron::from_str(&text).unwrap();
        assert_eq!(back, w);
        assert_eq!(w.snapshot(), w);
    }

    #[test]
    fn preset_matches_data() {
        let (data, w) = world();
        assert_eq!(w.provinces.len(), 3);
        assert_eq!(w.axes.len(), data.axes.len());
        assert_eq!(w.start_year, 1187);
        // Preset override wins over the rules default, the rest keep defaults.
        assert_eq!(w.axes[&AxisId("legitimacy".into())], Fx::from_int(60));
        assert_eq!(w.axes[&AxisId("bureaucracy".into())], Fx::from_int(20));
    }

    #[test]
    fn capital_beats_vassal_province() {
        let (_, w) = world();
        assert!(power(&w, "capital") > power(&w, "holm"));
    }

    #[test]
    fn low_loyalty_costs_crown_power() {
        let (data, mut w) = world();
        let mut at = |loyalty: i64| {
            w.provinces.get_mut(&pid("holm")).unwrap().loyalty = Fx::from_int(loyalty);
            w.recompute_crown_power(&data);
            power(&w, "holm")
        };
        let (high, low) = (at(80), at(20));
        assert!(low < high, "{low} vs {high}");
        // Above the threshold loyalty does not matter.
        assert_eq!(at(60), high);
    }

    #[test]
    fn crown_power_formula() {
        let (data, mut w) = world();
        // capital: crown 60 + fort 10 + crown_bonus 20, distance 0, loyal.
        assert_eq!(power(&w, "capital"), Fx::from_int(90));
        // holm: vassal 30 - (50 - 40) * 0.5 - 1 step * 5 + road 5.
        assert_eq!(power(&w, "holm"), Fx::from_int(25));
        // nordheim: foreign 0 - 2 steps * 5 clamps to 0.
        assert_eq!(power(&w, "nordheim"), Fx(0));
        // Cut off from the capital: distance counts as 3 steps (number of provinces).
        for p in w.provinces.values_mut() {
            p.neighbours.clear();
        }
        w.recompute_crown_power(&data);
        assert_eq!(power(&w, "holm"), Fx::from_int(15));
        assert_eq!(power(&w, "capital"), Fx::from_int(90));
    }
}
