//! War: one at a time, fought through the event chain of `data/events/war.ron`.
//! The effects that drive it live in `rules::Effect`; the numbers in `rules.ron` `war`.

use crate::data::Data;
use crate::fx::Fx;
use crate::rng::Rng;
use crate::state::{Holder, NeighbourId, World};
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
    let war = w.war.as_mut().expect("checked above");
    war.war_score = (war.war_score + delta).clamp(Fx(0) - r.max_score, r.max_score);
    (war.our_strength, war.their_strength) = (ours, theirs);
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
    }
}
