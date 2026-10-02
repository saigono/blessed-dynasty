//! Stage 6: the dynasty simulation and its chronicle.

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{DecisionKind, Game, PendingEvent, ReignEnd, Step};
use bd_core::rng::Rng;
use bd_core::rules::Target;
use bd_core::sim::{self, AutoChooser, Chronicle, FallReason};
use bd_core::state::{
    AxisId, Heir, HeirStatus, Holder, MarkKey, Preset, ProvinceId, Sex, VassalId,
};
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
    // Stage 12: Конрад does not live to reign (heirs.death), Агнесса does.
    // Stage 14: armies cost by the upkeep curve, the dynasty wars with its own actions.
    // Stage 15: gentler heir deaths, Конрад reigns with children born before; less money;
    // Вейр, grown by the three grants, revolts and takes the land bit by bit.
    // Stage 16: sons and daughters, the coronation resets the factions and tells the trait,
    // heirs marry one by one: Конрад outlives his father and reigns 31 years.
    // Stage 17: the rightful heir has his claim at once, heirs wed early (bastards before the
    // wedding): fewer disputes, no usurpation, the dynasty lived to the horizon.
    // Stage 17b: the founder's abdication leaves Конрад contested; a house of little fame
    // (prestige below 50) and a small army: he fights the claimants in «Смута» and loses.
    assert_eq!((c.years, &c.fall), (31, &FallReason::Usurped));
    let hint = |h: &'static str| Some(h);
    assert_eq!(
        texts(&c),
        [
            (
                "Новое правление",
                "Престол наследует Конрад.",
                hint("Наследник основателя учился власти в королевском совете."),
            ),
            (
                "Спор о короне",
                "Корона сложена до срока. Бароны не признают наследника и называют своих претендентов.",
                None,
            ),
            (
                "Новый закон о престоле",
                "Отныне престол наследуют по закону «Выборный закон».",
                None,
            ),
            (
                "Смута",
                "На престоле Конрад, но присягнули не все. Претенденты собирают сторонников, знать выжидает.",
                hint("Венчание в новом соборе укрепило право рода на трон."),
            ),
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
            sex: Sex::Male,
            // Grown heirs are wed, as `heir_marriage` would have it, and crowned so.
            married: *age >= 16,
            married_in: None,
            bastard: false,
        });
    }
    g
}

#[test]
fn succession_takes_the_highest_claim_then_the_eldest() {
    let data = content();
    let home = HeirStatus::Home;
    // Without a law (stage 16: every law has a rule of its own, tests/laws.rs).
    let next = |list: &[(u32, i64, HeirStatus)]| {
        let mut g = heirs(&data, list);
        g.world.flags.retain(|f| !f.starts_with("law_"));
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

/// The simulation with no events, no death and no disputes of minors or weak heirs: only what
/// a test sets up happens.
fn quiet(data: &mut Data) {
    data.events.clear();
    data.sim_events.clear();
    data.quiet_weight = 0;
    (data.death.base, data.death.health_k) = (vec![], Fx(0));
    data.heirs.birth = vec![];
    data.heirs.death = vec![];
    (data.heirs.dispute_minor, data.heirs.dispute_weak) = (Fx(0), (Fx(0), Fx(0)));
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
    // A weak claim, not the rightful one's (stage 17).
    data.heirs
        .laws
        .iter_mut()
        .for_each(|l| l.rightful_claim = Fx(0));
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

/// Stage 15: an adult crowned had children before: a birth roll for every adult year, each
/// child past the heir deaths of its years, all before the collateral line.
#[test]
fn children_born_before_the_coronation() {
    let mut data = content();
    quiet(&mut data);
    yearly(&mut data, "");
    data.heirs.birth = vec![(0, Fx(0)), (20, Fx::from_int(100))];
    data.heirs.death = vec![];
    data.sim.max_years = 1;
    let crowned = |data: &Data| {
        let g = heirs(
            data,
            &[(25, 80, HeirStatus::Home), (23, 75, HeirStatus::Home)],
        );
        let c = sim::run(end_now(&g), data, Rng::from_seed(1));
        let w = c.entries[0].snapshot.clone();
        (w.heirs.iter().map(|h| (h.name.clone(), h.age, h.ability))).collect::<Vec<_>>()
    };
    let line = crowned(&data);
    let ages: Vec<u32> = line.iter().map(|h| h.1).collect();
    assert_eq!(ages, [5, 4, 3, 2, 1, 23], "{line:?}");
    assert_eq!(line[5].0, "h1");
    // Ability grows at home (growth_home 2 a year) as it would have year by year.
    assert_eq!(line[0].2, Fx::from_int(60));
    // Children die by heirs.death in the years before the coronation.
    data.heirs.death = vec![(0, Fx::from_int(1000))];
    assert_eq!(crowned(&data).len(), 1);
}

/// Stage 12: h0 is crowned; his brother h1 becomes the collateral line, and the child born
/// to h0 a year later stands before him and takes the eldest's claim target.
#[test]
fn the_new_rulers_child_goes_before_his_brother() {
    let mut data = content();
    quiet(&mut data);
    yearly(&mut data, "");
    // No children before the coronation at 30 (see children_born_before_the_coronation).
    data.heirs.birth = vec![(30, Fx::from_int(100))];
    data.sim.max_years = 4;
    let g = heirs(
        &data,
        &[(30, 80, HeirStatus::Home), (28, 75, HeirStatus::Home)],
    );
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    let line = |i: usize| {
        let w = &c.entries[i].snapshot;
        (w.heirs.iter())
            .map(|h| (h.name.clone(), h.claim))
            .collect::<Vec<_>>()
    };
    assert_eq!(line(0), [("h1".into(), Fx::from_int(75))]);
    let later = line(2);
    let (child, brother) = (&later[0], later.last().unwrap());
    assert_eq!(brother.0, "h1");
    assert!(
        child.0 != "h1" && child.1 > Fx::from_int(50),
        "{:?}",
        line(2)
    );
    // No longer the rightful heir: down from rightful_claim (90).
    assert!(brother.1 < Fx::from_int(90), "{:?}", line(2));
}

/// (years after the end of the reign, title) of every entry.
fn told(g: &Game, c: &Chronicle) -> Vec<(u32, String)> {
    let at = |e: &bd_core::sim::ChronicleEntry| e.tick.0 - g.world.tick.0;
    (c.entries.iter())
        .map(|e| (at(e), e.title.clone()))
        .collect()
}

/// Stage 12: the first in line dies of age risk (heirs.death) at 14 or older: an entry tells
/// it; a younger one dying is not told while the dynasty goes on.
#[test]
fn the_death_of_a_grown_first_heir_is_told() {
    let mut data = content();
    quiet(&mut data);
    data.heirs.death = vec![(0, Fx(0)), (14, Fx::from_int(1000))];
    data.sim.max_years = 5;
    let g = heirs(
        &data,
        &[(30, 80, HeirStatus::Home), (12, 75, HeirStatus::Home)],
    );
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    let crowned = (0, "Новое правление".to_string());
    assert_eq!(
        told(&g, &c),
        [crowned.clone(), (2, "Смерть наследника".into())]
    );
    let e = &c.entries[1];
    assert_eq!(e.text, "Не стало первого в очереди на престол: h1.");
    assert_eq!(e.importance, data.sim.notable);
    assert!(e.snapshot.heirs.is_empty());
    // Dying at 13 (below sim.heir_death_age), the last heir of a living dynasty: not told.
    data.heirs.death = vec![(0, Fx(0)), (13, Fx::from_int(1000))];
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    assert_eq!(told(&g, &c), [crowned]);
    assert_eq!(c.entries[0].snapshot.heirs.len(), 1);
}

/// Stage 12: an infant, the last heir, dies; no other is born and the ruler dies: the
/// dynasty ends NoHeir, and the infant's death is told where it happened.
#[test]
fn the_death_of_the_last_infant_heir_is_told_at_the_fall() {
    let mut data = content();
    quiet(&mut data);
    data.add_events(&read("events/death.ron")).unwrap();
    data.heirs.death = vec![(0, Fx(0)), (1, Fx::from_int(1000))];
    data.death.base = vec![(0, Fx(0)), (33, Fx::from_int(1000))];
    let g = heirs(
        &data,
        &[(30, 80, HeirStatus::Home), (0, 75, HeirStatus::Home)],
    );
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    assert_eq!(c.fall, FallReason::NoHeir);
    let told = told(&g, &c);
    assert_eq!(
        told[..2],
        [
            (0, "Новое правление".into()),
            (1, "Смерть наследника".into())
        ]
    );
    // With an heir born later (a birth every year), the same death is not told.
    data.heirs.birth = vec![(0, Fx::from_int(100))];
    data.heirs.death = vec![(0, Fx(0)), (1, Fx::from_int(1000)), (2, Fx(0))];
    data.sim.max_years = 10;
    let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
    assert!(
        c.entries.iter().all(|e| e.title != "Смерть наследника"),
        "{:?}",
        texts(&c)
    );
}

/// The simulation with only these sim events of data/events/sim.
fn only(ids: &[&str]) -> Data {
    let mut data = content();
    let keep: Vec<_> = (data.sim_events.iter())
        .filter(|e| ids.contains(&e.id.as_str()))
        .cloned()
        .collect();
    quiet(&mut data);
    data.sim_events = keep;
    data
}

/// h0 crowned at `age` with a brother, the church at `church`.
fn vows(data: &Data, age: u32, church: i64) -> Chronicle {
    let mut g = heirs(
        data,
        &[(age, 80, HeirStatus::Home), (8, 70, HeirStatus::Home)],
    );
    (g.world.axes).insert(ax("loyalty_church"), Fx::from_int(church));
    sim::run(end_now(&g), data, Rng::from_seed(1))
}

/// Stage 12: a child king in the church's care takes the vows the year he comes of age, not
/// before; the next heir is crowned and the church's guardianship goes with the reign.
#[test]
fn a_king_under_church_regency_takes_the_vows_when_he_comes_of_age() {
    // The regency council gives the child to the church at once (its only choice here); the
    // vows are certain once their `when` holds. Set before the reign, church_regency would go
    // with the coronation (sim.reign_flags).
    let mut data = only(&["regency_council", "monastery_vows"]);
    for e in &mut data.sim_events {
        e.weight = 1_000_000;
        e.choices
            .retain(|c| c.cause_tag != "regency_single" && c.cause_tag != "regency_nobles");
    }
    data.sim.max_years = 10;
    let c = vows(&data, 12, 80);
    let names: Vec<_> = c.rulers.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names[1..], ["h0", "h1"]);
    assert_eq!(c.rulers[1].cause.as_deref(), Some("monastery"));
    let reign = c.rulers[1].end.0 - c.rulers[1].start.0;
    assert_eq!(reign, 4, "at 16, after four years of regency");
    let flag = |e: &bd_core::sim::ChronicleEntry| e.snapshot.flags.contains("church_regency");
    assert!(flag(&c.entries[1]), "{:?}", texts(&c)); // the council
    let crowned = c.entries.iter().rfind(|e| e.title == "Новое правление");
    assert!(!flag(crowned.unwrap()));
    // A church without that loyalty (40, +6 from the council) keeps its ward on the throne.
    assert_eq!(vows(&data, 12, 40).rulers.len(), 2);
}

/// Stage 12: a grown king takes the vows only with a very loyal church.
#[test]
fn a_grown_king_takes_the_vows_with_a_loyal_church() {
    let mut data = only(&["monastery_late"]);
    data.sim_events[0].weight = 1_000_000;
    data.sim.max_years = 3;
    // The church as given, not half-way back to its default after the coronation.
    data.coronation = Default::default();
    let c = vows(&data, 30, 80);
    assert_eq!(c.rulers[1].cause.as_deref(), Some("monastery"));
    assert_eq!(vows(&data, 30, 60).rulers.len(), 2);
}

/// Stage 12: each succession law has its own dispute threshold; no law, no dispute.
#[test]
fn the_dispute_threshold_follows_the_law() {
    let mut data = content();
    quiet(&mut data);
    data.heirs
        .laws
        .iter_mut()
        .for_each(|l| l.rightful_claim = Fx(0));
    data.sim.max_years = 1;
    let contested = |law: Option<&str>, claim: i64| {
        let mut g = heirs(&data, &[(30, claim, HeirStatus::Home)]);
        g.world.flags.retain(|f| !f.starts_with("law_"));
        g.world.flags.extend(law.map(String::from));
        let c = sim::run(end_now(&g), &data, Rng::from_seed(1));
        c.entries[0].snapshot.flags.contains("succession_contested")
    };
    // Claim 60: below primogeniture's 70 and elective's 65; 68 only below primogeniture's.
    assert!(contested(Some("law_primogeniture"), 60));
    assert!(contested(Some("law_elective"), 60));
    assert!(contested(Some("law_primogeniture"), 68));
    assert!(!contested(Some("law_elective"), 68));
    assert!(!contested(None, 60));
}

/// Stage 12: under a law with `dispute_per_heir` every heir left is a chance of dispute.
#[test]
fn rivals_quarrel_by_their_number() {
    let mut data = content();
    quiet(&mut data);
    data.sim.max_years = 1;
    let law = data
        .heirs
        .laws
        .iter()
        .position(|l| l.flag == "law_seniority")
        .unwrap();
    data.heirs.laws[law].dispute_per_heir = Fx::from_int(25);
    let contested = |rivals: usize, seed: u64| {
        let mut list = vec![(30, 80, HeirStatus::Home)];
        list.extend(std::iter::repeat_n((10, 50, HeirStatus::Home), rivals));
        let mut g = heirs(&data, &list);
        g.world.flags.retain(|f| !f.starts_with("law_"));
        g.world.flags.insert("law_seniority".into());
        let c = sim::run(end_now(&g), &data, Rng::from_seed(seed));
        c.entries[0].snapshot.flags.contains("succession_contested")
    };
    assert!(!contested(0, 1));
    assert!(contested(4, 1)); // 4 * 25%
    let quarrels = |rivals| (0..40).filter(|&seed| contested(rivals, seed)).count();
    let (one, three) = (quarrels(1), quarrels(3));
    assert!(0 < one && one < three && three < 40, "{one} {three}");
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
    let c = long
        .into_iter()
        .find(|c| c.entries.len() > 10)
        .expect("a long dynasty");
    let counts: Vec<usize> = c.entries.iter().map(|e| count(&e.snapshot)).collect();
    assert!(counts[0] > 0 && counts[0] <= count(&g.world));
    assert!(counts.windows(2).all(|w| w[1] <= w[0]), "{counts:?}");
}

/// Stage 17b: a war takes the year's event (its events are deferred and go first), and
/// `heir_marriage` with it; the automaton weds the heir by `marry_heir` all the same. Only an
/// heir unwed and of `marriage.age` is offered.
#[test]
fn an_heir_weds_in_war() {
    let mut data = content();
    quiet(&mut data);
    data.add_events(&read("events/war.ron")).unwrap();
    data.add_events(&read("events/heirs.ron")).unwrap();
    data.add_actions(&read("actions.ron")).unwrap();
    data.actions.retain(|a| a.id == "marry_heir");
    let mut g = heirs(
        &data,
        &[
            (15, 80, HeirStatus::Home),
            (10, 30, HeirStatus::Home),
            (20, 30, HeirStatus::Home),
        ],
    );
    g.world
        .axes
        .insert(AxisId("treasury".into()), Fx::from_int(100));
    let nordmark = bd_core::state::NeighbourId("nordmark".into());
    g.world.war = Some(bd_core::war::War {
        enemy: nordmark.clone(),
        stage: bd_core::war::WarStage::Fighting,
        our_strength: Fx(0),
        their_strength: Fx(0),
        war_score: Fx(0),
        started: Tick(0),
        target: None,
        battles: vec![],
    });
    let clash = PendingEvent {
        event_id: "war_clash".into(),
        target: Some(Target::Neighbour(nordmark)),
        neighbour: None,
    };
    g.queue.push((Tick(g.world.tick.0 + 1), clash));
    let groom = g.world.heirs.iter().find(|h| h.age == 15).unwrap().id;
    let offered = g.available_actions();
    assert_eq!(
        offered,
        vec![("marry_heir".into(), vec![Target::Heir(groom)])]
    );
    let auto = AutoChooser::for_ruler(&data, &g.world.ruler);
    let (id, target) = auto.action(&mut g).expect("a wedding");
    g.start_action(&id, target).unwrap();
    let Step::Event(v) = g.wait().unwrap() else {
        panic!("the war goes on");
    };
    assert_eq!(v.event_id, "war_clash");
    let i = g.world.heir_index(groom).unwrap();
    assert!(g.world.heirs[i].married);
    assert!(g.available_actions().is_empty());
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
        target: None,
        battles: vec![],
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

/// Stage 14: the automaton pays for its army. With `army_upkeep` weighed, more soldiers are
/// worth less the bigger the army already is, and past some size nothing.
#[test]
fn auto_chooser_counts_the_army_upkeep() {
    let data = content();
    let mut g = game(&data, 1);
    let soldiers = choices(
        r#"[(text: "", effects: [Axis("treasury", 10)], cause_tag: ""),
            (text: "", effects: [Axis("army", 20)], cause_tag: "")]"#,
    );
    let auto = |upkeep: i64| AutoChooser {
        weights: [("army", 1), ("treasury", 1), ("army_upkeep", upkeep)]
            .map(|(k, v)| (k.to_string(), Fx::from_int(v)))
            .into(),
        noise: Fx(0),
    };
    let at = |g: &mut Game, army: i64| {
        g.world.axes.insert(ax("army"), Fx::from_int(army));
    };
    // Army 50: 20 more cost 4 a year (curve 5 -> 9); weighed 2, +20 - 8 beats +10.
    at(&mut g, 50);
    assert_eq!(auto(0).choose(&mut g, &soldiers), 1);
    assert_eq!(auto(-2).choose(&mut g, &soldiers), 1);
    // Army 200: 20 more cost 20 a year; +20 - 40 loses to +10. Without the upkeep it wins.
    at(&mut g, 200);
    assert_eq!(auto(-2).choose(&mut g, &soldiers), 0);
    assert_eq!(auto(0).choose(&mut g, &soldiers), 1);
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

/// Stage 15: a crown over its limit is glad to grant land: `overreach` adds to a grant.
#[test]
fn auto_chooser_grants_land_over_the_limit() {
    let data = content();
    let mut g = game(&data, 1);
    let auto = AutoChooser {
        weights: [("grant", -20), ("overreach", 30)]
            .map(|(k, v)| (k.to_string(), Fx::from_int(v)))
            .into(),
        noise: Fx(0),
    };
    // Room 8 for the 6 crown provinces at the start: a grant only costs.
    assert_eq!(auto.action(&mut g), None);
    g.data.crown_capacity.per_power = Fx(50);
    g.data.crown_capacity.per_axis.clear(); // room 4
    let (id, target) = auto.action(&mut g).unwrap();
    assert_eq!((id.as_str(), target.is_some()), ("grant_province", true));
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

/// Stage 13: the year's summary for script A, seed 42: what `World::changes` reports
/// between the starts of consecutive years, every year with something.
#[test]
fn year_changes_of_seed_42_script_a() {
    use bd_core::state::Change;
    let data = content();
    let mut todo = vec!["berg", "lugovo", "gart"];
    let mut last: Option<bd_core::state::World> = None;
    let mut log: Vec<(u32, Vec<Change>)> = vec![];
    reign(game(&data, 42), |g| {
        if let Some(prev) = &last {
            let changes = g.world.changes(prev, &g.data);
            if !changes.is_empty() {
                log.push((g.world.year(), changes));
            }
        }
        if g.pending_event.is_none()
            && g.world.active_actions.is_empty()
            && let Some(p) = todo.pop()
        {
            grant(g, p);
        }
        last = Some(g.world.clone());
    });
    let axis = |a: &str, v| Change::Axis(ax(a), Fx::from_int(v));
    let nobles = |v| axis("loyalty_nobles", v);
    let granted = |p: &str| {
        let weir = Holder::Vassal(VassalId("weir".into()));
        [
            Change::Holder(pid(p), Holder::Crown, weir),
            Change::Done("grant_province".into(), Some(p.into())),
        ]
    };
    let [g1, g2] = granted("gart");
    let [l1, l2] = granted("lugovo");
    let [b1, b2] = granted("berg");
    let want = vec![
        (1188, vec![nobles(9), g1, g2]),
        (1189, vec![nobles(9), l1, l2]),
        (1190, vec![nobles(7), b1, b2]),
        (1191, vec![nobles(-6)]),
        (1193, vec![Change::Born("Генрих".into())]),
        (1194, vec![axis("army", 20)]),
        (
            1195,
            vec![
                Change::Born("Освальд".into()),
                Change::HeirGone("Генрих".into()),
            ],
        ),
        (1203, vec![Change::Born("Гизела".into())]),
        (
            1204,
            vec![
                axis("legitimacy", 6),
                axis("prestige", 15),
                Change::HeirGone("Гизела".into()),
            ],
        ),
        (1211, vec![nobles(6)]),
    ];
    assert_eq!(log, want);
}

/// Stage 13: the family tree follows births, deaths and coronations into the chronicle.
#[test]
fn kin_of_seed_42_script_a() {
    let (g, end) = script_a(42);
    let c = sim::run(end, &g.data, g.rng.clone());
    let k = &c.kin;
    // The founder: born 32 years before 1187, reigned from then.
    assert_eq!(
        (k[0].heir, k[0].name.as_str(), k[0].born),
        (None, "Ульрих", 1155)
    );
    assert_eq!((k[0].crowned, k[0].parent), (Some(1187), None));
    assert_eq!(k[0].died, Some(1187 + c.rulers[0].end.0));
    // Конрад, 6 at the start, reigned from 1215 until usurped (no death told); Генрих, born
    // 1193, died at 2 uncrowned; Освальд born 1195, alive at the fall; Гизела 1203, died the
    // next year.
    assert_eq!(
        (k[1].name.as_str(), k[1].born, k[1].crowned, k[1].died),
        ("Конрад", 1181, Some(1215), None)
    );
    assert_eq!(
        (k[2].name.as_str(), k[2].born, k[2].parent, k[2].crowned),
        ("Генрих", 1193, Some(0), None)
    );
    assert_eq!(k[2].died, Some(1195));
    assert_eq!(
        (k[3].name.as_str(), k[3].born, k[3].parent, k[3].died),
        ("Освальд", 1195, Some(0), None)
    );
    assert_eq!(
        (k[4].name.as_str(), k[4].born, k[4].parent, k[4].died),
        ("Гизела", 1203, Some(0), Some(1204))
    );
    // Every ruler in the chronicle is a crowned kin, in order; children point at a ruler.
    let crowned: Vec<(&str, u32)> = (k.iter())
        .filter_map(|x| Some((x.name.as_str(), x.crowned?)))
        .collect();
    let rulers: Vec<(&str, u32)> = (c.rulers.iter())
        .map(|r| (r.name.as_str(), 1187 + r.start.0))
        .collect();
    assert_eq!(crowned, rulers);
    for x in &k[1..] {
        let p = x.parent.expect("every heir has a parent");
        assert!(k[p].crowned.is_some() && k[p].born < x.born, "{x:?}");
    }
    // The dead are those no longer among the heirs nor on the throne (as of the last entry;
    // later births are not in it).
    let last = &c.entries.last().unwrap().snapshot;
    let alive =
        |x: &&bd_core::state::Kin| x.died.is_none() && x.crowned.is_none() && x.born <= last.year();
    for x in k.iter().filter(alive) {
        assert!(
            x.heir.is_some_and(|id| last.heir_index(id).is_some()),
            "{x:?}"
        );
    }
}

/// Stage 13: who is first in line, as the reign screen shows it.
#[test]
fn next_heir_has_the_highest_claim_the_eldest_on_a_tie() {
    let data = content();
    let mut g = game(&data, 1);
    assert_eq!(sim::next_heir(&g.world), Some(0));
    for name in ["Ада", "Бруно"] {
        let mut h = data.new_heir.clone();
        h.name = name.into();
        g.world.add_heir(h);
    }
    let names = |g: &Game| {
        g.world
            .heirs
            .iter()
            .map(|h| h.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&g), ["Конрад", "Ада", "Бруно"]);
    let claims = |g: &mut Game, c: [i64; 3]| {
        for (h, c) in g.world.heirs.iter_mut().zip(c) {
            h.claim = Fx::from_int(c);
        }
        sim::next_heir(&g.world).map(|i| g.world.heirs[i].name.clone())
    };
    assert_eq!(claims(&mut g, [40, 60, 50]).as_deref(), Some("Ада"));
    assert_eq!(claims(&mut g, [50, 50, 50]).as_deref(), Some("Конрад"));
    g.world.heirs.clear();
    assert_eq!(sim::next_heir(&g.world), None);
}

/// Stage 13: law names and texts come from rules.ron with the numbers filled in.
#[test]
fn law_texts_take_their_numbers_from_the_rules() {
    let mut data = content();
    for l in &data.heirs.laws {
        assert!(!l.name.is_empty() && !l.text().is_empty(), "{}", l.flag);
        assert!(!l.text().contains('{'), "{}", l.text());
    }
    let first = &mut data.heirs.laws[0];
    assert!(first.text().contains("ниже 70"), "{}", first.text());
    first.crisis_claim = Fx(65_500);
    assert!(first.text().contains("ниже 65.5"), "{}", first.text());
    let g = game(&data, 1);
    assert_eq!(
        data.heirs.law(&g.world).unwrap().name,
        "Абсолютное первородство"
    );
}

/// Stage 13: the bond of a finished marriage; stage 16: one per union (`World.unions`).
#[test]
fn bonds_name_the_married_neighbour() {
    let mut data = content();
    data.marriage.percent = Fx::from_int(100);
    let mut g = game(&data, 1);
    g.world.flags.remove("married");
    let vestrum = bd_core::state::NeighbourId("vestrum".into());
    assert!(g.bonds().is_empty());
    g.start_action("marry_neighbour", Some(Target::Neighbour(vestrum.clone())))
        .unwrap();
    assert!(g.bonds().is_empty(), "not while it runs");
    g.wait().unwrap();
    let bonds: Vec<_> = (g.bonds().into_iter())
        .map(|(n, a, t)| (n, a.bond.clone(), t))
        .collect();
    // Dated by the wedding, a year after the suit.
    assert_eq!(bonds, [(vestrum, "брачный союз".to_string(), Tick(1))]);
    g.world.unions.clear();
    assert!(g.bonds().is_empty());
}

/// Stage 15: the chronicle counts the years the army deserted in the simulation only, not
/// the founder's.
#[test]
fn the_chronicle_counts_desertions_of_the_dynasty() {
    let data = content();
    let mut g = game(&data, 3);
    g.world.deserted = 1000;
    g.world.axes.insert(ax("treasury"), Fx::from_int(-1000));
    g.world.axes.insert(ax("army"), Fx::from_int(200));
    let c = sim::run(end_now(&g), &data, g.rng.clone());
    assert!(
        0 < c.deserted && c.deserted <= c.years,
        "{} in {}",
        c.deserted,
        c.years
    );
}
