//! World state: plain data, constructors and reads. Changes come from the rules engine.

mod preset;

pub use preset::{Map, Preset};

use crate::data::{CrownPowerRules, Data};
use crate::fx::Fx;
use crate::time::{Tick, TimeUnit};
use crate::war::War;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

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
    /// Border crossings from the capital over the whole graph, foreign land included.
    /// Set by `World::from_preset`; the graph never changes, so it stays valid.
    #[serde(default)]
    pub distance_to_capital: u32,
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
pub struct Vassal {
    pub id: VassalId,
    pub name: String,
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
    #[serde(default)]
    pub sex: Sex,
}

/// Rolled at birth (`HeirRules::sex`); a preset sets it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Sex {
    #[default]
    Male,
    Female,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Heir {
    /// Stable across births and deaths, unlike the index; `Target::Heir` holds it.
    /// Assigned by the world (`from_preset`, `add_heir`), so data may omit it.
    #[serde(default)]
    pub id: u32,
    pub name: String,
    pub age: u32,
    pub ability: Fx,
    pub claim: Fx,
    pub status: HeirStatus,
    #[serde(default)]
    pub sex: Sex,
    /// Wed by a marriage (`Effect::Marry`) or the `heir_marriage` event: never twice.
    #[serde(default)]
    pub married: bool,
    /// The calendar year of the wedding; None for one married from the start (a preset),
    /// counted as wed since `heirs.adult_age`.
    #[serde(default)]
    pub married_in: Option<u32>,
    /// Born out of wedlock. Unrecognized, he lives in `World.bastards`, out of the line;
    /// recognized (`HeirOp::Recognize`), he stands in `heirs` after the lawful ones.
    #[serde(default)]
    pub bastard: bool,
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
    /// Strength per province held: `strength` recovers toward this times the provinces the
    /// state holds now (`neighbour_ai.recover`). Set by `World::from_preset` (start strength
    /// over start provinces) and by `Effect::Secede`; 0 means no recovery.
    #[serde(default)]
    pub per_province: Fx,
    /// The order the state came into the world: the preset's in their order, a new one
    /// (`Effect::Secede`) after all. Stable, unlike the position among `World.neighbours`;
    /// the map colours by it.
    #[serde(default)]
    pub ordinal: u32,
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
    pub ruler: Ruler,
    pub heirs: Vec<Heir>,
    pub neighbours: BTreeMap<NeighbourId, Neighbour>,
    pub active_actions: Vec<ActiveAction>,
    pub flags: BTreeSet<String>,
    /// Tick each event last fired at; drives `once` and cooldowns.
    #[serde(default)]
    pub last_fired: BTreeMap<String, Tick>,
    /// Added to the crown power formula; set by `Effect::CrownPower`, drifts to 0.
    #[serde(default)]
    pub crown_modifiers: BTreeMap<ProvinceId, Fx>,
    /// At most one war at a time.
    #[serde(default)]
    pub war: Option<War>,
    /// The id `add_heir` gives next.
    #[serde(default)]
    pub next_heir_id: u32,
    /// Heirs with an id from this on are the reigning ruler's children; older ones are the
    /// collateral line (his brothers and sisters and theirs) and stand after them in `heirs`.
    #[serde(default)]
    pub line_from: u32,
    /// What the player's decisions touched, for the causes of chronicle entries.
    /// Weights decay yearly by `Data.sim.decay`.
    #[serde(default)]
    pub marks: BTreeMap<MarkKey, Vec<CauseTag>>,
    /// The dynasty for the family tree: the founder, then every heir in order of appearance.
    #[serde(default)]
    pub kin: Vec<Kin>,
    /// Years the army deserted for want of pay (`war.desertion`), over the whole game.
    #[serde(default)]
    pub deserted: u32,
    /// Marriages into foreign courts (`Effect::Marry`), one per court. A union ends with its
    /// spouse (an heir dead, a ruler's reign over) or a war with that court.
    #[serde(default)]
    pub unions: BTreeMap<NeighbourId, Union>,
    /// The heir the ruler named to succeed him (`Heir.id`, `HeirOp::Designate`), over the
    /// rule of the law; cleared at a coronation.
    #[serde(default)]
    pub designated: Option<u32>,
    /// Children born out of wedlock, not recognized: out of the line, eldest first.
    #[serde(default)]
    pub bastards: Vec<Heir>,
    /// The source of each edge of `Data.influences` smoothed over its `delay`
    /// (`graph::tick`), in their order; filled on the first tick.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lagged: Vec<Fx>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Union {
    /// The heir wed (`Heir.id`); None: the ruler.
    pub spouse: Option<u32>,
    pub since: Tick,
}

/// A member of the dynasty. Years are calendar years.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Kin {
    /// `Heir.id`; None for the founder.
    pub heir: Option<u32>,
    pub name: String,
    pub born: u32,
    pub died: Option<u32>,
    /// Index in `World.kin` of the ruler whose child this is.
    pub parent: Option<usize>,
    /// The year of the coronation, for rulers.
    pub crowned: Option<u32>,
    #[serde(default)]
    pub bastard: bool,
}

/// What changed between two worlds, see `World::changes`.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// An axis moved by at least its `AxisDef.notable`.
    Axis(AxisId, Fx),
    Born(String),
    /// An heir left the list: died, or was crowned in the simulation.
    HeirGone(String),
    /// A province changed hands: from, to.
    Holder(ProvinceId, Holder, Holder),
    /// An action finished: its id and target key.
    Done(String, Option<String>),
}

/// A part of the world a decision can touch.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MarkKey {
    Axis(AxisId),
    Province(ProvinceId),
    Flag(String),
    Neighbour(NeighbourId),
    /// `Heir.id`.
    Heir(u32),
}

/// A link from a chronicle entry back to the player's decision `decision_idx`
/// (an index into `Game.decisions`), with that decision's `cause_tag`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CauseTag {
    pub decision_idx: usize,
    pub cause_tag: String,
    pub weight: Fx,
}

impl World {
    /// Rule axes at their defaults overridden by the preset, derived values computed.
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
            ruler: preset.ruler.clone(),
            heirs: preset.heirs.clone(),
            neighbours: by_id(&preset.neighbours, |n| n.id.clone()),
            active_actions: Vec::new(),
            flags: preset.flags.clone(),
            last_fired: BTreeMap::new(),
            crown_modifiers: BTreeMap::new(),
            war: None,
            next_heir_id: 0,
            line_from: 0,
            marks: BTreeMap::new(),
            kin: Vec::new(),
            deserted: 0,
            unions: BTreeMap::new(),
            designated: None,
            bastards: Vec::new(),
            lagged: Vec::new(),
        };
        let r = &world.ruler;
        let founder = Kin {
            heir: None,
            name: r.name.clone(),
            born: world.start_year.saturating_sub(r.age),
            died: None,
            parent: None,
            crowned: Some(world.year()),
            bastard: false,
        };
        world.kin.push(founder);
        for h in std::mem::take(&mut world.heirs) {
            world.add_heir(h);
        }
        for (i, n) in preset.neighbours.iter().enumerate() {
            let n = world.neighbours.get_mut(&n.id).expect("from the preset");
            n.ordinal = i as u32;
        }
        for n in world.neighbours.values_mut() {
            let holder = Holder::Foreign(n.id.clone());
            let held = world.provinces.values().filter(|p| p.holder == holder);
            n.per_province = n.strength / Fx::from_int(held.count().max(1) as i64);
        }
        world.recompute_loyalty(data);
        world.recompute_crown_power(data);
        // Unreachable provinces are not expected (the map is connected); they get u32::MAX.
        let hops = world.hops(&world.capital.province);
        for p in world.provinces.values_mut() {
            p.distance_to_capital = hops.get(&p.id).copied().unwrap_or(u32::MAX);
        }
        world
    }

    /// Adds the heir under the next free id, after the ruler's children and before the
    /// collateral line (`line_from`); a bastard goes to `bastards` instead.
    pub fn add_heir(&mut self, mut heir: Heir) {
        heir.id = self.next_heir_id;
        self.next_heir_id += 1;
        self.kin.push(Kin {
            heir: Some(heir.id),
            name: heir.name.clone(),
            born: self.year().saturating_sub(heir.age),
            died: None,
            parent: self.kin.iter().rposition(|k| k.crowned.is_some()),
            crowned: None,
            bastard: heir.bastard,
        });
        match heir.bastard {
            true => self.bastards.push(heir),
            false => self.insert_heir(heir),
        }
    }

    /// Puts the heir in the line: the ruler's child before the collateral line, the
    /// collateral line at its end.
    pub(crate) fn insert_heir(&mut self, heir: Heir) {
        let collateral = |h: &Heir| h.id < self.line_from;
        let at = match collateral(&heir) {
            true => None,
            false => self.heirs.iter().position(collateral),
        };
        self.heirs.insert(at.unwrap_or(self.heirs.len()), heir);
    }

    /// The calendar year now.
    pub fn year(&self) -> u32 {
        self.start_year + self.tick.year(self.time_unit)
    }

    /// Marks dead every heir of `kin` who is neither in `heirs` nor crowned.
    pub(crate) fn bury(&mut self) {
        let year = self.year();
        for k in &mut self.kin {
            let gone = |id| !self.heirs.iter().chain(&self.bastards).any(|h| h.id == id);
            if k.died.is_none() && k.crowned.is_none() && k.heir.is_some_and(gone) {
                k.died = Some(year);
            }
        }
        // A union ends with its heir.
        let heirs = &self.heirs;
        (self.unions).retain(|_, u| u.spouse.is_none_or(|id| heirs.iter().any(|h| h.id == id)));
    }

    /// What changed since `before`: notable axis moves, births, heirs gone, provinces that
    /// changed hands, finished actions; in that order.
    pub fn changes(&self, before: &World, data: &Data) -> Vec<Change> {
        let axes = (data.axes.iter()).filter_map(|a| {
            let delta = self.axes[&a.id] - before.axes[&a.id];
            (a.notable > Fx(0) && Fx(delta.0.abs()) >= a.notable)
                .then(|| Change::Axis(a.id.clone(), delta))
        });
        let new = |a: &[Heir], b: &[Heir]| {
            let ids: BTreeSet<u32> = b.iter().map(|h| h.id).collect();
            (a.iter().filter(|h| !ids.contains(&h.id)))
                .map(|h| h.name.clone())
                .collect::<Vec<_>>()
        };
        // A bastard recognized joins the heirs, but is no birth.
        let lawful: Vec<Heir> = self.heirs.iter().filter(|h| !h.bastard).cloned().collect();
        let born = new(&lawful, &before.heirs).into_iter().map(Change::Born);
        let gone = new(&before.heirs, &self.heirs)
            .into_iter()
            .map(Change::HeirGone);
        let holders = (self.provinces.values()).filter_map(|p| {
            let was = &before.provinces.get(&p.id)?.holder;
            (*was != p.holder).then(|| Change::Holder(p.id.clone(), was.clone(), p.holder.clone()))
        });
        let done = (before.active_actions.iter())
            .filter(|a| !self.active_actions.contains(a))
            .map(|a| Change::Done(a.id.clone(), a.target.clone()));
        (axes.chain(born).chain(gone).chain(holders).chain(done)).collect()
    }

    /// Index in `heirs` of the heir with this id.
    pub fn heir_index(&self, id: u32) -> Option<usize> {
        self.heirs.iter().position(|h| h.id == id)
    }

    /// Foreign states owning a province next to this one, its own holder excluded.
    pub fn foreign_neighbours(&self, id: &ProvinceId) -> BTreeSet<NeighbourId> {
        let p = self.provinces.get(id);
        p.map_or(BTreeSet::new(), |p| self.foreign_of(p).cloned().collect())
    }

    /// `foreign_neighbours` without allocating, repeats possible.
    pub(crate) fn foreign_of<'a>(
        &'a self,
        p: &'a Province,
    ) -> impl Iterator<Item = &'a NeighbourId> {
        let near = p.neighbours.iter().filter_map(|n| self.provinces.get(n));
        near.filter_map(move |q| match &q.holder {
            Holder::Foreign(n) if q.holder != p.holder => Some(n),
            _ => None,
        })
    }

    /// For a vassal's province: the vassal's strength times the number of his provinces over
    /// the crown power here (at least 0.001). 1 means the vassal matches the crown on his land.
    pub fn vassal_ratio(&self, p: &Province) -> Option<Fx> {
        let Holder::Vassal(v) = &p.holder else {
            return None;
        };
        let strength = self.vassals.get(v)?.strength;
        let held = self
            .provinces
            .values()
            .filter(|q| q.holder == p.holder)
            .count();
        Some(strength * Fx::from_int(held as i64) / p.crown_power.max(Fx(1)))
    }

    /// The own province on the border with `n` with the weakest crown power, smallest id
    /// on a tie: where that neighbour presses.
    pub fn weakest_border(&self, n: &NeighbourId) -> Option<&Province> {
        let theirs =
            (self.provinces.values()).filter(|p| matches!(&p.holder, Holder::Foreign(x) if x == n));
        // Borders are symmetric (Preset::load): our side is among the neighbours of theirs.
        // Starting there saves most lookups; this runs for every neighbour every year.
        let near = theirs.flat_map(|p| &p.neighbours);
        (near.filter_map(|id| self.provinces.get(id)))
            .filter(|p| !matches!(p.holder, Holder::Foreign(_)))
            .min_by_key(|p| (p.crown_power, &p.id))
    }

    /// Fewest border crossings from `from` to every reachable province (BFS).
    pub fn hops(&self, from: &ProvinceId) -> BTreeMap<ProvinceId, u32> {
        let mut hops = BTreeMap::from([(from.clone(), 0)]);
        let mut queue = std::collections::VecDeque::from([from.clone()]);
        while let Some(id) = queue.pop_front() {
            let d = hops[&id];
            for n in self.provinces.get(&id).map_or(&[][..], |p| &p.neighbours) {
                if !hops.contains_key(n) && self.provinces.contains_key(n) {
                    hops.insert(n.clone(), d + 1);
                    queue.push_back(n.clone());
                }
            }
        }
        hops
    }

    /// A copy with empty `marks`, which grow with every decision: what `Game::mark` compares.
    pub(crate) fn without_marks(&mut self) -> World {
        let marks = std::mem::take(&mut self.marks);
        let copy = self.clone();
        self.marks = marks;
        copy
    }

    /// A frozen copy for the chronicle.
    pub fn snapshot(&self) -> World {
        self.clone()
    }

    /// Sets `loyalty_axis` to the weighted mean of the faction axes, rounded toward zero.
    pub fn recompute_loyalty(&mut self, data: &Data) {
        let total: i64 = data.factions.iter().map(|f| f.weight as i64).sum();
        let sum: i64 = data
            .factions
            .iter()
            .map(|f| self.axes[&f.axis].0 * f.weight as i64)
            .sum();
        self.axes.insert(data.loyalty_axis.clone(), Fx(sum / total));
    }

    /// Recomputes `crown_power` of every province from scratch; formula in `rules.ron`.
    /// Foreign provinces and provinces cut off from the capital get 0.
    pub fn recompute_crown_power(&mut self, data: &Data) {
        let r = &data.crown_power;
        let costs = self.path_costs(r);
        for (p, path) in self.provinces.values_mut().zip(costs) {
            let base = match p.holder {
                Holder::Crown => Some(r.base.crown),
                Holder::Vassal(_) => Some(r.base.vassal),
                Holder::Foreign(_) => None,
            };
            let (Some(base), Some(path)) = (base, path) else {
                p.crown_power = Fx(0);
                continue;
            };
            let disloyalty = (r.loyalty_threshold - p.loyalty).max(Fx(0)) * r.loyalty_penalty;
            let distance = path * r.distance_penalty;
            let buildings = p.buildings.iter().filter_map(|b| r.buildings.get(b));
            let buildings = buildings.fold(Fx(0), |sum, &b| sum + b);
            let capital = if p.id == self.capital.province {
                self.capital.crown_bonus
            } else {
                Fx(0)
            };
            let modifier = self.crown_modifiers.get(&p.id).copied().unwrap_or_default();
            p.crown_power =
                (base - disloyalty - distance + buildings + capital + modifier).max(Fx(0));
        }
    }

    /// Cheapest path cost from the capital over `neighbours` (Dijkstra), in province order;
    /// None for cut-off provinces. Runs every tick, so it works on indices.
    fn path_costs(&self, r: &CrownPowerRules) -> Vec<Option<Fx>> {
        let one = Fx::from_int(1);
        let ps: Vec<&Province> = self.provinces.values().collect();
        // Ids are found by a hash first: comparing strings was most of the time here.
        let hash = |id: &ProvinceId| {
            (id.0.bytes()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
            })
        };
        let mut by_hash: Vec<(u64, usize)> = (ps.iter().enumerate())
            .map(|(i, p)| (hash(&p.id), i))
            .collect();
        by_hash.sort_unstable();
        let find = |id: &ProvinceId| {
            let h = hash(id);
            let from = by_hash.partition_point(|(x, _)| *x < h);
            let same = by_hash[from..].iter().take_while(|(x, _)| *x == h);
            same.map(|(_, i)| *i).find(|&i| ps[i].id == *id)
        };
        let steps: Vec<Fx> = (ps.iter())
            .map(|into| match &into.holder {
                Holder::Foreign(n) => {
                    let relation = self.neighbours.get(n).map_or(Fx(0), |n| n.relation);
                    (one + r.foreign_step_penalty - relation * r.foreign_step_relation).max(one)
                }
                _ => one,
            })
            .collect();
        let mut costs = vec![None; ps.len()];
        // Min-heap; equal costs pop in index (ProvinceId) order, so ties are deterministic.
        let mut heap: BinaryHeap<_> = find(&self.capital.province)
            .map(|i| Reverse((Fx(0), i)))
            .into_iter()
            .collect();
        while let Some(Reverse((cost, i))) = heap.pop() {
            if costs[i].is_some() {
                continue;
            }
            costs[i] = Some(cost);
            for j in ps[i].neighbours.iter().filter_map(find) {
                if costs[j].is_none() {
                    heap.push(Reverse((cost + steps[j], j)));
                }
            }
        }
        costs
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
        let map = include_str!("../../../../data/maps/default.ron");
        let preset = Preset::load_with_map(text, map, &data).unwrap();
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
        w.heirs.push(w.heirs[0].clone());
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
        w.last_fired.insert("plague".into(), Tick(3));
        w.crown_modifiers.insert(pid("holm"), Fx(-500));
        w.war = Some(War {
            enemy: NeighbourId("nordmark".into()),
            stage: crate::war::WarStage::Peace,
            our_strength: Fx(60_500),
            their_strength: Fx::from_int(40),
            war_score: Fx(-12_250),
            started: Tick(2),
            target: None,
            battles: vec![],
        });
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
        assert_eq!(w.provinces.len(), 20);
        assert_eq!(w.axes.len(), data.axes.len());
        assert_eq!(w.start_year, 1187);
        // Preset override wins over the rules default, the rest keep defaults.
        assert_eq!(w.axes[&AxisId("legitimacy".into())], Fx::from_int(45));
        assert_eq!(w.axes[&AxisId("army".into())], Fx::from_int(50));
        // Derived on load: nobles 40 * 2 + church 60 + people 50 (preset), over 4.
        assert_eq!(w.axes[&AxisId("loyalty".into())], Fx(47_500));
    }

    #[test]
    fn default_map_is_connected() {
        let (_, w) = world();
        let id = |p: &Province| p.id.0.clone();
        let foreign = |n: &str| {
            let of = |p: &&Province| p.holder == Holder::Foreign(NeighbourId(n.into()));
            w.provinces.values().filter(of).count()
        };
        assert_eq!(
            (foreign("nordmark"), foreign("purpur"), foreign("vestrum")),
            (3, 4, 3)
        );
        // The whole graph is reachable; the cached distance is the BFS one.
        let hops = w.hops(&w.capital.province);
        assert_eq!(hops.len(), 20);
        for p in w.provinces.values() {
            assert_eq!(p.distance_to_capital, hops[&p.id], "{}", id(p));
        }
        assert_eq!(w.provinces[&pid("capital")].distance_to_capital, 0);
        assert_eq!(w.provinces[&pid("gart")].distance_to_capital, 2);
        // Every province of the kingdom reaches the capital over its own land.
        let mut own = w.clone();
        own.provinces
            .retain(|_, p| !matches!(p.holder, Holder::Foreign(_)));
        assert_eq!(own.provinces.len(), 10);
        let reach = own.hops(&w.capital.province);
        assert_eq!(reach.len(), 10);
    }

    #[test]
    fn unreachable_province_is_infinitely_far() {
        let data = data();
        let mut preset = Preset::load_with_map(
            include_str!("../../../../data/presets/default.ron"),
            include_str!("../../../../data/maps/default.ron"),
            &data,
        )
        .unwrap();
        for p in &mut preset.map.provinces {
            p.neighbours.retain(|n| n.0 != "kirm");
            if p.id.0 == "kirm" {
                p.neighbours.clear();
            }
        }
        let w = World::from_preset(&data, &preset);
        assert_eq!(w.provinces[&pid("kirm")].distance_to_capital, u32::MAX);
    }

    #[test]
    fn foreign_neighbours_of_a_province() {
        let (_, w) = world();
        let of = |p: &str| {
            let ns = w.foreign_neighbours(&pid(p));
            ns.into_iter().map(|n| n.0).collect::<Vec<_>>()
        };
        assert_eq!(of("sol"), ["purpur", "vestrum"]);
        assert_eq!(of("arden"), ["nordmark"]);
        assert!(of("capital").is_empty());
        // A foreign province: other states only, not its own holder.
        assert!(of("nordheim").is_empty());
        assert_eq!(of("skala"), ["purpur"]);
        assert!(of("nowhere").is_empty());
        let weakest = |n: &str| {
            w.weakest_border(&NeighbourId(n.into()))
                .map(|p| p.id.0.clone())
        };
        assert_eq!(weakest("nordmark").as_deref(), Some("arden")); // 22.5 vs holm 25
        assert_eq!(weakest("nobody"), None);
    }

    #[test]
    fn heirs_get_stable_ids() {
        let (data, mut w) = world();
        assert_eq!((w.heirs[0].id, w.next_heir_id), (0, 1));
        w.add_heir(data.new_heir.clone());
        w.add_heir(data.new_heir.clone());
        w.heirs.remove(1);
        w.add_heir(data.new_heir.clone());
        let ids: Vec<_> = w.heirs.iter().map(|h| h.id).collect();
        assert_eq!(ids, [0, 2, 3], "a removed id is not given again");
        assert_eq!((w.heir_index(3), w.heir_index(1)), (Some(2), None));
    }

    #[test]
    fn loyalty_follows_faction_axes() {
        let (data, mut w) = world();
        let mut with_nobles = |v: Fx| {
            w.axes.insert(AxisId("loyalty_nobles".into()), v);
            w.recompute_loyalty(&data);
            w.axes[&AxisId("loyalty".into())]
        };
        assert_eq!(with_nobles(Fx::from_int(61)), Fx(58_000)); // (122 + 60 + 50) / 4
        assert_eq!(with_nobles(Fx(50_001)), Fx(52_500)); // 52.5005 rounds toward zero
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
        // capital: crown 60 + fort 10 + crown_bonus 20, path 0, loyal.
        assert_eq!(power(&w, "capital"), Fx::from_int(90));
        // holm: vassal 30 - (50 - 40) * 0.5 - path 1 * 5 + road 5.
        assert_eq!(power(&w, "holm"), Fx::from_int(25));
        // Modifiers from effects add on top.
        w.crown_modifiers.insert(pid("holm"), Fx::from_int(-3));
        w.recompute_crown_power(&data);
        assert_eq!(power(&w, "holm"), Fx::from_int(22));
        w.crown_modifiers.clear();
        // Foreign provinces are not counted.
        assert_eq!(power(&w, "nordheim"), Fx(0));
        // Cut off from the capital: no crown power at all.
        for p in w.provinces.values_mut() {
            p.neighbours.clear();
        }
        w.recompute_crown_power(&data);
        assert_eq!(power(&w, "holm"), Fx(0));
        assert_eq!(power(&w, "capital"), Fx::from_int(90));
    }

    /// capital - nordheim (foreign) - far, plus an own detour capital - a - b - far.
    fn path_world(detour: bool) -> (Data, World) {
        let (data, mut w) = world();
        let p = |id: &str, holder: Holder, neighbours: &[&str]| Province {
            id: pid(id),
            name: id.into(),
            income: Fx(0),
            population: 0,
            loyalty: Fx::from_int(50),
            holder,
            buildings: BTreeSet::new(),
            neighbours: neighbours.iter().map(|n| pid(n)).collect(),
            crown_power: Fx(0),
            distance_to_capital: 0,
        };
        let nordmark = Holder::Foreign(NeighbourId("nordmark".into()));
        let mut map = vec![p("nordheim", nordmark, &["capital", "far"])];
        if detour {
            map.extend([
                p("capital", Holder::Crown, &["nordheim", "a"]),
                p("far", Holder::Crown, &["nordheim", "b"]),
                p("a", Holder::Crown, &["capital", "b"]),
                p("b", Holder::Crown, &["a", "far"]),
            ]);
        } else {
            map.extend([
                p("capital", Holder::Crown, &["nordheim"]),
                p("far", Holder::Crown, &["nordheim"]),
            ]);
        }
        w.provinces = map.into_iter().map(|p| (p.id.clone(), p)).collect();
        (data, w)
    }

    fn far_cost(w: &mut World, data: &Data, relation: i64) -> Fx {
        let n = w
            .neighbours
            .get_mut(&NeighbourId("nordmark".into()))
            .unwrap();
        n.relation = Fx::from_int(relation);
        let far = w.provinces.keys().position(|id| *id == pid("far")).unwrap();
        w.path_costs(&data.crown_power)[far].unwrap()
    }

    #[test]
    fn friendly_neighbour_is_cheaper_to_cross() {
        let (data, mut w) = path_world(false);
        // Step into nordheim: max(1, 1 + 1 - relation * 0.02), then 1 into far.
        assert_eq!(far_cost(&mut w, &data, 100), Fx::from_int(2));
        assert_eq!(far_cost(&mut w, &data, 0), Fx::from_int(3));
        assert_eq!(far_cost(&mut w, &data, -100), Fx::from_int(5));
    }

    #[test]
    fn own_detour_wins_when_cheaper() {
        let (data, mut w) = path_world(true);
        assert_eq!(far_cost(&mut w, &data, 100), Fx::from_int(2)); // through the friend
        assert_eq!(far_cost(&mut w, &data, -100), Fx::from_int(3)); // around the enemy
        w.recompute_crown_power(&data);
        assert_eq!(power(&w, "far"), Fx::from_int(45)); // 60 - path 3 * 5
    }
}
