//! War: one at a time, fought through the event chain of `data/events/war.ron`.
//! The effects that drive it live in `rules::Effect`; the numbers in `rules.ron` `war`.

use crate::data::Data;
use crate::fx::Fx;
use crate::rng::Rng;
use crate::state::{Holder, NeighbourId, ProvinceId, World};
use crate::time::Tick;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct War {
    pub enemy: NeighbourId,
    pub stage: WarStage,
    /// As of the last clash; before the first, as of the declaration.
    pub our_strength: Fx,
    pub their_strength: Fx,
    /// Above 0 we are winning.
    pub war_score: Fx,
    pub started: Tick,
    /// The enemy province the war is fought for: what victory takes
    /// (`ProvinceTarget::WarTarget`). Set by `StartWar`.
    #[serde(default)]
    pub target: Option<ProvinceId>,
    /// Every clash so far: its tick and how it moved `war_score`.
    #[serde(default)]
    pub battles: Vec<(Tick, Fx)>,
}

/// Set by `StartWar` (`Declared`), then by `Effect::SetWarStage` from the chain's events,
/// whose `when` waits for the stage: a step spawned twice, or left over from an ended war,
/// is dropped.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum WarStage {
    Declared,
    Fighting,
    /// Terms of peace: what is left is the outcome.
    Peace,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum WarOutcome {
    Victory,
    Draw,
    Defeat,
}

/// `(ours, theirs)`. Ours: `army * (1 + fort_bonus + sum(axis * k)) * treasury_factor`,
/// fort_bonus per own province with the fort on the border with `enemy`, treasury_factor
/// from `treasury_poor` at an empty treasury up to 1 at `treasury_full`. Theirs: `strength`.
pub fn strengths(w: &World, data: &Data, enemy: &NeighbourId) -> (Fx, Fx) {
    let r = &data.war;
    let (fort, fort_k) = &r.fort;
    let forts = (w.provinces.values())
        .filter(|p| !matches!(p.holder, Holder::Foreign(_)) && p.buildings.contains(fort))
        .filter(|p| w.foreign_neighbours(&p.id).contains(enemy))
        .count();
    let bonus = r.bonus.iter().map(|(a, k)| w.axes[a] * *k);
    let one = Fx::from_int(1);
    let mult = bonus.fold(one + *fort_k * Fx::from_int(forts as i64), |s, b| s + b);
    let filled = (w.axes[&data.economy.treasury] / r.treasury_full).clamp(Fx(0), one);
    let treasury = r.treasury_poor + (one - r.treasury_poor) * filled;
    let ours = w.axes[&r.army] * mult * treasury;
    let theirs = w.neighbours.get(enemy).map_or(Fx(0), |n| n.strength);
    (ours, theirs)
}

/// One battle: each side's strength times a roll in `roll`, ours first;
/// `war_score += (ours - theirs) * score_k`, clamped to `±max_score`. No-op without a war.
pub fn clash(w: &mut World, data: &Data, rng: &mut Rng) {
    let Some(enemy) = w.war.as_ref().map(|war| war.enemy.clone()) else {
        return;
    };
    let r = &data.war;
    let (ours, theirs) = strengths(w, data, &enemy);
    let mut roll = || Fx(rng.range(r.roll.0.0, r.roll.1.0 + 1));
    let delta = (ours * roll() - theirs * roll()) * r.score_k;
    let tick = w.tick;
    let war = w.war.as_mut().expect("checked above");
    let before = war.war_score;
    war.war_score = (war.war_score + delta).clamp(Fx(0) - r.max_score, r.max_score);
    war.battles.push((tick, war.war_score - before));
    (war.our_strength, war.their_strength) = (ours, theirs);
}

/// `n`'s province next to the kingdom closest to the capital, smallest id on a tie: what a
/// war on `n` is fought for unless the crown names its target.
pub fn enemy_border(w: &World, n: &NeighbourId) -> Option<ProvinceId> {
    let own = |q: &ProvinceId| {
        (w.provinces.get(q)).is_some_and(|q| !matches!(q.holder, Holder::Foreign(_)))
    };
    (w.provinces.values())
        .filter(|p| p.holder == Holder::Foreign(n.clone()) && p.neighbours.iter().any(own))
        .min_by_key(|p| p.distance_to_capital)
        .map(|p| p.id.clone())
}

/// What the treasury gains in a year: `Economy::yearly_income`, less `income_penalty` of the
/// crown provinces' income while at war, less `army_upkeep` at the army's size, less
/// `crown_capacity.income` per province beyond the crown's room.
pub fn yearly_income(w: &World, data: &Data) -> Fx {
    let (income, upkeep) = income_parts(w, data);
    income - upkeep
}

/// `yearly_income` as (income, upkeep): the crown provinces and the positive `economy.flows`,
/// treasury flow edges (`graph::treasury_flows`) and `LawDef.treasury` of the laws in force;
/// the negative flows, the war's toll, the army and the land beyond the crown's room.
pub fn income_parts(w: &World, data: &Data) -> (Fx, Fx) {
    let r = &data.war;
    let crown = w.provinces.values().filter(|p| p.holder == Holder::Crown);
    let land = crown.fold(Fx(0), |s, p| s + p.income);
    let flows = data.economy.flows.iter().map(|(a, k)| w.axes[a] * *k);
    let flows = flows.chain(crate::graph::treasury_flows(data, w));
    let flows = flows.chain(data.laws_in_force(w).map(|l| l.treasury));
    let (gain, cost) = flows.fold((Fx(0), Fx(0)), |(g, c), f| match f > Fx(0) {
        true => (g + f, c),
        false => (g, c - f),
    });
    let war = match w.war {
        Some(_) => land * r.income_penalty,
        None => Fx(0),
    };
    let c = &data.crown_capacity;
    let over = match c.income == Fx(0) {
        true => Fx(0),
        false => c.income * Fx::from_int(c.over(w).len() as i64),
    };
    let army = crate::data::curve(&r.army_upkeep, w.axes[&r.army]);
    (land + gain, cost + war + army + over)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AxisId, Preset, ProvinceId};

    fn setup() -> (Data, World) {
        let data = crate::data::load(include_str!("../../../data/rules.ron")).unwrap();
        let preset = Preset::load_with_map(
            include_str!("../../../data/presets/default.ron"),
            include_str!("../../../data/maps/default.ron"),
            &data,
        )
        .unwrap();
        let w = World::from_preset(&data, &preset);
        (data, w)
    }

    fn nordmark() -> NeighbourId {
        NeighbourId("nordmark".into())
    }

    #[test]
    fn strength_formula() {
        let (data, mut w) = setup();
        let ax = |s: &str| AxisId(s.into());
        // Army 50 * (1 + nobles 40 * 0.005) * full treasury (150 >= 100); Nordmark 60.
        assert_eq!(
            strengths(&w, &data, &nordmark()),
            (Fx::from_int(60), Fx::from_int(60))
        );
        // A fort on the Nordmark border counts, the capital's does not.
        let holm = w.provinces.get_mut(&ProvinceId("holm".into())).unwrap();
        holm.buildings.insert("fort".into());
        assert_eq!(strengths(&w, &data, &nordmark()).0, Fx::from_int(65));
        let purpur = NeighbourId("purpur".into());
        assert_eq!(strengths(&w, &data, &purpur).0, Fx::from_int(60));
        // An empty treasury: factor 0.6; half full: 0.8.
        w.axes.insert(ax("treasury"), Fx(-50_000));
        assert_eq!(strengths(&w, &data, &nordmark()).0, Fx::from_int(39));
        w.axes.insert(ax("treasury"), Fx::from_int(50));
        assert_eq!(strengths(&w, &data, &nordmark()).0, Fx::from_int(52));
    }

    #[test]
    fn clash_moves_the_score() {
        let (data, mut w) = setup();
        let mut rng = Rng::from_seed(1);
        clash(&mut w, &data, &mut rng);
        assert_eq!(w.war, None, "no war, no clash");
        w.neighbours.get_mut(&nordmark()).unwrap().strength = Fx::from_int(40);
        w.war = Some(War {
            enemy: nordmark(),
            stage: WarStage::Declared,
            our_strength: Fx(0),
            their_strength: Fx(0),
            war_score: Fx(0),
            started: Tick(0),
            target: None,
            battles: vec![],
        });
        // 60 vs 40, rolls in 0.5..=1.5: the score moves by (60a - 40b) * 0.5, within -30..=40.
        let mut scores = vec![];
        for _ in 0..50 {
            let before = w.war.as_ref().unwrap().war_score;
            clash(&mut w, &data, &mut rng);
            let war = w.war.as_ref().unwrap();
            let d = war.war_score - before;
            assert!(d >= Fx::from_int(-30) && d <= Fx::from_int(40), "{d}");
            assert_eq!(war.our_strength, Fx::from_int(60));
            scores.push(war.war_score);
        }
        // The stronger side drives the score up to the cap, never past it.
        assert_eq!(scores.iter().max(), Some(&data.war.max_score));
        // Every clash is on record with what it moved; the score is their sum.
        let war = w.war.as_ref().unwrap();
        assert_eq!(war.battles.len(), 50);
        let sum = war.battles.iter().fold(Fx(0), |s, (_, d)| s + *d);
        assert_eq!(sum, war.war_score);
    }

    #[test]
    fn war_and_a_big_army_cost_income() {
        let (data, mut w) = setup();
        let ax = |s: &str| AxisId(s.into());
        let flows = |w: &World| {
            let flows = crate::graph::treasury_flows(&data, w);
            flows.fold(data.economy.yearly_income(w), |s, f| s + f)
        };
        let base = flows(&w);
        let upkeep = |army: i64| crate::data::curve(&data.war.army_upkeep, Fx::from_int(army));
        assert_eq!(yearly_income(&w, &data), base - upkeep(50));
        // At war the crown provinces (7 + 4 + 3 + 5 + 4 + 4) bring income_penalty less.
        w.war = Some(War {
            enemy: nordmark(),
            stage: WarStage::Fighting,
            our_strength: Fx(0),
            their_strength: Fx(0),
            war_score: Fx(0),
            started: Tick(0),
            target: None,
            battles: vec![],
        });
        let penalty = Fx::from_int(27) * data.war.income_penalty;
        assert!(penalty > Fx(0));
        assert_eq!(yearly_income(&w, &data), base - upkeep(50) - penalty);
        // Upkeep grows faster than the army: twice the army costs more than twice as much.
        for (a, b) in [(50, 100), (100, 200)] {
            assert!(upkeep(b) > upkeep(a) * Fx::from_int(2), "{a} -> {b}");
        }
        w.axes.insert(ax("army"), Fx::from_int(200));
        let base = flows(&w);
        assert_eq!(yearly_income(&w, &data), base - upkeep(200) - penalty);
    }
}
