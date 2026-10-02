//! Stage 16: succession laws, the sex of heirs, the coronation, marriages.

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{Game, ReignEnd};
use bd_core::rng::Rng;
use bd_core::rules::Target;
use bd_core::sim::{self, AutoChooser, FallReason};
use bd_core::state::{AxisId, Heir, HeirStatus, Holder, NeighbourId, Preset, ProvinceId, Sex};
use std::fs;
use std::path::PathBuf;

const PRESET: &str = include_str!("../../../data/presets/default.ron");
const MAP: &str = include_str!("../../../data/maps/default.ron");

fn read(rel: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    fs::read_to_string(dir.join(rel)).unwrap()
}

/// Rules, actions and names; no events; disputes of minors and weak heirs off (their own
/// test turns them on).
fn data() -> Data {
    let mut data = bd_core::data::load(&read("rules.ron")).unwrap();
    data.add_actions(&read("actions.ron")).unwrap();
    data.add_names(&read("names.ron")).unwrap();
    (data.heirs.dispute_minor, data.heirs.dispute_weak) = (Fx(0), (Fx(0), Fx(0)));
    data
}

/// A person: sex, age, a child of the ruler (else the collateral line), ability.
type Person = (Sex, u32, bool, i64);

const M: Sex = Sex::Male;
const F: Sex = Sex::Female;

/// The default preset under `law` with these heirs, named p0, p1, … in the order given;
/// the collateral line is added first, as it would have been born.
fn game(data: &Data, law: &str, people: &[Person]) -> Game {
    let preset = Preset::load_with_map(PRESET, MAP, data).unwrap();
    let mut g = Game::new(data.clone(), &preset, 1);
    let w = &mut g.world;
    w.heirs.clear();
    w.flags.retain(|f| !f.starts_with("law_"));
    w.flags.insert(law.into());
    let heir = |i: usize, (sex, age, _, ability): &Person| Heir {
        id: 0,
        name: format!("p{i}"),
        age: *age,
        ability: Fx::from_int(*ability),
        claim: Fx::from_int(50),
        status: HeirStatus::Home,
        sex: *sex,
        // Grown heirs are wed, and crowned so (births before the coronation).
        married: *age >= 16,
        married_in: None,
        bastard: false,
    };
    for (i, p) in people.iter().enumerate().filter(|(_, p)| !p.2) {
        w.add_heir(heir(i, p));
    }
    w.line_from = w.next_heir_id;
    for (i, p) in people.iter().enumerate().filter(|(_, p)| p.2) {
        w.add_heir(heir(i, p));
    }
    g
}

/// The name of the heir the law puts on the throne.
fn heir(law: &str, people: &[Person]) -> Option<String> {
    let data = data();
    let g = game(&data, law, people);
    let i = sim::successor(&g.world, &data)?;
    Some(g.world.heirs[i].name.clone())
}

/// The ruler's elder daughter p0 (20), his son p1 (18), his brother p2 (40) and sister p3 (45).
const FAMILY: [Person; 4] = [
    (F, 20, true, 50),
    (M, 18, true, 50),
    (M, 40, false, 50),
    (F, 45, false, 50),
];

#[test]
fn absolute_primogeniture_takes_the_eldest_child() {
    assert_eq!(heir("law_primogeniture", &FAMILY).as_deref(), Some("p0"));
    // No children: the collateral line in its order (birth order where it comes from).
    assert_eq!(
        heir("law_primogeniture", &FAMILY[2..]).as_deref(),
        Some("p0")
    );
}

#[test]
fn male_primogeniture_takes_a_son_then_a_daughter() {
    assert_eq!(heir("law_male", &FAMILY).as_deref(), Some("p1"));
    // Without a son, the ruler's daughter before his brother.
    let no_son = [FAMILY[0], FAMILY[2], FAMILY[3]];
    assert_eq!(heir("law_male", &no_son).as_deref(), Some("p0"));
    // A brother before a sister in the collateral line.
    assert_eq!(
        heir("law_male", &[FAMILY[3], FAMILY[2]]).as_deref(),
        Some("p1")
    );
}

#[test]
fn salic_law_takes_men_only() {
    assert_eq!(heir("law_salic", &FAMILY).as_deref(), Some("p1"));
    // No son: the brother, never the daughter.
    let no_son = [FAMILY[0], FAMILY[2], FAMILY[3]];
    assert_eq!(heir("law_salic", &no_son).as_deref(), Some("p1"));
    assert_eq!(heir("law_salic", &[FAMILY[0], FAMILY[3]]), None);
}

/// Acceptance: no man left under the Salic law, the dynasty ends without an heir.
#[test]
fn salic_law_without_men_ends_the_dynasty() {
    let mut data = data();
    data.heirs.birth = vec![];
    let g = game(&data, "law_salic", &[FAMILY[0], FAMILY[3]]);
    let end = ReignEnd {
        cause: "illness".into(),
        tick: g.world.tick,
        world: g.world.clone(),
    };
    let c = sim::run(end.clone(), &data, Rng::from_seed(1));
    assert_eq!((c.fall, c.rulers.len()), (FallReason::NoHeir, 1));
    // The same women under absolute primogeniture: a queen.
    let mut end = end;
    end.world.flags.remove("law_salic");
    end.world.flags.insert("law_primogeniture".into());
    let c = sim::run(end, &data, Rng::from_seed(1));
    assert_eq!(c.rulers.len(), 2);
    assert_eq!(c.rulers[1].name, "p0");
}

#[test]
fn seniority_takes_the_eldest_man_of_the_line_first() {
    let line = [
        (M, 20, true, 50),
        (M, 40, false, 50),
        (M, 45, false, 50),
        (F, 50, false, 50),
    ];
    assert_eq!(heir("law_seniority", &line).as_deref(), Some("p2"));
    // No brother left: the eldest son, before an elder daughter.
    let children = [(F, 22, true, 50), (M, 20, true, 50), (M, 21, true, 50)];
    assert_eq!(heir("law_seniority", &children).as_deref(), Some("p2"));
}

#[test]
fn elective_law_takes_the_ablest_the_eldest_on_a_tie() {
    let line = [
        (M, 30, true, 60),
        (F, 20, true, 80),
        (M, 12, true, 80),
        (M, 40, false, 70),
    ];
    assert_eq!(heir("law_elective", &line).as_deref(), Some("p1"));
}

/// Stage 15 meets stage 16: a brother crowned under seniority has the children born before
/// his coronation in his own line: before the collateral line, his in the family tree.
#[test]
fn a_brother_crowned_by_seniority_brings_his_own_children() {
    let mut data = data();
    data.heirs.birth = vec![(0, Fx(0)), (20, Fx::from_int(100))];
    data.heirs.death = vec![];
    data.sim.max_years = 1;
    let g = game(
        &data,
        "law_seniority",
        &[(M, 10, true, 50), (M, 24, false, 50)],
    );
    let end = ReignEnd {
        cause: "illness".into(),
        tick: g.world.tick,
        world: g.world.clone(),
    };
    let c = sim::run(end, &data, Rng::from_seed(1));
    assert_eq!(c.rulers[1].name, "p1");
    let w = &c.entries[0].snapshot;
    let ages: Vec<u32> = w.heirs.iter().map(|h| h.age).collect();
    assert_eq!(ages, [4, 3, 2, 1, 10], "his four, then his nephew");
    assert!(w.heirs[..4].iter().all(|h| h.id >= w.line_from));
    assert!(w.heirs[4].id < w.line_from);
    let king = w.kin.iter().position(|k| k.name == "p1").unwrap();
    let parent = |h: &Heir| w.kin.iter().find(|k| k.heir == Some(h.id)).unwrap().parent;
    assert!(w.heirs[..4].iter().all(|h| parent(h) == Some(king)));
}

/// Acceptance: under partition every younger son of the late ruler gets one crown province,
/// the farthest from the capital first, as a house of his name; the capital stays.
#[test]
fn partition_gives_the_younger_sons_a_province_each() {
    let mut data = data();
    data.heirs.birth = vec![];
    data.sim.max_years = 1;
    let sons = [
        (M, 30, true, 50),
        (F, 28, true, 50),
        (M, 25, true, 50),
        (M, 20, true, 50),
    ];
    let g = game(&data, "law_partition", &sons);
    let crown = |w: &bd_core::state::World| -> Vec<ProvinceId> {
        (w.provinces.values())
            .filter(|p| p.holder == Holder::Crown)
            .map(|p| p.id.clone())
            .collect()
    };
    let before = crown(&g.world);
    let end = ReignEnd {
        cause: "illness".into(),
        tick: g.world.tick,
        world: g.world.clone(),
    };
    let c = sim::run(end, &data, Rng::from_seed(1));
    assert_eq!(c.rulers[1].name, "p0");
    let e = c
        .entries
        .iter()
        .find(|e| e.title == "Земли поделены между братьями");
    let w = &e.expect("the partition is told").snapshot;
    let after = crown(w);
    assert_eq!(after.len(), before.len() - 2);
    assert!(after.contains(&w.capital.province));
    // The two sons, p2 then p3, each a house of his own on one province.
    let houses: Vec<_> = (w.vassals.values())
        .filter(|v| v.name == "p2" || v.name == "p3")
        .collect();
    assert_eq!(houses.len(), 2);
    for v in houses {
        let held: Vec<_> = (w.provinces.values())
            .filter(|p| p.holder == Holder::Vassal(v.id.clone()))
            .collect();
        assert_eq!(held.len(), 1);
        // The house's (loyalty, strength) of the law.
        let law = data.heirs.laws.iter().find(|l| l.flag == "law_partition");
        assert_eq!((v.loyalty, v.strength), law.unwrap().house);
        // The farthest of the crown's lands first.
        let nearer = before.iter().map(|id| &g.world.provinces[id]);
        let max = nearer.filter(|p| p.id != g.world.capital.province);
        let max = max.map(|p| p.distance_to_capital).max().unwrap();
        if v.name == "p2" {
            assert_eq!(held[0].distance_to_capital, max);
        }
    }
    // The daughter gets none; the sons stay in the line.
    assert!(w.heirs.iter().any(|h| h.name == "p2") && w.heirs.iter().any(|h| h.name == "p3"));
}

/// Acceptance: a newborn's sex is a roll of the game's rng: the same seed, the same sexes;
/// `male_percent` sets the odds.
#[test]
fn the_sex_of_a_newborn_follows_the_seed() {
    let sexes = |seed: u64, male: i64| {
        let mut data = data();
        data.heirs.male_percent = Fx::from_int(male);
        data.heirs.birth = vec![(0, Fx::from_int(100))];
        data.heirs.death = vec![];
        let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
        let mut g = Game::new(data, &preset, seed);
        for _ in 0..20 {
            g.wait().unwrap();
        }
        let born = g.world.heirs.iter().skip(1);
        born.map(|h| (h.sex, h.name.clone())).collect::<Vec<_>>()
    };
    let a = sexes(7, 50);
    assert_eq!(a.len(), 20);
    assert_eq!(a, sexes(7, 50));
    assert_ne!(a, sexes(8, 50));
    assert!(
        a.iter().any(|x| x.0 == M) && a.iter().any(|x| x.0 == F),
        "{a:?}"
    );
    // Sons take the sons' names, daughters the daughters'.
    let d = data();
    for (sex, name) in &a {
        let pool = if *sex == M {
            &d.names.heirs
        } else {
            &d.names.daughters
        };
        assert!(pool.contains(name), "{name}");
    }
    assert!(sexes(7, 100).iter().all(|x| x.0 == M));
    assert!(sexes(7, 0).iter().all(|x| x.0 == F));
}

fn axis(g: &Game, a: &str) -> Fx {
    g.world.axes[&AxisId(a.into())]
}

fn laws(g: &Game) -> Vec<&String> {
    g.world
        .flags
        .iter()
        .filter(|f| f.starts_with("law_"))
        .collect()
}

/// Acceptance: a change of law costs its price at the start, takes its years with the
/// resistance (stage 19: the church's anchor 10 lower), sets the new flag only at the end and
/// is closed during a dispute.
#[test]
fn changing_the_law_costs_money_and_years() {
    let data = data();
    let new = || {
        let mut g = game(&data, "law_primogeniture", &FAMILY);
        g.world
            .axes
            .insert(AxisId("treasury".into()), Fx::from_int(500));
        g
    };
    let mut g = new();
    let id = "enact_law_salic";
    let a = data.actions.iter().find(|a| a.id == id).unwrap();
    assert_eq!((a.cost, a.duration_years.0), (Fx::from_int(45), 2));
    let church = |g: &Game| {
        let def = g.data.axes.iter().find(|a| a.id.0 == "loyalty_church");
        bd_core::graph::anchor(&g.data, &g.world, def.unwrap())
    };
    assert_eq!(church(&g), Fx::from_int(50));
    g.start_action(id, None).unwrap();
    assert_eq!(axis(&g, "treasury"), Fx::from_int(500 - 45));
    assert_eq!(church(&g), Fx::from_int(40));
    g.wait().unwrap();
    assert_eq!(laws(&g), ["law_primogeniture"]);
    g.wait().unwrap();
    assert_eq!(laws(&g), ["law_salic"]);
    assert_eq!(church(&g), Fx::from_int(50));
    // Under a contested succession no change is offered.
    let mut g = new();
    g.world.flags.insert("succession_contested".into());
    let offered = g.available_actions().into_iter();
    assert!(!offered.map(|(id, _)| id).any(|id| {
        id.starts_with("enact_law_")
            && data
                .law(&id["enact_".len()..])
                .is_some_and(|l| l.group == "Наследование")
    }));
}

/// The automaton changes the law by its weights (`law_*` keys: the new law's weight less the
/// old one's), and the chronicle tells the new law.
#[test]
fn the_automaton_changes_the_law_by_its_weights() {
    let mut data = data();
    let mut g = game(&data, "law_primogeniture", &FAMILY);
    g.world
        .axes
        .insert(AxisId("treasury".into()), Fx::from_int(500));
    let mut auto = AutoChooser {
        weights: [("law_salic".to_string(), Fx::from_int(100))].into(),
        noise: Fx(0),
    };
    let picked = auto.action(&mut g).map(|(id, _)| id);
    assert_eq!(picked.as_deref(), Some("enact_law_salic"));
    // Leaving a law it holds dearer is worth nothing to it.
    auto.weights
        .insert("law_primogeniture".into(), Fx::from_int(200));
    assert_eq!(auto.action(&mut g), None);
    data.sim
        .auto
        .base
        .insert("law_salic".into(), Fx::from_int(100));
    data.sim.max_years = 10;
    // A firm claim: no dispute to close the change.
    g.world.heirs[0].claim = Fx::from_int(100);
    let end = ReignEnd {
        cause: "illness".into(),
        tick: g.world.tick,
        world: g.world.clone(),
    };
    let c = sim::run(end, &data, Rng::from_seed(1));
    let e = c
        .entries
        .iter()
        .find(|e| e.title == "Новый закон о престоле");
    let e = e.expect("the change is told");
    assert_eq!(
        e.text,
        "Отныне престол наследуют по закону «Салический закон»."
    );
    assert!(e.snapshot.flags.contains("law_salic"));
}

/// The coronation entry for an heir of `sex` and `claim` under absolute primogeniture with
/// these axes; no trait rolls unless `data` has some, no births before.
fn crowned(data: &Data, sex: Sex, claim: i64, axes: &[(&str, i64)]) -> sim::ChronicleEntry {
    let mut data = data.clone();
    (data.heirs.birth, data.sim.max_years) = (vec![], 1);
    let mut g = game(&data, "law_primogeniture", &[(sex, 30, true, 50)]);
    g.world.heirs[0].claim = Fx::from_int(claim);
    for (a, v) in axes {
        g.world.axes.insert(AxisId((*a).into()), Fx::from_int(*v));
    }
    let end = ReignEnd {
        cause: "illness".into(),
        tick: g.world.tick,
        world: g.world.clone(),
    };
    sim::run(end, &data, Rng::from_seed(1)).entries.remove(0)
}

/// `data` with only the coronation step under test, no traits and claims as given (no
/// `rightful_claim`).
fn plain(f: impl FnOnce(&mut bd_core::data::CoronationRules)) -> Data {
    let mut data = data();
    data.sim.traits.clear();
    data.heirs
        .laws
        .iter_mut()
        .for_each(|l| l.rightful_claim = Fx(0));
    data.coronation = Default::default();
    f(&mut data.coronation);
    data
}

fn at(e: &sim::ChronicleEntry, a: &str) -> i64 {
    e.snapshot.axes[&AxisId(a.into())].0 / Fx::SCALE
}

/// Acceptance: every faction axis moves `reset` of the way back to its default (50).
#[test]
fn the_coronation_moves_the_factions_toward_their_defaults() {
    let data = plain(|c| c.reset = Fx(300));
    let e = crowned(
        &data,
        M,
        80,
        &[("loyalty_nobles", 90), ("loyalty_church", 20)],
    );
    assert_eq!(
        (at(&e, "loyalty_nobles"), at(&e, "loyalty_church")),
        (78, 29)
    );
    // Not a faction: bureaucracy stays.
    let e = crowned(&data, M, 80, &[("bureaucracy", 90)]);
    assert_eq!(at(&e, "bureaucracy"), 90);
    // The loyalty axis follows its factions: (78 * 2 + 50 + 50) / 4.
    let factions = [
        ("loyalty_nobles", 90),
        ("loyalty_church", 50),
        ("loyalty_people", 50),
    ];
    let e = crowned(&data, M, 80, &factions);
    assert_eq!(at(&e, "loyalty"), 64);
}

/// Acceptance: legitimacy moves its share of the way toward the new ruler's claim.
#[test]
fn the_coronation_takes_legitimacy_from_the_claim() {
    let data = plain(|c| c.legitimacy_from_claim = Some((AxisId("legitimacy".into()), Fx(500))));
    let e = crowned(&data, M, 80, &[("legitimacy", 40)]);
    assert_eq!(at(&e, "legitimacy"), 60);
    let e = crowned(&data, M, 70, &[("legitimacy", 90)]);
    assert_eq!(at(&e, "legitimacy"), 80);
}

/// Acceptance: a contested succession (a claim below the law's crisis_claim, 70) costs
/// legitimacy and the nobles' loyalty.
#[test]
fn a_contested_coronation_costs_legitimacy_and_the_nobles() {
    let ax = |a: &str| AxisId(a.into());
    let data = plain(|c| {
        c.contested = vec![
            (ax("legitimacy"), Fx::from_int(-10)),
            (ax("loyalty_nobles"), Fx::from_int(-10)),
        ]
    });
    let axes = [("legitimacy", 50), ("loyalty_nobles", 50)];
    let firm = crowned(&data, M, 80, &axes);
    let contested = crowned(&data, M, 50, &axes);
    assert!(contested.snapshot.flags.contains("succession_contested"));
    assert!(!firm.snapshot.flags.contains("succession_contested"));
    assert_eq!(
        (at(&firm, "legitimacy"), at(&firm, "loyalty_nobles")),
        (50, 50)
    );
    assert_eq!(
        (
            at(&contested, "legitimacy"),
            at(&contested, "loyalty_nobles")
        ),
        (40, 40)
    );
}

const FACTIONS: [(&str, i64); 3] = [
    ("loyalty_nobles", 50),
    ("loyalty_church", 50),
    ("loyalty_people", 50),
];

/// Acceptance: the traits of the new ruler shift the axes once; the chronicle tells the
/// largest shift, of a king or of a queen.
#[test]
fn the_traits_of_a_new_ruler_shift_the_axes() {
    let mut data = plain(|_| {});
    data.sim.traits = self::data().sim.traits;
    // Pious and warlike for sure, the other two never.
    for t in &mut data.sim.traits {
        let sure = t.id == "pious" || t.id == "warlike";
        (t.percent, t.ability_k, t.studying) =
            (Fx::from_int(if sure { 100 } else { 0 }), Fx(0), Fx(0));
    }
    data.sim
        .traits
        .iter_mut()
        .find(|t| t.id == "pious")
        .unwrap()
        .axes[0]
        .1 = Fx::from_int(15);
    let e = crowned(&data, M, 80, &FACTIONS);
    assert_eq!(
        (at(&e, "loyalty_church"), at(&e, "loyalty_nobles")),
        (65, 60)
    );
    assert_eq!(at(&e, "loyalty_people"), 50);
    assert_eq!(
        e.text,
        "Престол наследует p0. Церковь ликует: на троне набожный король."
    );
    let e = crowned(&data, F, 80, &FACTIONS);
    assert_eq!(
        e.text,
        "Престол наследует p0. Церковь ликует: на троне набожная королева."
    );
}

fn court(id: &str) -> NeighbourId {
    NeighbourId(id.into())
}

/// The default preset with these heirs, a full treasury and only the `marriage_refused`
/// event; a suit of `percent` flat, unless None (the rules.ron formula).
fn suitors(percent: Option<i64>, people: &[Person]) -> Game {
    let mut data = data();
    let heirs_ron = read("events/heirs.ron");
    data.add_events(&heirs_ron).unwrap();
    data.events
        .retain(|e| e.id == "marriage_refused" || e.id == "heir_marriage");
    data.events.iter_mut().for_each(|e| e.weight = 0);
    if let Some(p) = percent {
        let m = &mut data.marriage;
        (m.percent, m.relation_k, m.strength_k, m.axes) = (Fx::from_int(p), Fx(0), Fx(0), vec![]);
    }
    let mut g = game(&data, "law_primogeniture", people);
    g.world
        .axes
        .insert(AxisId("treasury".into()), Fx::from_int(1000));
    g
}

fn courts(g: &Game) -> Vec<String> {
    let suit = g
        .available_actions()
        .into_iter()
        .find(|(id, _)| id == "marry_neighbour");
    let targets = suit.map(|(_, t)| t).unwrap_or_default();
    (targets.into_iter())
        .map(|t| match t {
            Target::Neighbour(n) => n.0,
            t => panic!("{t:?}"),
        })
        .collect()
}

/// War on the court for a province of its on the border, declared within the year.
fn war_on(g: &mut Game, n: &str) {
    let (_, targets) = (g.available_actions().into_iter())
        .find(|(id, _)| id == "declare_war")
        .unwrap();
    let theirs = |t: &&Target| match t {
        Target::Province(p) => g.world.provinces[p].holder == Holder::Foreign(court(n)),
        _ => false,
    };
    let t = targets.iter().find(theirs).cloned();
    g.start_action("declare_war", t).unwrap();
    g.wait().unwrap();
    assert_eq!(g.world.war.as_ref().map(|w| w.enemy.0.as_str()), Some(n));
}

fn suit(g: &mut Game, n: &str) {
    g.start_action("marry_neighbour", Some(Target::Neighbour(court(n))))
        .unwrap();
    g.wait().unwrap();
}

/// Acceptance: a court at war with the crown or with a relation below `refuse_below` turns
/// the suit away outright: no chance, no suit.
#[test]
fn a_suit_is_turned_away_at_war_and_at_low_relations() {
    let mut g = suitors(None, &[(M, 20, true, 50)]);
    g.world.heirs[0].married = false;
    let m = g.data.marriage.clone();
    // Нордмарк at -40, below -20; the others take suits.
    let chance = |g: &Game, n: &str| m.chance(&g.world, &g.data, &court(n));
    assert_eq!(chance(&g, "nordmark"), Fx(0));
    assert!(chance(&g, "vestrum") > Fx(0) && chance(&g, "purpur") > Fx(0));
    assert_eq!(courts(&g), ["purpur", "vestrum"]);
    g.world
        .neighbours
        .get_mut(&court("nordmark"))
        .unwrap()
        .relation = Fx::from_int(-20);
    assert!(chance(&g, "nordmark") > Fx(0), "at the threshold");
    assert_eq!(courts(&g), ["nordmark", "purpur", "vestrum"]);
    // At war with Веструм (and Нордмарк cold again: war on a friend costs trust).
    war_on(&mut g, "vestrum");
    assert_eq!(chance(&g, "vestrum"), Fx(0));
    assert_eq!(courts(&g), ["purpur"]);
}

/// Acceptance: the chance follows the formula of rules.ron and the roll the seed.
#[test]
fn the_chance_of_a_suit_follows_the_seed() {
    let mut g = suitors(None, &[(M, 20, true, 50)]);
    g.world.heirs[0].married = false;
    let (w, d) = (&g.world, &g.data);
    // 30 + relation 40 * 0.5 + prestige 20 * 0.1 + (ours / 45 - 1) * 20.
    let (ours, theirs) = bd_core::war::strengths(w, d, &court("vestrum"));
    let one = Fx::from_int(1);
    let want = Fx::from_int(52) + (ours / theirs - one) * Fx::from_int(20);
    assert_eq!(d.marriage.chance(w, d, &court("vestrum")), want);
    let wed = |seed: u64| {
        let mut g = suitors(Some(50), &[(M, 20, true, 50)]);
        g.world.heirs[0].married = false;
        g.rng = Rng::from_seed(seed);
        suit(&mut g, "vestrum");
        g.world.unions.contains_key(&court("vestrum"))
    };
    let first: Vec<bool> = (0..20).map(wed).collect();
    assert_eq!(first, (0..20).map(wed).collect::<Vec<_>>());
    assert!(first.contains(&true) && first.contains(&false), "{first:?}");
}

/// Acceptance: a refusal costs prestige and some relation, and is told as an event.
#[test]
fn a_refused_suit_costs_prestige() {
    let mut g = suitors(Some(100), &[(M, 20, true, 50)]);
    g.world.heirs[0].married = false;
    g.start_action("marry_neighbour", Some(Target::Neighbour(court("purpur"))))
        .unwrap();
    // The court turns cold while the envoys travel: no chance left.
    g.world
        .neighbours
        .get_mut(&court("purpur"))
        .unwrap()
        .relation = Fx::from_int(-50);
    let prestige = axis(&g, "prestige");
    let step = g.wait().unwrap();
    assert_eq!(axis(&g, "prestige"), prestige - Fx::from_int(5));
    assert!(g.world.unions.is_empty() && !g.world.heirs[0].married);
    let bd_core::game::Step::Event(v) = step else {
        panic!("{step:?}");
    };
    assert_eq!(v.title, "Пурпуляндия отверг сватовство");
}

/// Acceptance: a married heir does not marry again, by a suit or by `heir_marriage`.
#[test]
fn a_married_heir_does_not_marry_again() {
    let mut g = suitors(Some(100), &[(M, 20, true, 50), (M, 18, true, 50)]);
    (g.world.heirs[0].married, g.world.heirs[1].married) = (false, false);
    suit(&mut g, "vestrum");
    assert_eq!(
        g.world.unions[&court("vestrum")].spouse,
        Some(g.world.heirs[0].id)
    );
    assert!(g.world.heirs[0].married && !g.world.heirs[1].married);
    // The event finds only the unmarried one, and once he weds, nobody.
    let e = g.data.events.iter_mut().find(|e| e.id == "heir_marriage");
    (e.unwrap().weight, g.data.quiet_weight) = (1000, 0);
    let p1 = g.world.heirs[1].id;
    let bd_core::game::Step::Event(v) = g.wait().unwrap() else {
        panic!("heir_marriage fires");
    };
    assert_eq!(v.target, Some(Target::Heir(p1)));
    g.choose(0).unwrap();
    assert!(g.world.heirs[1].married);
    for _ in 0..5 {
        assert_eq!(g.wait().unwrap(), bd_core::game::Step::Idle);
    }
    // Nobody left to wed: no suit at all.
    assert!(courts(&g).is_empty());
}

/// Acceptance: unions with two courts at once, each until its spouse dies or a war comes.
#[test]
fn unions_with_two_courts_end_with_the_spouse_and_with_war() {
    let mut g = suitors(Some(100), &[(M, 20, true, 50), (M, 18, true, 50)]);
    (g.world.heirs[0].married, g.world.heirs[1].married) = (false, false);
    suit(&mut g, "vestrum");
    suit(&mut g, "purpur");
    let bonds = |g: &Game| g.bonds().into_iter().map(|(n, ..)| n.0).collect::<Vec<_>>();
    assert_eq!(bonds(&g), ["purpur", "vestrum"]);
    // The war on Веструм ends that union only.
    war_on(&mut g, "vestrum");
    assert_eq!(bonds(&g), ["purpur"]);
    // The death of Purpur's son-in-law ends the other.
    g.world.heirs.remove(1);
    g.wait().unwrap();
    assert!(bonds(&g).is_empty());
}

/// The new ruler comes to the throne wed or not as he was, with his unions; the late ruler's
/// unions end with him.
#[test]
fn a_ruler_is_crowned_with_his_marriage_and_his_unions() {
    let crowned = |married: bool| {
        let mut g = suitors(Some(100), &[(M, 20, true, 50)]);
        g.world.heirs[0].married = married;
        let since = g.world.tick;
        let id = g.world.heirs[0].id;
        let union = |spouse| bd_core::state::Union { spouse, since };
        g.world.unions.insert(court("vestrum"), union(None));
        g.world.unions.insert(court("purpur"), union(Some(id)));
        g.data.sim.max_years = 1;
        let end = ReignEnd {
            cause: "illness".into(),
            tick: g.world.tick,
            world: g.world.clone(),
        };
        sim::run(end, &g.data, Rng::from_seed(1))
            .entries
            .remove(0)
            .snapshot
    };
    let w = crowned(true);
    assert!(w.flags.contains("married"));
    let unions: Vec<_> = w
        .unions
        .iter()
        .map(|(n, u)| (n.0.as_str(), u.spouse))
        .collect();
    assert_eq!(unions, [("purpur", None)]);
    assert!(!crowned(false).flags.contains("married"));
}

// Stage 17: the rightful heir, the designated heir, bastards.

/// The coronation entry of a dynasty whose founder dies now under `law` with these heirs;
/// no births, no traits.
fn crown_under(data: &Data, law: &str, people: &[Person]) -> sim::ChronicleEntry {
    let mut data = data.clone();
    (data.heirs.birth, data.sim.max_years) = (vec![], 1);
    data.sim.traits.clear();
    let g = game(&data, law, people);
    let end = ReignEnd {
        cause: "illness".into(),
        tick: g.world.tick,
        world: g.world.clone(),
    };
    sim::run(end, &data, Rng::from_seed(1)).entries.remove(0)
}

fn contested(e: &sim::ChronicleEntry) -> bool {
    e.snapshot.flags.contains("succession_contested")
}

/// Acceptance: the heir the law puts first has `rightful_claim` at once, the next one too as
/// soon as he is first; the rest move toward `others` year by year.
#[test]
fn the_rightful_heir_has_his_claim_at_once() {
    let mut data = data();
    data.heirs.birth = vec![];
    data.heirs.death = vec![];
    let mut g = game(
        &data,
        "law_primogeniture",
        &[(M, 10, true, 50), (M, 8, true, 50)],
    );
    g.data.events.clear();
    let claims = |g: &Game| g.world.heirs.iter().map(|h| h.claim).collect::<Vec<_>>();
    g.wait().unwrap();
    assert_eq!(claims(&g), [Fx::from_int(90), Fx::from_int(48)]);
    g.world.heirs.remove(0);
    g.wait().unwrap();
    assert_eq!(claims(&g), [Fx::from_int(90)]);
    // Crowned the same tick he became first: still with his right.
    let e = crown_under(&data, "law_primogeniture", &[(M, 30, true, 50)]);
    assert!(!contested(&e));
}

/// Disputes besides the claim: a child crowned below `regency_age`, an heir of ability below
/// the threshold of `dispute_weak`; a grown, able rightful heir reigns undisputed.
#[test]
fn a_child_or_a_weak_heir_is_disputed() {
    let mut data = data();
    (data.heirs.dispute_minor, data.heirs.dispute_weak) =
        (Fx::from_int(100), (Fx::from_int(80), Fx::from_int(100)));
    let law = "law_primogeniture";
    assert!(!contested(&crown_under(&data, law, &[(M, 30, true, 90)])));
    assert!(contested(&crown_under(&data, law, &[(M, 30, true, 50)])));
    assert!(contested(&crown_under(&data, law, &[(M, 10, true, 90)])));
}

/// Acceptance: under male primogeniture a daughter comes to the throne only without sons, and
/// her coronation is contested and costs legitimacy and the nobles; a son's is not.
#[test]
fn male_primogeniture_crowns_a_daughter_only_without_sons_contested() {
    let data = data();
    let son = crown_under(&data, "law_male", &[(F, 20, true, 50), (M, 18, true, 50)]);
    assert!(son.text.contains("p1"), "{}", son.text);
    assert!(!contested(&son));
    let daughter = crown_under(&data, "law_male", &[(F, 20, true, 50), (M, 40, false, 50)]);
    assert!(daughter.text.contains("p0"), "{}", daughter.text);
    assert!(contested(&daughter));
    // The same daughter under absolute primogeniture: no dispute, more legitimacy and nobles.
    let queen = crown_under(
        &data,
        "law_primogeniture",
        &[(F, 20, true, 50), (M, 40, false, 50)],
    );
    assert!(!contested(&queen));
    assert!(at(&daughter, "legitimacy") < at(&queen, "legitimacy"));
    assert!(at(&daughter, "loyalty_nobles") < at(&queen, "loyalty_nobles"));
}

/// Names `heir` (an index) by the action `designate_heir`, done at once.
fn designate(g: &mut Game, heir: usize) {
    g.world
        .axes
        .insert(AxisId("treasury".into()), Fx::from_int(500));
    let id = g.world.heirs[heir].id;
    g.start_action("designate_heir", Some(Target::Heir(id)))
        .unwrap();
    g.data.events.clear();
    g.wait().unwrap();
}

/// The dynasty after `g`'s reign ends now, one year.
fn next_reign(g: &Game) -> sim::Chronicle {
    let mut data = g.data.clone();
    (data.heirs.birth, data.sim.max_years) = (vec![], g.world.tick.0 + 1);
    data.sim.traits.clear();
    let end = ReignEnd {
        cause: "illness".into(),
        tick: g.world.tick,
        world: g.world.clone(),
    };
    sim::run(end, &data, Rng::from_seed(1))
}

/// Acceptance: the designated heir comes to the throne instead of the rightful one, who keeps
/// his claim; a hostage named gives way to the law until he is back.
#[test]
fn the_designated_heir_succeeds_over_the_rightful_one() {
    let mut data = data();
    data.heirs.death = vec![];
    let mut g = game(
        &data,
        "law_primogeniture",
        &[(M, 20, true, 50), (M, 18, true, 50)],
    );
    designate(&mut g, 1);
    assert_eq!(g.world.designated, Some(g.world.heirs[1].id));
    assert_eq!(sim::successor(&g.world, &g.data), Some(1));
    assert_eq!(sim::rightful(&g.world, &g.data), Some(0));
    assert_eq!(g.world.heirs[0].claim, Fx::from_int(90));
    let c = next_reign(&g);
    assert_eq!(
        (c.rulers[1].name.as_str(), c.rulers[1].designated),
        ("p1", true)
    );
    let w = &c.entries[0].snapshot;
    assert_eq!(w.designated, None);
    assert_eq!(w.heirs[0].name, "p0");
    // A hostage named: the law decides while he is away.
    g.world.heirs[1].status = HeirStatus::Hostage(court("vestrum"));
    assert_eq!(sim::successor(&g.world, &g.data), Some(0));
}

/// Acceptance: naming an heir over the law costs `designate_penalty` at once and makes his
/// coronation contested by `designate_dispute`; naming the rightful heir costs nothing.
#[test]
fn designating_over_the_law_costs_and_raises_the_dispute() {
    let mut data = data();
    data.heirs.death = vec![];
    data.heirs.designate_dispute = Fx::from_int(100);
    let people = [(M, 20, true, 50), (M, 18, true, 50)];
    let named = |heir: usize| {
        let mut g = game(&data, "law_primogeniture", &people);
        g.world.heirs[1].claim = Fx::from_int(80);
        designate(&mut g, heir);
        g
    };
    let (over, lawful) = (named(1), named(0));
    for (a, v) in [
        ("legitimacy", -10),
        ("loyalty_nobles", -5),
        ("loyalty_church", -5),
    ] {
        assert_eq!(axis(&over, a) - axis(&lawful, a), Fx::from_int(v), "{a}");
    }
    assert!(contested(&next_reign(&over).entries[0]));
    assert!(!contested(&next_reign(&lawful).entries[0]));
    let mut data = data.clone();
    data.heirs.designate_dispute = Fx(0);
    let mut g = game(&data, "law_primogeniture", &people);
    g.world.heirs[1].claim = Fx::from_int(80);
    designate(&mut g, 1);
    assert!(!contested(&next_reign(&g).entries[0]));
}

/// The dispute's «Назначить младшего» names him for real (stage 16 question 1).
#[test]
fn the_heir_dispute_designates_the_younger() {
    let mut data = data();
    data.add_events(&read("events/heirs.ron")).unwrap();
    let e = data.events.iter().find(|e| e.id == "heir_dispute").unwrap();
    let younger = &e.choices[1].effects;
    assert!(younger.contains(&bd_core::rules::Effect::HeirOp(
        bd_core::rules::HeirOp::Designate(1)
    )));
}

/// A year of an unmarried ruler whose every year brings a child, heirs as given.
fn unwed_year(people: &[Person]) -> Game {
    let mut data = data();
    (data.heirs.birth, data.heirs.death) = (vec![(0, Fx::from_int(500))], vec![]);
    data.heirs.unmarried = Fx::from_int(1);
    let mut g = game(&data, "law_primogeniture", people);
    g.world.flags.remove("married");
    g.data.events.clear();
    g.wait().unwrap();
    g
}

/// Acceptance: a child born out of wedlock is a bastard: out of the line, in the family tree;
/// with only bastards the dynasty ends.
#[test]
fn bastards_are_out_of_the_line() {
    let g = unwed_year(&[]);
    let w = &g.world;
    assert!(w.heirs.is_empty());
    assert_eq!(w.bastards.len(), 1);
    assert!(w.bastards[0].bastard && w.kin.last().unwrap().bastard);
    assert_eq!(sim::successor(w, &g.data), None);
    assert_eq!(w.kin.last().unwrap().died, None);
    // A year on he is alive still, and the ruler dies with no heir.
    let mut data = g.data.clone();
    data.heirs.birth = vec![];
    let end = ReignEnd {
        cause: "illness".into(),
        tick: g.world.tick,
        world: g.world.clone(),
    };
    let c = sim::run(end, &data, Rng::from_seed(1));
    assert_eq!((c.fall, c.rulers.len()), (FallReason::NoHeir, 1));
}

/// Acceptance: a bastard recognized joins the line with `bastard_claim`, after every lawful
/// heir, without the rightful claim; the church frowns. The automaton recognizes one only
/// with no heir left.
#[test]
fn a_recognized_bastard_joins_the_line_with_a_low_claim() {
    let mut g = unwed_year(&[(M, 5, true, 50)]);
    g.data.heirs.birth = vec![];
    let church = axis(&g, "loyalty_church");
    g.world
        .axes
        .insert(AxisId("treasury".into()), Fx::from_int(500));
    g.start_action("recognize_bastard", None).unwrap();
    g.wait().unwrap();
    let w = &g.world;
    assert!(w.bastards.is_empty());
    let names: Vec<_> = w
        .heirs
        .iter()
        .map(|h| (h.name.as_str(), h.bastard))
        .collect();
    assert_eq!(names, [("p0", false), (w.heirs[1].name.as_str(), true)]);
    assert_eq!(w.heirs[1].claim, Fx::from_int(20));
    assert_eq!(sim::rightful(w, &g.data), Some(0));
    assert!(axis(&g, "loyalty_church") < church);
    // Alone in the line he is first, yet his claim only creeps toward `others`.
    g.world.heirs.remove(0);
    g.wait().unwrap();
    assert_eq!(sim::successor(&g.world, &g.data), Some(0));
    assert_eq!(g.world.heirs[0].claim, Fx::from_int(22));
    // The automaton: not while a lawful heir lives, at once when none is left.
    let auto = |people: &[Person]| {
        let mut g = unwed_year(people);
        g.world
            .axes
            .insert(AxisId("treasury".into()), Fx::from_int(500));
        let a = AutoChooser::for_ruler(&g.data, &g.world.ruler);
        (0..20).any(|_| {
            a.action(&mut g)
                .is_some_and(|(id, _)| id == "recognize_bastard")
        })
    };
    assert!(auto(&[]));
    assert!(!auto(&[(M, 5, true, 50)]));
}

/// Acceptance: the children an heir had before his coronation are lawful only from his
/// wedding on; before it, and without one, bastards.
#[test]
fn children_before_the_coronation_are_lawful_only_in_wedlock() {
    let crowned = |married_in: Option<u32>| {
        let mut data = data();
        (data.heirs.birth, data.heirs.death) = (vec![(0, Fx::from_int(100))], vec![]);
        data.sim.max_years = 1;
        let mut g = game(&data, "law_primogeniture", &[(M, 30, true, 50)]);
        let h = &mut g.world.heirs[0];
        (h.married, h.married_in) = (married_in.is_some(), married_in);
        let end = ReignEnd {
            cause: "illness".into(),
            tick: g.world.tick,
            world: g.world.clone(),
        };
        let w = sim::run(end, &data, Rng::from_seed(1))
            .entries
            .remove(0)
            .snapshot;
        (w.heirs.len(), w.bastards.len())
    };
    // 30 years old in 1187, wed in 1182 at 25: five lawful children, his 25th year to 29th;
    // bastards at `unmarried` (0.2) of a sure birth in the nine years before.
    let (lawful, bastards) = crowned(Some(1182));
    assert_eq!(lawful, 5);
    assert!((1..=9).contains(&bastards), "{bastards}");
    let (lawful, bastards) = crowned(None);
    assert_eq!(lawful, 0);
    assert!(bastards > 0);
}
