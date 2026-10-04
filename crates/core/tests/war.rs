//! Stage 4: war, its event chain and its outcome on the map.

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{Game, Step};
use bd_core::rules::Target;
use bd_core::state::{AxisId, Holder, NeighbourId, Preset, ProvinceId};
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

/// War on the state holding `province`, for that province.
fn declare(g: &mut Game, province: &str) {
    let target = Target::Province(ProvinceId(province.into()));
    g.start_action("declare_war", Some(target)).unwrap();
}

fn ax(s: &str) -> AxisId {
    AxisId(s.into())
}

fn holders(g: &Game, holder: &Holder) -> Vec<String> {
    let of = g.world.provinces.values().filter(|p| p.holder == *holder);
    of.map(|p| p.id.0.clone()).collect()
}

/// Declares war on Nordmark for `province`, sets its strength to `ratio` (ours / theirs) at
/// the declaration, plays the chain with the middle choice. Returns the last event and the
/// provinces of the crown and of Nordmark before and after.
fn fight(seed: u64, ratio: (i64, i64), province: &str) -> (String, [Vec<String>; 4]) {
    let mut g = war_only(seed);
    let (crown, foreign) = (Holder::Crown, Holder::Foreign(nordmark()));
    let before = [holders(&g, &crown), holders(&g, &foreign)];
    declare(&mut g, province);
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
    (0..1000)
        .filter(|&s| fight(s, ratio, "frostad").0 == outcome)
        .count()
}

#[test]
fn three_to_one_wins_and_one_to_three_loses() {
    let won = wins((3, 1), "war_victory");
    let lost = wins((1, 3), "war_defeat");
    eprintln!("3:1 won {won}, 1:3 lost {lost} of 1000");
    assert!(won >= 900, "{won}");
    assert!(lost >= 900, "{lost}");
}

/// Stage 14: victory takes the province the war was declared for, not the default border one.
#[test]
fn victory_takes_the_chosen_province_defeat_gives_ours() {
    for target in ["frostad", "skala", "nordheim"] {
        let (last, [_, theirs, crown_after, theirs_after]) = fight(1, (3, 1), target);
        assert_eq!(last, "war_victory");
        let taken: Vec<_> = theirs
            .iter()
            .filter(|p| !theirs_after.contains(p))
            .collect();
        assert_eq!(taken, [target]);
        assert!(crown_after.contains(&target.to_string()));
    }

    let (last, [crown, theirs, crown_after, theirs_after]) = fight(1, (1, 3), "skala");
    assert_eq!(last, "war_defeat");
    let lost: Vec<_> = crown.iter().filter(|p| !crown_after.contains(p)).collect();
    assert_eq!(lost, ["arden"], "the weakest border province of the crown");
    assert!(theirs_after.contains(&"arden".to_string()));
    assert_eq!(theirs_after.len(), theirs.len() + 1);
}

/// The war is declared on the state holding the chosen province; only foreign provinces on
/// the realm's border are offered. A war a neighbour starts is fought for the default one.
#[test]
fn the_target_is_an_enemy_border_province() {
    let mut g = war_only(1);
    let targets: Vec<_> = (g.available_actions().into_iter())
        .find(|(id, _)| id == "declare_war")
        .unwrap()
        .1;
    // Stage 28: the foreign land on our border, not the empire beyond the buffers.
    let w = &g.world;
    let own = |id: &ProvinceId| !matches!(w.provinces[id].holder, Holder::Foreign(_));
    let border: std::collections::BTreeSet<_> = (w.provinces.values())
        .filter(|p| matches!(p.holder, Holder::Foreign(_)) && p.neighbours.iter().any(own))
        .map(|p| Target::Province(p.id.clone()))
        .collect();
    assert_eq!(
        targets
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        border
    );
    assert_eq!(border.len(), 12);
    for t in &targets {
        let Target::Province(id) = t else {
            panic!("{t:?}")
        };
        assert!(
            matches!(g.world.provinces[id].holder, Holder::Foreign(_)),
            "{id:?}"
        );
    }
    // Land behind another state's is no target.
    let holm = g
        .world
        .provinces
        .get_mut(&ProvinceId("holm".into()))
        .unwrap();
    holm.holder = Holder::Foreign(nordmark());
    let targets: Vec<_> = (g.available_actions().into_iter())
        .find(|(id, _)| id == "declare_war")
        .unwrap()
        .1;
    assert!(!targets.contains(&Target::Province(ProvinceId("nordheim".into()))));
    assert!(targets.contains(&Target::Province(ProvinceId("holm".into()))));

    let mut g = war_only(1);
    declare(&mut g, "kirm");
    g.wait().unwrap();
    let war = g.world.war.clone().unwrap();
    assert_eq!(war.enemy, NeighbourId("purpur".into()));
    assert_eq!(war.target, Some(ProvinceId("kirm".into())));

    let mut g = war_only(1);
    g.data
        .add_events(include_str!("../../../data/events/neighbours.ron"))
        .unwrap();
    g.pending_event = Some(bd_core::game::PendingEvent {
        event_id: "neighbour_war_declared".into(),
        target: Some(Target::Neighbour(nordmark())),
        neighbour: None,
    });
    g.choose(0).unwrap();
    assert_eq!(
        g.world.war.unwrap().target,
        Some(ProvinceId("frostad".into()))
    );
}

#[test]
fn a_second_war_is_refused() {
    let mut g = war_only(1);
    declare(&mut g, "frostad");
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
/// and again, and by the neighbours. Every war ends within six years of its start, plus a
/// year for every death event of the ruler while it goes on (they go before the war's own,
/// see `Game::death_roll`; stage 16: seeds 370 and 647), or the reign ends first.
#[test]
fn every_war_ends_within_six_years_and_the_deaths() {
    let data = content();
    let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
    let unit = data.time_unit;
    let war_actions = [
        "war_recruit",
        "war_battle",
        "war_peace_talks",
        "war_siege_target",
    ];
    let (mut wars, mut longest) = (0, 0);
    let d = &data.death;
    let deaths: Vec<&String> = std::iter::once(&d.event)
        .chain(d.risks.iter().map(|r| &r.event))
        .collect();
    for seed in 0..1000u64 {
        let mut held = 0;
        let mut g = Game::new(data.clone(), &preset, seed);
        let mut n = seed as usize;
        for _ in 0..60 * unit.ticks_per_year {
            if g.pending_event.is_none() {
                n += 1;
                let actions = g.available_actions();
                let target = |id: &str| {
                    let (_, t) = actions.iter().find(|(a, _)| a == id)?;
                    t.get(n % t.len()).cloned()
                };
                // Now and then one of the war's own actions, at its enemy.
                let id = match g.world.war {
                    None => "declare_war",
                    Some(_) => war_actions[n % 5 % 4],
                };
                if let Some(t) = target(id).filter(|_| g.world.war.is_none() || n % 5 < 4) {
                    let _ = g.start_action(id, Some(t));
                }
            }
            match g.wait().unwrap() {
                Step::Event(v) => {
                    if g.world.war.is_some() && deaths.contains(&&v.event_id) {
                        held += 1;
                    }
                    g.choose((seed as usize + n) % v.choices.len()).unwrap()
                }
                Step::Idle => {}
                Step::ReignEnded(_) => break,
            }
            if let Some(war) = &g.world.war {
                let age = g.world.tick.0 - war.started.0;
                longest = longest.max(age);
                if age == 0 {
                    (wars, held) = (wars + 1, 0);
                }
                let years = 6 + held;
                assert!(age < years * unit.ticks_per_year, "seed {seed}: {war:?}");
            }
        }
    }
    eprintln!("{wars} wars, the longest ran {longest} ticks");
    assert!(wars > 1000, "{wars}");
}

/// Prestige and the relations with Nordmark and the others a tick after declaring war on
/// `enemy` for `province`, minus the same in a game that only waited.
fn treachery(enemy: &str, province: &str) -> (Fx, Vec<Fx>) {
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
    declare(&mut g, province);
    g.wait().unwrap();
    let ((p0, r0), (p1, r1)) = (at(&base), at(&g));
    (p1 - p0, r1.iter().zip(&r0).map(|(a, b)| *a - *b).collect())
}

#[test]
fn war_on_a_friend_costs_prestige_and_trust() {
    // Vestrum is friendly (40), Purpur neutral (0).
    let (prestige, others) = treachery("vestrum", "vestburg");
    assert_eq!(prestige, Fx::from_int(-15));
    // Every other loses 10; those at 0 after it (Purpur, Таврика from 0, Ольховия from 10)
    // drift 1 back less. By id: kadar, nordmark, olkhovia, purpur, tavrika, zudmark.
    let i = Fx::from_int;
    assert_eq!(others, [i(-10), i(-10), i(-9), i(-9), i(-9), i(-10)]);
    assert_eq!(treachery("purpur", "porfir"), (Fx(0), vec![Fx(0); 6]));
}

/// Stage 14: while at war the crown provinces bring `income_penalty` of their income less.
#[test]
fn war_lowers_the_income() {
    let mut peace = war_only(1);
    let mut war = war_only(1);
    war.world.war = Some(bd_core::war::War {
        enemy: nordmark(),
        stage: bd_core::war::WarStage::Fighting,
        our_strength: Fx(0),
        their_strength: Fx(0),
        war_score: Fx(0),
        started: bd_core::time::Tick(0),
        target: None,
        battles: vec![],
    });
    let crown = (peace.world.provinces.values()).filter(|p| p.holder == Holder::Crown);
    let income = crown.fold(Fx(0), |s, p| s + p.income);
    let penalty = income * peace.data.war.income_penalty;
    assert!(penalty > Fx(0));
    let treasury = |g: &mut Game| {
        g.wait().unwrap();
        g.world.axes[&ax("treasury")]
    };
    assert_eq!(treasury(&mut peace) - treasury(&mut war), penalty);
}

/// Stage 14: a year below 0 in the treasury costs `desertion` of the army; with money in it
/// nobody leaves. The upkeep follows `army_upkeep`.
#[test]
fn an_unpaid_army_melts() {
    let army = |treasury: i64, size: i64| {
        let mut g = war_only(1);
        g.world.axes.insert(ax("treasury"), Fx::from_int(treasury));
        g.world.axes.insert(ax("army"), Fx::from_int(size));
        let before = g.world.axes[&ax("treasury")];
        let income = bd_core::war::yearly_income(&g.world, &g.data);
        g.wait().unwrap();
        assert_eq!(g.world.axes[&ax("treasury")], before + income);
        // Stage 15: the years of desertion are counted.
        let deserted = g.world.axes[&ax("army")] < Fx::from_int(size);
        assert_eq!(g.world.deserted, deserted as u32);
        g.world.axes[&ax("army")]
    };
    let kept = Fx::from_int(1) - war_only(1).data.war.desertion;
    assert!(kept < Fx::from_int(1));
    assert_eq!(army(-200, 100), Fx::from_int(100) * kept);
    assert_eq!(army(500, 100), Fx::from_int(100));
    // A big army eats a treasury that would carry a small one.
    assert_eq!(army(10, 300), Fx::from_int(300) * kept);
    assert_eq!(army(10, 50), Fx::from_int(50));
}

/// Stage 14: recruiting, a battle, peace talks and a siege are offered only while a war goes
/// on, at its enemy, and each does what it says.
#[test]
fn war_actions_only_at_war() {
    let ids = [
        "war_recruit",
        "war_battle",
        "war_peace_talks",
        "war_siege_target",
    ];
    let offered = |g: &Game| {
        let all = g.available_actions();
        ids.map(|id| all.iter().find(|(a, _)| a == id).map(|(_, t)| t.clone()))
    };
    let mut g = war_only(1);
    g.world.axes.insert(ax("treasury"), Fx::from_int(1000));
    assert_eq!(offered(&g), [None, None, None, None], "peace");
    for id in ids {
        let t = Some(Target::Neighbour(nordmark()));
        assert_eq!(
            g.start_action(id, t),
            Err(bd_core::game::GameError::Unavailable)
        );
    }
    declare(&mut g, "frostad");
    let Step::Event(v) = g.wait().unwrap() else {
        panic!()
    };
    assert_eq!(v.event_id, "war_declared");
    g.choose(2).unwrap();
    let at = vec![Target::Neighbour(nordmark())];
    assert_eq!(
        offered(&g),
        [
            Some(at.clone()),
            Some(at.clone()),
            Some(at.clone()),
            Some(at)
        ]
    );

    // Recruit: money into army.
    let (army, treasury) = (g.world.axes[&ax("army")], g.world.axes[&ax("treasury")]);
    g.start_action("war_recruit", Some(Target::Neighbour(nordmark())))
        .unwrap();
    assert_eq!(g.world.axes[&ax("treasury")], treasury - Fx::from_int(60));
    g.wait().unwrap();
    assert_eq!(g.world.axes[&ax("army")], army + Fx::from_int(20));
    if g.pending_event.is_some() {
        g.choose(1).unwrap();
    }
    // A battle out of turn moves the score.
    let battles = g.world.war.as_ref().unwrap().battles.len();
    g.start_action("war_battle", Some(Target::Neighbour(nordmark())))
        .unwrap();
    g.wait().unwrap();
    assert!(g.world.war.as_ref().unwrap().battles.len() > battles);
    // Talks: the terms by the score at once.
    if g.pending_event.is_some() {
        g.choose(1).unwrap();
    }
    if g.world
        .war
        .as_ref()
        .is_some_and(|w| w.stage != bd_core::war::WarStage::Peace)
    {
        g.start_action("war_peace_talks", Some(Target::Neighbour(nordmark())))
            .unwrap();
        let Step::Event(v) = g.wait().unwrap() else {
            panic!()
        };
        assert!(["war_victory", "war_defeat", "war_draw"].contains(&v.event_id.as_str()));
    }
    while g.world.war.is_some() {
        if g.pending_event.is_some() {
            g.choose(0).unwrap();
        } else {
            g.wait().unwrap();
        }
    }
    assert_eq!(offered(&g), [None, None, None, None], "peace again");
}

/// Stage 14: a siege takes the target if the field army holds the field (score above 0) by
/// then, whatever the rolls. Stage 15: the fall comes as a card naming the target, whose one
/// choice signs the peace.
#[test]
fn a_siege_takes_the_target() {
    let siege = |score: i64| {
        let mut g = war_only(1);
        declare(&mut g, "skala");
        g.wait().unwrap();
        g.choose(2).unwrap();
        g.start_action("war_siege_target", Some(Target::Neighbour(nordmark())))
            .unwrap();
        // Hold the score where the test wants it; the chain's own events wait.
        g.queue.clear();
        let mut card = None;
        for _ in 0..2 {
            g.world.war.as_mut().unwrap().war_score = Fx::from_int(score);
            if let Step::Event(v) = g.wait().unwrap() {
                card = Some(v.title);
            }
        }
        let skala = g.world.provinces[&ProvinceId("skala".into())]
            .holder
            .clone();
        if card.is_some() {
            g.choose(0).unwrap();
        }
        (skala, card, g.world.war.is_some())
    };
    let fell = Some("Крепость Скала пала".to_string());
    assert_eq!(siege(1), (Holder::Crown, fell, false));
    assert_eq!(siege(0), (Holder::Foreign(nordmark()), None, true));
}

/// Acceptance, bug of playtest 02 (stage 25): a war may end with no battle that year, so
/// its outcome card and its chronicle entry tell how many battles were won and lost.
#[test]
fn a_war_outcome_tells_its_battles() {
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..60 {
        let mut g = war_only(seed);
        declare(&mut g, "frostad");
        for _ in 0..20 {
            if let Step::Event(v) = g.wait().unwrap() {
                if ["war_victory", "war_defeat", "war_draw"].contains(&v.event_id.as_str()) {
                    assert!(
                        v.text.contains(" сражени") && !v.text.contains('{'),
                        "{}",
                        v.text
                    );
                    for i in 0..v.choices.len() {
                        let told = g.told(i).unwrap();
                        assert!(told.contains(" сражени") && !told.contains('{'), "{told}");
                    }
                    seen.insert(v.event_id.clone());
                }
                g.choose((seed as usize) % v.choices.len()).unwrap();
            }
            if g.world.war.is_none() {
                break;
            }
        }
    }
    assert_eq!(seen.len(), 3, "{seen:?}");
}

/// Stage 25: a court agrees to a suit less readily while the crown is at war with anyone
/// (`marriage.war_penalty`); the war is no ban.
#[test]
fn a_war_lowers_the_chance_of_a_suit() {
    let data = content();
    let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
    let mut g = Game::new(data, &preset, 1);
    g.world.heirs[0].age = 20; // someone to wed
    let vestrum = NeighbourId("vestrum".into());
    let chance = |g: &Game| g.data.marriage.chance(&g.world, &g.data, &vestrum);
    let peace = chance(&g);
    g.world.war = Some(bd_core::war::War {
        enemy: nordmark(),
        stage: bd_core::war::WarStage::Fighting,
        our_strength: Fx(0),
        their_strength: Fx(0),
        war_score: Fx(0),
        started: bd_core::time::Tick(0),
        target: None,
        battles: vec![],
    });
    let war = chance(&g);
    assert!(war > Fx(0), "{peace:?} {war:?}");
    assert_eq!(
        peace - war,
        g.data.marriage.war_penalty,
        "{peace:?} {war:?}"
    );
}
