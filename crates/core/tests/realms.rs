//! Stage 26: the neighbours as kingdoms of their own, played by our rules beside us.

use bd_core::batch::{self, Files};
use bd_core::game::Game;
use bd_core::sim::{Chronicle, Dynasty};
use bd_core::state::{Holder, NeighbourId, ProvinceId};
use bd_core::time::Tick;

const PRESET: &str = "presets/default.ron";
const MAP: &str = "maps/default.ron";

/// Every `.ron` of `data/` as `cli` reads them.
fn files() -> Files {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/");
    let mut out = Files::new();
    for dir in ["", "events/", "events/sim/", "presets/", "maps/"] {
        for e in std::fs::read_dir(format!("{root}{dir}")).unwrap() {
            let p = e.unwrap().path();
            if p.is_file() && p.extension().is_some_and(|x| x == "ron") {
                let name = p.file_name().unwrap().to_string_lossy();
                out.insert(format!("{dir}{name}"), std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    out
}

/// The default preset with `edit` applied to its text.
fn with_preset(edit: impl Fn(&str) -> String) -> Files {
    let mut f = files();
    let p = edit(&f[PRESET]);
    assert_ne!(p, f[PRESET], "the edit changed nothing");
    f.insert(PRESET.into(), p);
    f
}

/// The preset without `realms`: the neighbours as numbers only, as before the stage.
fn without_realms(p: &str) -> String {
    let (a, b) = (
        p.find("    realms: Some((").unwrap(),
        p.find("    intro:").unwrap(),
    );
    format!("{}{}", &p[..a], &p[b..])
}

fn id(s: &str) -> NeighbourId {
    NeighbourId(s.into())
}

/// A neutral reign of `seed` and the dynasty after it.
fn play(f: &Files, seed: u64) -> (Game, Chronicle) {
    let mut g = batch::load(f, PRESET, MAP, 0).unwrap().reseeded(seed);
    batch::play(&mut g, None, &mut vec![]).unwrap();
    let rules = batch::score_rules(f, &g).unwrap();
    let c = batch::dynasty(&g, &rules).0.unwrap();
    (g, c)
}

/// What the player gets of a game: the decisions, the chronicle told, the rulers, the end.
fn outcome(g: &Game, c: &Chronicle) -> String {
    let rulers: Vec<_> = c.rulers.iter().map(|r| (&r.name, &r.biography)).collect();
    format!(
        "{:?}\n{}\n{rulers:?}\n{:?} {} {:?}",
        g.decisions,
        batch::chronicle_text(g, c),
        c.fall,
        c.years,
        c.axes
    )
}

/// Acceptance: the kingdoms change nothing of ours. `cli batch` (the golden tests of seed 42
/// and the scripts stay pinned in `sim.rs` and `cli.rs`) gives the same text with them as
/// without, for a neutral and an active strategy.
#[test]
fn the_kingdoms_change_nothing_of_ours() {
    let (f, bare) = (files(), with_preset(without_realms));
    for strategy in ["neutral", "warmonger"] {
        let batch = |f: &Files| {
            let start = batch::load(f, PRESET, MAP, 0).unwrap();
            let rules = batch::score_rules(f, &start).unwrap();
            let auto = batch::chooser(f, &start, strategy).unwrap();
            batch::batch(&start, 0..8, &[], auto.as_ref(), &rules, |_| {})
                .unwrap()
                .0
        };
        assert_eq!(batch(&f), batch(&bare), "{strategy}");
    }
    let (g, c) = play(&f, 42);
    assert_eq!(c.realms.len(), 3);
    let (g0, c0) = play(&bare, 42);
    assert!(c0.realms.is_empty());
    assert_eq!(outcome(&g, &c), outcome(&g0, &c0));
}

/// Acceptance: a link made before the stage (seed 42, the script `test.ron`, then neutral to
/// the founder's death; 28 decisions) opens and gives the outcome it gave then, word for word.
/// Stage 26b: it opens to the same reign; the dynasty after it goes another way (the queue
/// of events by importance, one more action for the automaton), re-pinned.
#[test]
fn a_link_from_before_the_stage_plays_the_same() {
    let link = "AQdkZWZhdWx0Kh0AASMAAQABAQABAQABAQABAQABAQAAAQABAgABAQAAAQABAQABAQABAgABAQABAQABAQABAQ\
                ABAQAAAQAAAQAAAQABAQABAQABAQAAAQABAQABAQAB";
    let f = files();
    let l = bd_core::link::decode(link).unwrap();
    assert_eq!((l.preset_id.as_str(), l.seed), ("default", 42));
    let mut g = batch::load(&f, PRESET, MAP, l.seed).unwrap();
    l.play(&mut g).unwrap();
    let rules = batch::score_rules(&f, &g).unwrap();
    let (c, s) = batch::dynasty(&g, &rules);
    let (c, s) = (c.unwrap(), s.unwrap());
    let text = batch::chronicle_text(&g, &c);
    let hash = (text.bytes()).fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    let fall = format!("{:?}", c.fall);
    assert_eq!(
        (
            g.decisions.len(),
            c.years,
            fall.as_str(),
            c.entries.len(),
            s.total
        ),
        (28, 300, "Alive", 83, 25737)
    );
    assert_eq!(format!("{hash:016x}"), "3562134f6dad11eb");
    assert_eq!(c.realms.len(), 3, "the kingdoms play beside it");
}

/// Acceptance: the same seed gives the whole world byte for byte, ours and every kingdom's,
/// at the end of the reign and in the chronicles after it.
#[test]
fn the_whole_world_repeats_byte_for_byte() {
    let f = files();
    let dump = || {
        let (g, c) = play(&f, 7);
        let realms: Vec<_> = (g.realms.list.iter())
            .map(|(id, d)| (id, &d.g.world, &d.g.rng, &d.g.queue, &d.c))
            .collect();
        let world = ron::to_string(&(&g.world, &g.rng, realms)).unwrap();
        (world, ron::to_string(&c).unwrap())
    };
    let (a, b) = (dump(), dump());
    assert!(a.0.contains("Эрлинги") && b.1.contains("Аргириды"));
    assert!(a == b);
}

/// The kingdom of `realm` alone, as it starts in the game of `seed`.
fn kingdom(f: &Files, seed: u64, realm: &str) -> Dynasty {
    let g = batch::load(f, PRESET, MAP, 0).unwrap().reseeded(seed);
    g.realms.list[&id(realm)].clone()
}

/// Who is crowned when the founding ruler of `d` dies now.
fn heir_crowned(mut d: Dynasty) -> String {
    d.g.ended = Some("old_age".into());
    d.until(Tick(1));
    d.c.rulers[1].name.clone()
}

/// Acceptance: a kingdom lives a hundred years by our rules: its rulers change by its law of
/// succession, its axes move, its laws change, its chronicle is kept.
#[test]
fn a_kingdom_lives_a_hundred_years_by_its_own_law() {
    let f = files();
    // Веструм elects the ablest, Матильда (65) over her elder brother Арнульф (50); under
    // primogeniture he goes first. Пурпуляндия's queen leaves the throne to her son, Нордмарк's
    // king to his, the eldest of the house.
    let primogeniture = with_preset(|p| p.replacen("\"law_elective\"", "\"law_primogeniture\"", 1));
    for seed in 0..3 {
        assert_eq!(heir_crowned(kingdom(&f, seed, "vestrum")), "Матильда");
        assert_eq!(
            heir_crowned(kingdom(&primogeniture, seed, "vestrum")),
            "Арнульф"
        );
        assert_eq!(heir_crowned(kingdom(&f, seed, "purpur")), "Иоанн");
        assert_eq!(heir_crowned(kingdom(&f, seed, "nordmark")), "Сигурд");
    }
    let lived = (0..20)
        .map(|seed| kingdom(&f, seed, "purpur"))
        .find_map(|mut d| {
            let start = d.g.world.clone();
            d.until(Tick(100 * start.time_unit.ticks_per_year));
            d.fall.is_none().then_some((start, d))
        });
    let (start, d) = lived.expect("one of twenty lives a hundred years");
    let w = &d.g.world;
    assert_eq!(w.tick.year(w.time_unit), 100);
    assert!(d.c.rulers.len() >= 2, "{:?}", d.c.rulers);
    assert_ne!(w.ruler, start.ruler);
    let moved = (start.axes.iter())
        .filter(|(a, v)| w.axes[*a] != **v)
        .count();
    assert!(
        moved * 2 > start.axes.len(),
        "{moved} axes of {}",
        start.axes.len()
    );
    let laws = |w: &bd_core::state::World| {
        let all = d.g.data.laws_in_force(w).map(|l| l.id.clone());
        all.collect::<Vec<_>>()
    };
    assert_ne!(laws(w), laws(&start));
    assert!(d.c.entries.len() > 10);
}

/// Acceptance: every kingdom has a stream of its own from the seed and its id: another
/// Нордмарк (an older king, another law) changes nothing of ours nor of Пурпуляндия, while
/// another seed gives Нордмарк another history.
#[test]
fn the_streams_of_the_kingdoms_are_apart() {
    let f = files();
    let other = with_preset(|p| {
        let p = p.replacen(
            "ruler: (name: \"Харальд\", age: 48",
            "ruler: (name: \"Харальд\", age: 61",
            1,
        );
        p.replacen(
            r#"flags: ["law_seniority", "married"]"#,
            r#"flags: ["law_salic", "married"]"#,
            1,
        )
    });
    let (g, c) = play(&f, 3);
    let (g2, c2) = play(&other, 3);
    assert_eq!(outcome(&g, &c), outcome(&g2, &c2));
    assert_eq!(c.realms[&id("purpur")], c2.realms[&id("purpur")]);
    assert_ne!(c.realms[&id("nordmark")], c2.realms[&id("nordmark")]);
    let (_, c3) = play(&f, 4);
    assert_ne!(c.realms[&id("nordmark")], c3.realms[&id("nordmark")]);
    let streams: std::collections::BTreeSet<_> = (g.realms.list.values())
        .map(|d| format!("{:?}", d.g.rng))
        .collect();
    assert_eq!(streams.len(), 3);
}

/// The map is shared: land that changes hands with us changes in their worlds at the end
/// of the year, and what they see of each other follows ours; a kingdom's own story never
/// takes land from us. What we see of a kingdom follows its world.
#[test]
fn the_kingdoms_share_our_map_and_show_their_rulers() {
    let mut g = batch::load(&files(), PRESET, MAP, 0).unwrap().reseeded(11);
    let tpy = g.world.time_unit.ticks_per_year;
    let pid = |s: &str| ProvinceId(s.into());
    let holder = |g: &Game, realm: &str, p: &str| {
        g.realms.list[&id(realm)].g.world.provinces[&pid(p)]
            .holder
            .clone()
    };
    let us = Holder::Foreign(id("kingdom"));
    assert_eq!(holder(&g, "nordmark", "holm"), us);
    assert_eq!(holder(&g, "nordmark", "nordheim"), Holder::Crown);
    assert_eq!(
        holder(&g, "purpur", "kirm"),
        Holder::Vassal(bd_core::state::VassalId("melissin".into()))
    );
    assert_eq!(
        holder(&g, "purpur", "nordheim"),
        Holder::Foreign(id("nordmark"))
    );
    // Нордмарк takes Хольм from us, we take Фростад from it.
    g.world.provinces.get_mut(&pid("holm")).unwrap().holder = Holder::Foreign(id("nordmark"));
    g.world.provinces.get_mut(&pid("frostad")).unwrap().holder = Holder::Crown;
    // In its own story Веструм takes Гарт from us, Пурпуляндия loses Порфир to us and Кирм
    // to its vassal Мелиссин, who breaks away.
    let mut set = |realm: &str, p: &str, h: Holder| {
        let w = &mut g.realms.list.get_mut(&id(realm)).unwrap().g.world;
        w.provinces.get_mut(&pid(p)).unwrap().holder = h;
    };
    set("vestrum", "gart", Holder::Crown);
    set("purpur", "porfir", us.clone());
    set("purpur", "kirm", Holder::Foreign(id("melissin")));
    while g.world.tick.0 < tpy {
        if g.wait()
            .is_ok_and(|s| matches!(s, bd_core::game::Step::Event(_)))
        {
            g.choose(0).unwrap();
        }
    }
    assert_eq!(holder(&g, "nordmark", "holm"), Holder::Crown);
    assert_eq!(holder(&g, "nordmark", "frostad"), us);
    assert_eq!(
        holder(&g, "purpur", "holm"),
        Holder::Foreign(id("nordmark"))
    );
    assert_eq!(holder(&g, "purpur", "frostad"), us);
    assert_eq!(holder(&g, "vestrum", "gart"), us);
    assert_eq!(holder(&g, "purpur", "porfir"), Holder::Crown);
    assert_eq!(
        holder(&g, "purpur", "kirm"),
        Holder::Foreign(id("melissin"))
    );
    assert_eq!(
        g.world.provinces[&pid("kirm")].holder,
        Holder::Foreign(id("purpur"))
    );
    for (rid, d) in &g.realms.list {
        let v = g.world.neighbours[rid].realm.as_ref().unwrap();
        let w = &d.g.world;
        assert_eq!(v.ruler, w.ruler.name);
        assert_eq!(Some(&v.law), d.g.data.heirs.law(w).map(|l| &l.flag));
        assert_eq!(
            v.stability,
            w.axes[&bd_core::state::AxisId("stability".into())]
        );
        assert_eq!(v.fallen, d.fall.is_some());
    }
    let houses: Vec<_> = (g.world.neighbours.values())
        .map(|n| n.realm.as_ref().unwrap().house.as_str())
        .collect();
    assert_eq!(houses, ["Эрлинги", "Аргириды", "Вестинги"]);
}

/// Stage 26b: a link to a game the automaton played (`warmonger`) opens to the same world:
/// its noise comes from a stream of its own, the game's rng goes to the game alone.
#[test]
fn a_link_of_an_automaton_game_plays_the_same() {
    let f = files();
    for seed in [3, 42] {
        let start = batch::load(&f, PRESET, MAP, 0).unwrap().reseeded(seed);
        let auto = batch::chooser(&f, &start, "warmonger").unwrap();
        let mut g = start.clone();
        batch::play(&mut g, auto.as_ref(), &mut vec![]).unwrap();
        assert!(g.decisions.len() > 10 && g.ended.is_some());
        let l = bd_core::link::decode(&bd_core::link::encode("default", seed, &g)).unwrap();
        let mut again = start.clone();
        l.play(&mut again).unwrap();
        assert_eq!(batch::world_hash(&again), batch::world_hash(&g), "seed {seed}");
        assert_eq!(again.rng, g.rng);
    }
}
