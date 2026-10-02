//! Stage 16: succession laws, the sex of heirs.

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{Game, ReignEnd};
use bd_core::rng::Rng;
use bd_core::sim::{self, FallReason};
use bd_core::state::{Heir, HeirStatus, Holder, Preset, ProvinceId, Sex};
use std::fs;
use std::path::PathBuf;

const PRESET: &str = include_str!("../../../data/presets/default.ron");
const MAP: &str = include_str!("../../../data/maps/default.ron");

fn read(rel: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    fs::read_to_string(dir.join(rel)).unwrap()
}

/// Rules, actions and names; no events.
fn data() -> Data {
    let mut data = bd_core::data::load(&read("rules.ron")).unwrap();
    data.add_actions(&read("actions.ron")).unwrap();
    data.add_names(&read("names.ron")).unwrap();
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
        married: false,
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
        assert_eq!(
            (v.loyalty, v.strength),
            (Fx::from_int(60), Fx::from_int(10))
        );
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
