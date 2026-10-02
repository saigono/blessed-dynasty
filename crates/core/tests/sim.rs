//! Stage 6: the dynasty simulation and its chronicle.

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{DecisionKind, Game, PendingEvent, ReignEnd, Step};
use bd_core::rng::Rng;
use bd_core::rules::Target;
use bd_core::sim::{self, AutoChooser, Chronicle, FallReason};
use bd_core::state::{AxisId, Heir, HeirStatus, Holder, MarkKey, Preset, ProvinceId, VassalId};
use bd_core::time::Tick;
use std::fs;
use std::path::PathBuf;

const PRESET: &str = include_str!("../../../data/presets/default.ron");
const MAP: &str = include_str!("../../../data/maps/default.ron");

fn dir(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(dir(rel)).unwrap()
}

fn ron_files(rel: &str) -> Vec<String> {
    let mut files: Vec<_> = (fs::read_dir(dir(rel)).unwrap())
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "ron"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| fs::read_to_string(f).unwrap())
        .collect()
}

/// Everything in data/, the simulation events included.
fn content() -> Data {
    let mut data = bd_core::data::load(&read("rules.ron")).unwrap();
    ron_files("events")
        .iter()
        .for_each(|t| data.add_events(t).unwrap());
    (ron_files("events/sim").iter()).for_each(|t| data.add_sim_events(t).unwrap());
    data.add_actions(&read("actions.ron")).unwrap();
    data.add_names(&read("names.ron")).unwrap();
    data.add_hints(&read("hints.ron")).unwrap();
    data
}

fn game(data: &Data, seed: u64) -> Game {
    let preset = Preset::load_with_map(PRESET, MAP, data).unwrap();
    Game::new(data.clone(), &preset, seed)
}

fn pid(s: &str) -> ProvinceId {
    ProvinceId(s.into())
}

fn ax(s: &str) -> AxisId {
    AxisId(s.into())
}

/// Plays the reign to its end: `act` may start actions before each tick, events take the
/// middle choice.
fn reign(mut g: Game, mut act: impl FnMut(&mut Game)) -> (Game, ReignEnd) {
    loop {
        act(&mut g);
        match g.wait().unwrap() {
            Step::Event(v) => g.choose((v.choices.len() - 1) / 2).unwrap(),
            Step::Idle => {}
            Step::ReignEnded(end) => return (g, end),
        }
    }
}

/// The reign ends now, as it is.
fn end_now(g: &Game) -> ReignEnd {
    ReignEnd {
        cause: "illness".into(),
        tick: g.world.tick,
        world: g.world.clone(),
    }
}

fn grant(g: &mut Game, province: &str) {
    let t = Some(Target::Province(pid(province)));
    g.start_action("grant_province", t).unwrap();
}

/// Script A: grant Гарт, Лугово and Берг one after another (all go to Вейр), then the middle
/// choice of every event until the founder dies.
fn script_a(seed: u64) -> (Game, ReignEnd) {
    let data = content();
    let mut todo = vec!["berg", "lugovo", "gart"];
    reign(game(&data, seed), move |g| {
        if g.pending_event.is_none()
            && g.world.active_actions.is_empty()
            && let Some(p) = todo.pop()
        {
            grant(g, p);
        }
    })
}

/// Title, text and hint of every entry.
fn texts(c: &Chronicle) -> Vec<(&str, &str, Option<&str>)> {
    let told = c.entries.iter();
    told.map(|e| (e.title.as_str(), e.text.as_str(), e.hint.as_deref()))
        .collect()
}

/// Pinned: changes to rules, content or the engine move it on purpose, then it is re-pinned.
#[test]
fn golden_seed_42_script_a() {
    let (g, end) = script_a(42);
    let c = sim::run(end, &g.data, g.rng.clone());
    // Stage 12: Конрад does not live to reign (heirs.death), Агнесса does; her first in
    // line dies of age risk and is told in the chronicle.
    assert_eq!((c.years, &c.fall), (69, &FallReason::Usurped));
    let hint = |h: &'static str| Some(h);
    assert_eq!(
        texts(&c)[..4],
        [
            (
                "Новое правление",
                "Престол наследует Агнесса.",
                hint("Основатель породнил наследника с домом своего барона."),
            ),
            (
                "Набег из степи",
                "В тот год из степи пришла конная орда. Кочевники жгут сёла земли Оствик и уводят людей в полон.",
                hint("Набег, отбитый при основателе, научил соседа осторожности."),
            ),
            (
                "Смерть наследника",
                "Не стало первого в очереди на престол: Матильда.",
                hint("Наследник основателя учился власти в королевском совете."),
            ),
            ("Новое правление", "Престол наследует Освальд.", None),
        ]
    );
    // The same seed and decisions give the same chronicle.
    let (g2, end2) = script_a(42);
    assert_eq!(sim::run(end2, &g2.data, g2.rng.clone()), c);
}

/// 1000 dynasties, each after a neutral reign of its own seed.
#[test]
#[ignore = "takes about two minutes in the test profile; stage 6 acceptance, run with --ignored"]
fn thousand_dynasties_end() {
    dynasties(1000);
}

#[test]
fn thirty_dynasties_end() {
    dynasties(30);
}

fn dynasties(n: u64) {
    let data = content();
    let start = game(&data, 0);
    for seed in 0..n {
        let mut g = start.clone();
        g.rng = Rng::from_seed(seed);
        let (g, end) = reign(g, |_| {});
        let c = sim::run(end, &data, g.rng.clone());
        assert!(c.years <= data.sim.max_years, "seed {seed}: {}", c.years);
        assert!(
            c.rulers.len() >= 2 || c.fall == FallReason::NoHeir,
            "seed {seed}"
        );
    }
}

#[test]
fn revolt_of_a_vassal_who_got_three_provinces_points_at_the_grants() {
    let (g, _) = script_a(42);
    let weir = Holder::Vassal(VassalId("weir".into()));
    for p in ["gart", "lugovo", "berg"] {
        assert_eq!(g.world.provinces[&pid(p)].holder, weir, "{p}");
    }
    let grants: Vec<usize> = (0..g.decisions.len())
        .filter(|&i| match &g.decisions[i].kind {
            DecisionKind::ActionStarted { action_id, .. } => action_id == "grant_province",
            _ => false,
        })
        .collect();
    assert_eq!(grants.len(), 3);
    // The first dynasty with a revolt; the seed only varies the simulation.
    let revolt = (0..50)
        .find_map(|seed| {
            let c = sim::run(end_now(&g), &g.data, Rng::from_seed(seed));
            c.entries
                .into_iter()
                .find(|e| e.event.as_deref() == Some("vassal_revolt"))
        })
        .expect("a revolt in 50 dynasties");
    let granted = |c: &&bd_core::state::CauseTag| {
        grants.contains(&c.decision_idx) && c.cause_tag == "province_granted"
    };
    assert!(
        revolt.causes.iter().any(|c| granted(&c)),
        "{:?}",
        revolt.causes
    );
}

#[test]
fn no_heir_no_dynasty() {
    let data = content();
    let mut g = game(&data, 1);
    g.world.heirs.clear();
    assert_eq!(
        sim::succession(&g.world, &data, &mut Rng::from_seed(1)),
        None
    );
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    assert_eq!(c.fall, FallReason::NoHeir);
    assert_eq!((c.years, c.rulers.len()), (0, 1));
    assert_eq!(c.rulers[0].cause.as_deref(), Some("illness"));
    assert!(c.entries.is_empty());
}

/// Heirs of these (age, claim, status); the ruler is one of them.
fn heirs(data: &Data, list: &[(u32, i64, HeirStatus)]) -> Game {
    let mut g = game(data, 1);
    g.world.heirs.clear();
    for (i, (age, claim, status)) in list.iter().enumerate() {
        g.world.add_heir(Heir {
            id: 0,
            name: format!("h{i}"),
            age: *age,
            ability: Fx::from_int(50),
            claim: Fx::from_int(*claim),
            status: status.clone(),
        });
    }
    g
}

#[test]
fn succession_takes_the_highest_claim_then_the_eldest() {
    let data = content();
    let home = HeirStatus::Home;
    let next = |list: &[(u32, i64, HeirStatus)]| {
        let g = heirs(&data, list);
        sim::succession(&g.world, &data, &mut Rng::from_seed(1)).unwrap()
    };
    let r = next(&[(20, 40, home.clone()), (18, 60, home.clone())]);
    assert_eq!((r.name.as_str(), r.age), ("h1", 18));
    assert_eq!(r.health, data.sim.ruler_health);
    let r = next(&[(20, 60, home.clone()), (18, 60, home.clone())]);
    assert_eq!(r.name, "h0");
    // A newborn's placeholder name gives way to one from names.ron.
    let mut g = heirs(&data, &[(1, 50, home.clone())]);
    g.world.heirs[0].name = data.new_heir.name.clone();
    let r = sim::succession(&g.world, &data, &mut Rng::from_seed(1)).unwrap();
    assert!(data.names.rulers.contains(&r.name), "{}", r.name);
}

#[test]
fn traits_follow_ability_and_status() {
    let mut data = content();
    for t in &mut data.sim.traits {
        (t.percent, t.ability_k, t.studying, t.hostage) = (Fx(0), Fx(0), Fx(0), Fx(0));
    }
    data.sim.traits[0].ability_k = Fx::from_int(2); // ability 50: certain
    data.sim.traits[1].studying = Fx::from_int(100);
    data.sim.traits[2].hostage = Fx::from_int(100);
    let traits = |status: HeirStatus| {
        let g = heirs(&data, &[(20, 50, status)]);
        let r = sim::succession(&g.world, &data, &mut Rng::from_seed(1)).unwrap();
        r.traits.into_iter().collect::<Vec<_>>()
    };
    let id = |i: usize| data.sim.traits[i].id.clone();
    assert_eq!(traits(HeirStatus::Home), [id(0)]);
    let mut studying = vec![id(0), id(1)];
    studying.sort();
    assert_eq!(traits(HeirStatus::Studying("Монастырь".into())), studying);
    let hostage = traits(HeirStatus::Hostage(bd_core::state::NeighbourId(
        "nordmark".into(),
    )));
    assert!(hostage.contains(&id(2)) && !hostage.contains(&id(1)));
}

/// The simulation with no events and no death: only what a test sets up happens.
fn quiet(data: &mut Data) {
    data.events.clear();
    data.sim_events.clear();
    data.quiet_weight = 0;
    (data.death.base, data.death.health_k) = (vec![], Fx(0));
    data.heirs.birth = vec![];
    data.heirs.death = vec![];
    data.actions.clear();
    let ai = &mut data.neighbour_ai;
    for s in [&mut ai.expand, &mut ai.defend, &mut ai.trade, &mut ai.wait] {
        s.events.clear();
    }
}

/// An event every tick with these effects, importance 5: an entry, and a snapshot, per tick.
fn yearly(data: &mut Data, effects: &str) {
    data.add_events(&format!(
        "[(id: \"year\", title: \"\", text: \"Год прошёл.\", when: All([]), weight: 1, \
         once: false, cooldown_years: 0, importance: 5, target: None, \
         choices: [(text: \"\", effects: [{effects}], cause_tag: \"year\")])]"
    ))
    .unwrap();
}

#[test]
fn a_child_reigns_under_regency_and_a_weak_claim_is_contested() {
    let mut data = content();
    quiet(&mut data);
    yearly(&mut data, "");
    data.sim.max_years = 12;
    let g = heirs(&data, &[(10, 30, HeirStatus::Home)]);
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    assert_eq!((c.fall.clone(), c.years), (FallReason::Alive, 12));
    assert_eq!(c.rulers[1].name, "h0");
    let crowned = &c.entries[0].snapshot;
    assert!(crowned.flags.contains("succession_contested"));
    // Regency until he is 16; the flag goes the year after.
    for e in &c.entries {
        let (age, regency) = (e.snapshot.ruler.age, e.snapshot.flags.contains("regency"));
        assert!(age > 16 || regency, "{age}");
        assert!(age < 17 || !regency, "{age}");
    }
    assert_eq!(c.entries.last().unwrap().snapshot.ruler.age, 22);

    // An adult with a strong claim: neither.
    let g = heirs(&data, &[(30, 80, HeirStatus::Home)]);
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    let w = &c.entries[0].snapshot;
    assert!(!w.flags.contains("regency") && !w.flags.contains("succession_contested"));
}

#[test]
fn falls() {
    let mut data = content();
    quiet(&mut data);
    let fall = |data: &Data, setup: &dyn Fn(&mut Game)| {
        let mut g = heirs(data, &[(30, 80, HeirStatus::Home)]);
        setup(&mut g);
        sim::run(end_now(&g), data, Rng::from_seed(1)).fall
    };
    assert_eq!(fall(&data, &|_| {}), FallReason::Alive);
    let usurped = |g: &mut Game| {
        g.world.flags.insert("usurped".into());
    };
    assert_eq!(fall(&data, &usurped), FallReason::Usurped);
    let capital = |g: &mut Game| {
        let p = g.world.provinces.get_mut(&pid("capital")).unwrap();
        p.holder = Holder::Vassal(VassalId("weir".into()));
    };
    assert_eq!(fall(&data, &capital), FallReason::CapitalLost);
    let nothing_left = |g: &mut Game| {
        for p in g.world.provinces.values_mut() {
            if p.holder == Holder::Crown {
                p.holder = Holder::Vassal(VassalId("weir".into()));
            }
        }
    };
    assert_eq!(fall(&data, &nothing_left), FallReason::NoCrownLand);
    // The second ruler dies with no one left.
    let mut deadly = data.clone();
    deadly.death.base = vec![(0, Fx::from_int(1000))];
    deadly.add_events(&read("events/death.ron")).unwrap();
    let mut g = heirs(&deadly, &[(30, 80, HeirStatus::Home)]);
    g.world.ruler.age = 30;
    let c = sim::run(end_now(&g), &deadly, Rng::from_seed(1));
    assert_eq!(c.fall, FallReason::NoHeir);
    assert_eq!(c.rulers.len(), 2);
    assert!(c.years > 0 && c.years < 30, "{}", c.years);
}

#[test]
fn decisions_mark_what_they_touch_and_marks_fade() {
    let data = content();
    // Seed 1 loses Конрад in the first year (heirs.death).
    let mut g = game(&data, 2);
    g.pending_event = Some(PendingEvent {
        event_id: "cap_court_intrigue".into(),
        target: None,
        neighbour: None,
    });
    g.choose(0).unwrap(); // bureaucracy +4, nobles -5
    g.start_action("grant_province", Some(Target::Province(pid("gart"))))
        .unwrap();
    let tags = |g: &Game, k: MarkKey| {
        let marks = g.world.marks.get(&k).cloned().unwrap_or_default();
        marks
            .into_iter()
            .map(|t| (t.decision_idx, t.cause_tag, t.weight))
            .collect::<Vec<_>>()
    };
    let one = Fx::from_int(1);
    assert_eq!(
        tags(&g, MarkKey::Axis(ax("bureaucracy"))),
        [(0, "court_clerks".into(), one)]
    );
    assert_eq!(tags(&g, MarkKey::Axis(ax("loyalty_nobles"))).len(), 1);
    assert!(tags(&g, MarkKey::Axis(ax("treasury"))).is_empty());
    // The action marks when it completes, under the decision that started it.
    assert!(tags(&g, MarkKey::Province(pid("gart"))).is_empty());
    while g.world.tick < Tick(1) {
        if let Step::Event(v) = g.wait().unwrap() {
            g.choose((v.choices.len() - 1) / 2).unwrap();
        }
    }
    let decay = data.sim.decay;
    assert_eq!(
        tags(&g, MarkKey::Province(pid("gart"))),
        [(1, "province_granted".into(), one)]
    );
    // A year on, the older marks have faded once.
    assert_eq!(tags(&g, MarkKey::Axis(ax("bureaucracy")))[0].2, decay);
    // Marks follow the world into the simulation, which adds none of its own: their number
    // only falls as weights fade to nothing.
    let count = |w: &bd_core::state::World| w.marks.values().flatten().count();
    // Any long enough dynasty: heirs die and child rulers fall, so not every seed has one.
    let long = (0..20).map(|seed| sim::run(end_now(&g), &data, Rng::from_seed(seed)));
    let c = long.into_iter().find(|c| c.entries.len() > 10).expect("a long dynasty");
    let counts: Vec<usize> = c.entries.iter().map(|e| count(&e.snapshot)).collect();
    assert!(counts[0] > 0 && counts[0] <= count(&g.world));
    assert!(counts.windows(2).all(|w| w[1] <= w[0]), "{counts:?}");
}

#[test]
fn an_unfinished_war_starts_over_under_the_heir() {
    let mut data = content();
    quiet(&mut data);
    data.add_events(&read("events/war.ron")).unwrap();
    data.sim.max_years = 5;
    let mut g = heirs(&data, &[(30, 80, HeirStatus::Home)]);
    let nordmark = bd_core::state::NeighbourId("nordmark".into());
    g.world.war = Some(bd_core::war::War {
        enemy: nordmark,
        stage: bd_core::war::WarStage::Fighting,
        our_strength: Fx(0),
        their_strength: Fx(0),
        war_score: Fx(0),
        started: Tick(0),
    });
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    let events: Vec<_> = c
        .entries
        .iter()
        .filter_map(|e| e.event.as_deref())
        .collect();
    assert_eq!(events.first(), Some(&"war_declared"));
    assert!(events.contains(&"war_clash"), "{events:?}");
}

#[test]
fn lost_and_regained_provinces_are_told_with_the_hint_of_their_cause() {
    let mut data = content();
    let mut g = game(&data, 1);
    g.start_action("build_road", Some(Target::Province(pid("berg"))))
        .unwrap();
    while g.world.tick < Tick(2) {
        if let Step::Event(v) = g.wait().unwrap() {
            g.choose((v.choices.len() - 1) / 2).unwrap();
        }
    }
    quiet(&mut data);
    data.sim.threshold = 9; // the two events stay out of the chronicle
    data.add_events(
        r#"[
        (id: "lose", title: "", text: "", when: All([]), weight: 1, once: true, cooldown_years: 0,
         importance: 1, target: None, choices: [(text: "", cause_tag: "lose", effects: [
            TransferProvince(ById("berg"), Foreign(ById("nordmark"))), SpawnEvent("win", 2)])]),
        (id: "win", title: "", text: "", when: All([]), weight: 0, once: true, cooldown_years: 0,
         importance: 1, target: None, choices: [(text: "", cause_tag: "win", effects: [
            TransferProvince(ById("berg"), Crown)])]),
    ]"#,
    )
    .unwrap();
    data.sim.max_years = 6;
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    let told: Vec<_> = (c.entries.iter())
        .map(|e| {
            (
                e.tick.0,
                e.title.as_str(),
                e.text.as_str(),
                e.hint.as_deref(),
            )
        })
        .collect();
    let hint = Some("Дороги, проложенные основателем, связали край со столицей.");
    assert_eq!(
        told[1..],
        [
            (
                3,
                "Потеря земли",
                "Земля Берг потеряна, ею владеет Нордмарк.",
                hint
            ),
            (
                5,
                "Земля возвращена",
                "Земля Берг отошла к короне. Прежний владелец: Нордмарк.",
                hint
            ),
        ]
    );
    let lost = &c.entries[1];
    assert_eq!(
        (lost.event.clone(), lost.importance),
        (None, data.sim.notable)
    );
    assert_eq!(lost.causes[0].cause_tag, "road_built");
    // A faded cause gives no hint.
    data.sim.hint_weight = Fx::from_int(2);
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    assert_eq!(c.entries[1].hint, None);
}

fn choices(text: &str) -> Vec<bd_core::rules::Choice> {
    ron::from_str(text).unwrap()
}

#[test]
fn auto_chooser_weighs_effects_by_the_ruler_traits() {
    let data = content();
    let mut g = game(&data, 1);
    let war_or_peace = choices(
        r#"[(text: "", effects: [], cause_tag: ""),
            (text: "", effects: [StartWar(ById("nordmark"))], cause_tag: "")]"#,
    );
    let with = |traits: &[&str]| {
        let mut r = g.world.ruler.clone();
        r.traits = traits.iter().map(|t| t.to_string()).collect();
        AutoChooser::for_ruler(&data, &r)
    };
    let (warlike, cautious) = (with(&["warlike"]), with(&["cautious"]));
    assert_eq!(warlike.weights["war"], Fx::from_int(15)); // base -10 + warlike 25
    for _ in 0..20 {
        assert_eq!(warlike.choose(&mut g, &war_or_peace), 1);
        assert_eq!(cautious.choose(&mut g, &war_or_peace), 0);
    }
    // A chance counts by its odds; the first option wins a tie.
    let flat = |pairs: &[(&str, i64)]| AutoChooser {
        weights: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Fx::from_int(*v)))
            .collect(),
        noise: Fx(0),
    };
    let gamble = |percent: u32| {
        choices(&format!(
            r#"[(text: "", cause_tag: "", effects: [Chance((percent: {percent},
                then: [Axis("treasury", 100)], otherwise: [Axis("treasury", -100)]))]),
                (text: "", cause_tag: "", effects: [Axis("treasury", 20)])]"#
        ))
    };
    let money = flat(&[("treasury", 1)]);
    assert_eq!(money.choose(&mut g, &gamble(61)), 0); // +22 vs +20
    assert_eq!(money.choose(&mut g, &gamble(60)), 0); // +20 vs +20
    assert_eq!(money.choose(&mut g, &gamble(59)), 1);
    // Noise decides between equals.
    let noisy = AutoChooser {
        noise: Fx::from_int(10),
        ..flat(&[])
    };
    let picks: std::collections::BTreeSet<_> = (0..20)
        .map(|_| noisy.choose(&mut g, &war_or_peace))
        .collect();
    assert_eq!(picks.len(), 2);
}

#[test]
fn auto_chooser_acts_only_when_it_pays() {
    let data = content();
    let mut g = game(&data, 1);
    let flat = |pairs: &[(&str, i64)]| AutoChooser {
        weights: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Fx::from_int(*v)))
            .collect(),
        noise: Fx(0),
    };
    assert_eq!(flat(&[]).action(&mut g), None);
    // Building is worth it only while it outweighs the money: the cheapest, a road, costs 40.
    assert_eq!(
        flat(&[("build", 100), ("treasury", 3)]).action(&mut g),
        None
    );
    let (id, target) = flat(&[("build", 100), ("treasury", 1)])
        .action(&mut g)
        .unwrap();
    assert!(id.starts_with("build_") && target.is_some(), "{id}");
    // No free slot: nothing.
    g.start_action(&id, target).unwrap();
    assert_eq!(flat(&[("build", 100)]).action(&mut g), None);
}

/// Stage 7: the score of a chronicle depends on nothing else.
#[test]
fn the_score_of_a_dynasty_is_deterministic() {
    let data = content();
    let rules = bd_core::score::load(&read("score.ron"), &data).unwrap();
    let g = game(&data, 7);
    let c = sim::run(end_now(&g), &data, g.rng.clone());
    let s = bd_core::score::compute(&c, &g.decisions, &rules);
    assert!(s.total > 0, "{s:?}");
    assert_eq!(bd_core::score::compute(&c, &g.decisions, &rules), s);
    let c2 = sim::run(end_now(&g), &data, g.rng.clone());
    assert_eq!(bd_core::score::compute(&c2, &g.decisions, &rules), s);
}
