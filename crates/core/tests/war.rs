//! Stage 4: war, its event chain and its outcome on the map.

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{Game, Step};
use bd_core::rules::Target;
use bd_core::state::{Holder, NeighbourId, Preset};
use std::fs;
use std::path::PathBuf;

const RULES: &str = include_str!("../../../data/rules.ron");
const PRESET: &str = include_str!("../../../data/presets/default.ron");
const MAP: &str = include_str!("../../../data/maps/default.ron");

fn nordmark() -> NeighbourId {
    NeighbourId("nordmark".into())
}

/// Every events/*.ron, actions.ron and names.ron, as the game loads them.
fn content() -> Data {
    let mut data = bd_core::data::load(RULES).unwrap();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut files: Vec<_> = (fs::read_dir(dir.join("events")).unwrap())
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    for f in files {
        data.add_events(&fs::read_to_string(f).unwrap()).unwrap();
    }
    data.add_actions(include_str!("../../../data/actions.ron"))
        .unwrap();
    data.add_names(include_str!("../../../data/names.ron"))
        .unwrap();
    data
}

/// Only the war: its chain and actions, no other event, no death, quiet neighbours.
/// Every province of the kingdom belongs to the crown.
fn war_only(seed: u64) -> Game {
    let mut data = bd_core::data::load(RULES).unwrap();
    data.add_events(include_str!("../../../data/events/war.ron"))
        .unwrap();
    data.add_actions(include_str!("../../../data/actions.ron"))
        .unwrap();
    (data.quiet_weight, data.death.base, data.death.health_k) = (0, vec![], Fx(0));
    let ai = &mut data.neighbour_ai;
    for s in [&mut ai.expand, &mut ai.defend, &mut ai.trade, &mut ai.wait] {
        s.events.clear();
    }
    let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
    let mut g = Game::new(data, &preset, seed);
    for p in g.world.provinces.values_mut() {
        if matches!(p.holder, Holder::Vassal(_)) {
            p.holder = Holder::Crown;
        }
    }
    g
}

fn holders(g: &Game, holder: &Holder) -> Vec<String> {
    let of = g.world.provinces.values().filter(|p| p.holder == *holder);
    of.map(|p| p.id.0.clone()).collect()
}

/// Declares war on Nordmark, sets its strength to `ratio` (ours / theirs) at the declaration,
/// plays the chain with the middle choice. Returns the last event and the provinces of the
/// crown and of Nordmark before and after.
fn fight(seed: u64, ratio: (i64, i64)) -> (String, [Vec<String>; 4]) {
    let mut g = war_only(seed);
    let (crown, foreign) = (Holder::Crown, Holder::Foreign(nordmark()));
    let before = [holders(&g, &crown), holders(&g, &foreign)];
    g.start_action("declare_war", Some(Target::Neighbour(nordmark())))
        .unwrap();
    let mut last = String::new();
    for i in 0..20 {
        if let Step::Event(v) = g.wait().unwrap() {
            if i == 0 {
                // The war started this tick: fix the ratio before any clash.
                let war = g.world.war.as_mut().unwrap();
                war.their_strength =
                    war.our_strength * Fx::from_int(ratio.1) / Fx::from_int(ratio.0);
                let theirs = war.their_strength;
                g.world.neighbours.get_mut(&nordmark()).unwrap().strength = theirs;
            }
            g.choose((v.choices.len() - 1) / 2).unwrap();
            last = v.event_id;
        }
        if g.world.war.is_none() {
            break;
        }
    }
    assert!(g.world.war.is_none(), "seed {seed}: the war never ended");
    let [c, f] = before;
    (last, [c, f, holders(&g, &crown), holders(&g, &foreign)])
}

/// Share of 1000 seeds that end in `outcome` at this strength ratio.
fn wins(ratio: (i64, i64), outcome: &str) -> usize {
    (0..1000).filter(|&s| fight(s, ratio).0 == outcome).count()
}

#[test]
fn three_to_one_wins_and_one_to_three_loses() {
    let won = wins((3, 1), "war_victory");
    let lost = wins((1, 3), "war_defeat");
    eprintln!("3:1 won {won}, 1:3 lost {lost} of 1000");
    assert!(won >= 900, "{won}");
    assert!(lost >= 900, "{lost}");
}

#[test]
fn victory_takes_their_province_defeat_gives_ours() {
    let (last, [_, theirs, crown_after, theirs_after]) = fight(1, (3, 1));
    assert_eq!(last, "war_victory");
    let taken: Vec<_> = theirs
        .iter()
        .filter(|p| !theirs_after.contains(p))
        .collect();
    assert_eq!(taken, ["frostad"]);
    assert!(crown_after.contains(&"frostad".to_string()));

    let (last, [crown, theirs, crown_after, theirs_after]) = fight(1, (1, 3));
    assert_eq!(last, "war_defeat");
    let lost: Vec<_> = crown.iter().filter(|p| !crown_after.contains(p)).collect();
    assert_eq!(lost, ["arden"], "the weakest border province of the crown");
    assert!(theirs_after.contains(&"arden".to_string()));
    assert_eq!(theirs_after.len(), theirs.len() + 1);
}

#[test]
fn a_second_war_is_refused() {
    let mut g = war_only(1);
    g.start_action("declare_war", Some(Target::Neighbour(nordmark())))
        .unwrap();
    g.wait().unwrap();
    let war = g.world.war.clone().unwrap();
    assert_eq!(war.enemy, nordmark());
    // Not offered while at war; a war that starts anyway, e.g. from an action already
    // running, is refused and queues nothing.
    let actions = g.available_actions();
    assert!(actions.iter().all(|(id, _)| id != "declare_war"));
    let queued = g.queue.len();
    g.pending_event = Some(bd_core::game::PendingEvent {
        event_id: "neighbour_war_declared".into(),
        target: Some(Target::Neighbour(NeighbourId("purpur".into()))),
        neighbour: None,
    });
    g.data
        .add_events(include_str!("../../../data/events/neighbours.ron"))
        .unwrap();
    g.choose(0).unwrap();
    assert_eq!(g.world.war, Some(war));
    assert_eq!(g.queue.len(), queued);
}

/// The whole content, death included, all kinds of choices; wars declared by the crown again
/// and again, and by the neighbours. Every war ends within six years of its start, unless the
/// reign ends first.
#[test]
fn every_war_ends_within_six_years() {
    let data = content();
    let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
    let unit = data.time_unit;
    let ids: Vec<_> = preset.neighbours.iter().map(|n| n.id.clone()).collect();
    let (mut wars, mut longest) = (0, 0);
    for seed in 0..1000u64 {
        let mut g = Game::new(data.clone(), &preset, seed);
        let mut n = seed as usize;
        for _ in 0..60 * unit.ticks_per_year {
            if g.world.war.is_none() && g.pending_event.is_none() {
                n += 1;
                let enemy = Target::Neighbour(ids[n % ids.len()].clone());
                let _ = g.start_action("declare_war", Some(enemy));
            }
            match g.wait().unwrap() {
                Step::Event(v) => g.choose((seed as usize + n) % v.choices.len()).unwrap(),
                Step::Idle => {}
                Step::ReignEnded(_) => break,
            }
            if let Some(war) = &g.world.war {
                let age = g.world.tick.0 - war.started.0;
                longest = longest.max(age);
                if age == 0 {
                    wars += 1;
                }
                assert!(age < 6 * unit.ticks_per_year, "seed {seed}: {war:?}");
            }
        }
    }
    eprintln!("{wars} wars, the longest ran {longest} ticks");
    assert!(wars > 1000, "{wars}");
}

/// Prestige and the relations with Nordmark and the others a tick after declaring war on
/// `enemy`, minus the same in a game that only waited.
fn treachery(enemy: &str) -> (Fx, Vec<Fx>) {
    let at = |g: &Game| {
        let prestige = g.world.axes[&bd_core::state::AxisId("prestige".into())];
        let others = g.world.neighbours.values().filter(|n| n.id.0 != enemy);
        (prestige, others.map(|n| n.relation).collect::<Vec<_>>())
    };
    // Room for prestige to fall whatever the preset starts it at.
    let start = || {
        let mut g = war_only(1);
        let prestige = bd_core::state::AxisId("prestige".into());
        g.world.axes.insert(prestige, Fx::from_int(50));
        g
    };
    let mut base = start();
    base.wait().unwrap();
    let mut g = start();
    let target = Target::Neighbour(NeighbourId(enemy.into()));
    g.start_action("declare_war", Some(target)).unwrap();
    g.wait().unwrap();
    let ((p0, r0), (p1, r1)) = (at(&base), at(&g));
    (p1 - p0, r1.iter().zip(&r0).map(|(a, b)| *a - *b).collect())
}

#[test]
fn war_on_a_friend_costs_prestige_and_trust() {
    // Vestrum is friendly (40), Purpur neutral (0).
    let (prestige, others) = treachery("vestrum");
    assert_eq!(prestige, Fx::from_int(-15));
    // Nordmark (hostile) and Purpur both lose 10; Purpur, from 0, then drifts 1 back.
    assert_eq!(others, [Fx::from_int(-10), Fx::from_int(-9)]);
    assert_eq!(treachery("purpur"), (Fx(0), vec![Fx(0); 2]));
}
