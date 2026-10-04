//! Stage 26: the neighbours as kingdoms of their own, played by our rules beside us.

use bd_core::batch::{self, Files};
use bd_core::game::Game;
use bd_core::sim::{Chronicle, Dynasty, FallReason};
use bd_core::state::{Holder, NeighbourId, ProvinceId, World};
use bd_core::time::Tick;
use std::collections::BTreeMap;

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

/// Stage 26 acceptance, turned by stage 27: the kingdoms now reach us (their real strength
/// is what we face), and only through `realm:` of rules.ron. Without it a game with the
/// kingdoms in the preset plays as one without them, the text of `cli batch` alike.
#[test]
fn the_kingdoms_reach_us_only_through_the_realm_rules() {
    let bare = with_preset(without_realms);
    let mut deaf = files();
    let rules = &deaf["rules.ron"];
    let a = rules.find("\n    realm: (").unwrap();
    let b = a + rules[a..].find("\n    ),\n").unwrap() + 7;
    let cut = format!("{}{}", &rules[..a], &rules[b..]);
    deaf.insert("rules.ron".into(), cut);
    let batch = |f: &Files, strategy: &str| {
        let start = batch::load(f, PRESET, MAP, 0).unwrap();
        let rules = batch::score_rules(f, &start).unwrap();
        let auto = batch::chooser(f, &start, strategy).unwrap();
        batch::batch(&start, 0..8, &[], auto.as_ref(), &rules, |_| {})
            .unwrap()
            .0
    };
    for strategy in ["neutral", "warmonger"] {
        assert_eq!(batch(&deaf, strategy), batch(&bare, strategy), "{strategy}");
    }
    assert_ne!(batch(&files(), "neutral"), batch(&bare, "neutral"));
    let (g, c) = play(&deaf, 42);
    assert!(c.realms.is_empty() && g.realms.list.is_empty());
    let (g0, c0) = play(&bare, 42);
    assert_eq!(outcome(&g, &c), outcome(&g0, &c0));
}

/// Acceptance: a link made before the stage (seed 42, the script `test.ron`, then neutral to
/// the founder's death; 28 decisions) opens and gives the outcome it gave then, word for word.
/// Stage 26b: it opens to the same reign; the dynasty after it goes another way (the queue
/// of events by importance, one more action for the automaton), re-pinned. Stage 27: the same
/// reign again, the dynasty after it re-pinned: the neighbours' strength is their kingdoms'
/// own now, their wars and houses move it, and a usurper ends the dynasty in its 205th year.
/// Stage 26c: the compound events join the pool and the texts vary: the house dies out without
/// an heir in its 141st year (without the compound events it ends as in stage 27, only the
/// texts differ).
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
        (28, 141, "NoHeir", 45, 9399)
    );
    assert_eq!(format!("{hash:016x}"), "049160f01bb3f49c");
    assert_eq!(
        c.realms.len(),
        5,
        "the kingdoms play beside it, two of them new"
    );
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
    assert!(a.0.contains("house:\"") && b.1.contains("Аргириды"));
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
/// Нордмарк (an older king, another law) changes nothing of Пурпуляндия's stream, while
/// another seed gives Нордмарк another. Since stage 27 the kingdoms meet, so their stories,
/// and ours, part ways at the first year.
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
    let start = |f: &Files, seed: u64| batch::load(f, PRESET, MAP, 0).unwrap().reseeded(seed);
    let (g, g2, g3) = (start(&f, 3), start(&other, 3), start(&f, 4));
    let rng = |g: &Game, realm: &str| g.realms.list[&id(realm)].g.rng.clone();
    assert_eq!(rng(&g, "purpur"), rng(&g2, "purpur"));
    assert_eq!(rng(&g, "nordmark"), rng(&g2, "nordmark"));
    assert_ne!(rng(&g, "nordmark"), rng(&g3, "nordmark"));
    assert_ne!(
        g.realms.list[&id("nordmark")],
        g2.realms.list[&id("nordmark")]
    );
    let streams: std::collections::BTreeSet<_> = (g.realms.list.values())
        .map(|d| format!("{:?}", d.g.rng))
        .collect();
    assert_eq!(streams.len(), 3);
}

/// The one map as world `w` of `me` sees it: the owner of every province.
fn map_of(w: &World, me: &NeighbourId) -> BTreeMap<ProvinceId, NeighbourId> {
    (w.provinces.values())
        .map(|p| match &p.holder {
            Holder::Foreign(n) => (p.id.clone(), n.clone()),
            _ => (p.id.clone(), me.clone()),
        })
        .collect()
}

/// Every kingdom holds the map as we do (`Realms.owners`).
fn one_map(g: &Game) {
    let r = &g.realms;
    assert_eq!(map_of(&g.world, &r.us), r.owners);
    for (id, d) in &r.list {
        assert_eq!(map_of(&d.g.world, id), r.owners, "{id:?}");
    }
}

/// Plays `g` to the end of its year, the first choice of every event.
fn to_year_end(g: &mut Game) {
    let tpy = g.world.time_unit.ticks_per_year;
    let end = (g.world.tick.0 / tpy + 1) * tpy;
    while g.world.tick.0 < end {
        if g.wait()
            .is_ok_and(|s| matches!(s, bd_core::game::Step::Event(_)))
        {
            g.choose(0).unwrap();
        }
    }
}

/// The map is shared: land that changes hands with us changes in their worlds at the end
/// of the year, and what they see of each other follows ours; a kingdom's own story never
/// takes land from us nor gives it (stage 27: land between kingdoms it moves, see below).
/// What we see of a kingdom follows its world.
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
    // In its own story Веструм takes Гарт from us, Пурпуляндия loses Порфир to us.
    let mut set = |realm: &str, p: &str, h: Holder| {
        let w = &mut g.realms.list.get_mut(&id(realm)).unwrap().g.world;
        w.provinces.get_mut(&pid(p)).unwrap().holder = h;
    };
    set("vestrum", "gart", Holder::Crown);
    set("purpur", "porfir", us.clone());
    to_year_end(&mut g);
    assert_eq!(g.world.tick.0, tpy);
    one_map(&g);
    assert_eq!(holder(&g, "nordmark", "holm"), Holder::Crown);
    assert_eq!(holder(&g, "nordmark", "frostad"), us);
    assert_eq!(
        holder(&g, "purpur", "holm"),
        Holder::Foreign(id("nordmark"))
    );
    assert_eq!(holder(&g, "purpur", "frostad"), us);
    assert_eq!(holder(&g, "vestrum", "gart"), us);
    assert_eq!(holder(&g, "purpur", "porfir"), Holder::Crown);
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
        assert_eq!(
            batch::world_hash(&again),
            batch::world_hash(&g),
            "seed {seed}"
        );
        assert_eq!(again.rng, g.rng);
    }
}

/// Stage 27 calibration: the world of `seed` played from the start by the automaton, ours
/// too (`Dynasty::new`), for `years`: the game at the end.
fn world_of(f: &Files, seed: u64, years: u32) -> Dynasty {
    let g = batch::load(f, PRESET, MAP, 0).unwrap().reseeded(seed);
    let mut d = Dynasty::new(g);
    let tpy = d.g.world.time_unit.ticks_per_year;
    d.until(Tick(years * tpy));
    d
}

/// Stage 27 acceptance of the calibration (docs/calibration.md): the first dynasty of an
/// established kingdom lives 80 to 150 years at the median, over 100 worlds of 300 years (a
/// dynasty alive at the end counts 300). Stage 26c: over 1000 worlds. Purpur's dynasties
/// live to the horizon in a third of the worlds, and the median of a hundred jumps: seeds
/// 0..99 gave 110 before the compound events and 166 after them, 100..199 gave 138 and 162;
/// over 1000 worlds 137 and 140 (docs/calibration.md, stage 26c).
/// `cargo test --release -p core --test realms -- --ignored dynasties`.
#[test]
#[ignore = "release only, a minute and a half"]
fn kingdom_dynasties_live_80_to_150_years() {
    let f = files();
    let mut lives: std::collections::BTreeMap<String, Vec<u32>> = Default::default();
    let mut how: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let mut news: std::collections::BTreeMap<String, u32> = Default::default();
    let mut realms = vec![];
    let world = |seed| world_of(&f, seed, 300).g.realms;
    for r in batch::par_seeds(0..1000, batch::threads(), world, |_| {}) {
        for id in ["nordmark", "purpur", "vestrum"] {
            let first = r.falls.iter().find(|(x, ..)| x.0 == id);
            let years = first.map_or(300, |(_, t, _)| t.0);
            lives.entry(id.into()).or_default().push(years);
            let fall = first.map_or("Alive".into(), |(.., f)| format!("{f:?}"));
            how.entry(id.into()).or_default().push(fall);
        }
        for n in &r.news {
            *news.entry(n.title.clone()).or_default() += 1;
        }
        realms.push(r.list.len());
    }
    for (id, mut v) in lives.clone() {
        v.sort();
        let mut falls: std::collections::BTreeMap<&String, u32> = Default::default();
        for f in &how[&id] {
            *falls.entry(f).or_default() += 1;
        }
        let n = v.len();
        println!(
            "{id}: {} / {} / {} {falls:?}",
            v[n / 4],
            v[n / 2],
            v[n * 3 / 4]
        );
    }
    realms.sort();
    let n = realms.len();
    println!(
        "kingdoms at the end: {} / {} / {}",
        realms[n / 4],
        realms[n / 2],
        realms[n * 3 / 4]
    );
    println!("news per world: {news:?}");
    for (id, mut v) in lives {
        v.sort();
        let median = v[v.len() / 2];
        assert!((80..=150).contains(&median), "{id}: median {median}");
    }
}

/// Applies `effect` (RON) in the world of kingdom `realm`.
fn apply(g: &mut Game, realm: &str, effect: &str) {
    let e: bd_core::rules::Effect = ron::from_str(effect).unwrap();
    let d = g.realms.list.get_mut(&id(realm)).unwrap();
    let mut queue = vec![];
    let mut ctx = bd_core::rules::Ctx {
        data: &d.g.data,
        queue: &mut queue,
        target: None,
        neighbour: None,
    };
    e.apply(&mut d.g.world, &mut ctx);
}

/// The titles of the news of a kind (`rules.ron` `realm.news`) told so far.
fn news<'a>(
    g: &'a Game,
    kind: impl Fn(&bd_core::data::NewsRules) -> &bd_core::data::NewsKind,
) -> Vec<&'a str> {
    let title = &kind(&g.data.realm.as_ref().unwrap().news).title;
    (g.realms.news.iter())
        .filter(|n| n.title == *title)
        .map(|n| n.text.as_str())
        .collect()
}

/// Stage 27 acceptance: two kingdoms with a common border go to war by the automaton and the
/// war chain, the winner takes a province on that border, every kingdom holds the new map,
/// and the crown hears of it.
#[test]
fn kingdoms_at_war_take_border_land_and_the_map_follows() {
    let f = files();
    let found = (0..40).find_map(|seed| {
        let mut d = Dynasty::new(batch::load(&f, PRESET, MAP, 0).unwrap().reseeded(seed));
        let tpy = d.g.world.time_unit.ticks_per_year;
        for year in 1..=150 {
            let before = d.g.realms.clone();
            d.until(Tick(year * tpy));
            let r = &d.g.realms;
            let taken = (r.owners.iter()).find(|(p, o)| {
                let was = &before.owners[*p];
                was != *o && before.list.contains_key(was) && before.list.contains_key(*o)
            });
            if let Some((p, o)) = taken.map(|(p, o)| (p.clone(), o.clone())) {
                return Some((d, before, p, o));
            }
            if d.fall.is_some() {
                return None;
            }
        }
        None
    });
    let (d, before, p, winner) = found.expect("a war between kingdoms in 40 worlds");
    let g = &d.g;
    let loser = &before.owners[&p];
    // A war between the two was on in one of their worlds.
    let at_war = |a: &NeighbourId, b: &NeighbourId| {
        before.list[a]
            .g
            .world
            .war
            .as_ref()
            .is_some_and(|w| w.enemy == *b)
    };
    assert!(at_war(&winner, loser) || at_war(loser, &winner));
    // The province lay on the winner's border.
    let w = &before.list[&winner].g.world;
    let border = w.provinces[&p]
        .neighbours
        .iter()
        .any(|q| before.owners[q] == winner);
    assert!(border, "{p:?}");
    one_map(g);
    assert_eq!(
        g.realms.list[&winner].g.world.provinces[&p].holder,
        Holder::Crown
    );
    let name = &g.world.provinces[&p].name;
    assert!(
        news(g, |n| &n.capture)
            .iter()
            .any(|t| t.contains(name.as_str())),
        "{name}"
    );
}

/// Stage 27 acceptance: a vassal of a foreign kingdom that breaks away (`Effect::Secede`)
/// founds a kingdom of his own: his house on the throne, a ruler and heirs of the profile
/// (`realms.founded`), a grudge against his old lord; every world holds it, ours too.
#[test]
fn a_foreign_vassal_that_breaks_away_founds_a_kingdom() {
    let mut g = batch::load(&files(), PRESET, MAP, 0).unwrap().reseeded(5);
    apply(&mut g, "purpur", r#"Secede(ById("kirm"))"#);
    to_year_end(&mut g);
    let melissin = id("melissin");
    let d = &g.realms.list[&melissin];
    let w = &d.g.world;
    assert_eq!(d.house, "Мелиссин");
    assert!(d.fall.is_none() && !w.ruler.name.is_empty());
    assert_eq!(w.heirs.len(), 2);
    assert_eq!(w.capital.province, ProvinceId("kirm".into()));
    assert_eq!(w.provinces[&w.capital.province].holder, Holder::Crown);
    assert_eq!(
        w.axes[&bd_core::state::AxisId("army".into())],
        bd_core::fx::Fx::from_int(70)
    );
    assert!(w.neighbours[&id("purpur")].relation < bd_core::fx::Fx(0));
    one_map(&g);
    let v = g.world.neighbours[&melissin].realm.as_ref().unwrap();
    assert_eq!(
        (v.house.as_str(), v.ruler.as_str()),
        ("Мелиссин", w.ruler.name.as_str())
    );
    assert!(
        g.realms.list[&id("nordmark")]
            .g
            .world
            .neighbours
            .contains_key(&melissin)
    );
    assert_eq!(news(&g, |n| &n.breakaway).len(), 1);
}

/// Stage 27 acceptance: a kingdom with no crown land left is no more; its land goes by
/// `realm.dissolution`: to the state that took its capital (`Conqueror`), or to a state of
/// each of its vassals (`Vassals`). Here Нордмарк has taken all Пурпуляндия's crown land.
#[test]
fn a_kingdom_without_land_is_no_more_and_its_land_goes_by_rule() {
    let lost = |f: &Files| {
        let mut g = batch::load(f, PRESET, MAP, 0).unwrap().reseeded(5);
        let purpur = g.realms.list.get_mut(&id("purpur")).unwrap();
        for p in purpur.g.world.provinces.values_mut() {
            if p.holder == Holder::Crown {
                p.holder = Holder::Foreign(id("nordmark"));
            }
        }
        to_year_end(&mut g);
        assert!(!g.realms.list.contains_key(&id("purpur")));
        let falls: Vec<_> = g
            .realms
            .falls
            .iter()
            .map(|(id, _, f)| (id.0.as_str(), f))
            .collect();
        assert_eq!(falls, [("purpur", &FallReason::Conquered)]);
        let told = news(&g, |n| &n.fallen);
        assert!(
            told.len() == 1 && told[0].contains("Пурпуляндия"),
            "{told:?}"
        );
        one_map(&g);
        g
    };
    let kirm = ProvinceId("kirm".into());
    let g = lost(&files());
    assert_eq!(g.realms.owners[&kirm], id("nordmark"));
    assert!(!g.realms.owners.values().any(|o| *o == id("purpur")));
    let mut f = files();
    let rules = f["rules.ron"].replacen("dissolution: Conqueror", "dissolution: Vassals", 1);
    f.insert("rules.ron".into(), rules);
    let g = lost(&f);
    assert_eq!(g.realms.owners[&kirm], id("melissin"));
    assert_eq!(g.realms.list[&id("melissin")].house, "Мелиссин");
    assert_eq!(
        g.realms.owners[&ProvinceId("amaran".into())],
        id("nordmark")
    );
}

/// Stage 27 acceptance: a kingdom whose dynasty ends keeps its throne under a new house, as a
/// usurper takes ours: a ruler and heirs of the profile, the legitimacy of `realm.usurper`;
/// the crown hears of it and sees the new house.
#[test]
fn a_fallen_dynasty_leaves_its_throne_to_a_new_house() {
    let mut g = batch::load(&files(), PRESET, MAP, 0).unwrap().reseeded(5);
    let nordmark = g.realms.list.get_mut(&id("nordmark")).unwrap();
    nordmark.g.world.heirs.clear();
    nordmark.g.ended = Some("old_age".into());
    to_year_end(&mut g);
    assert_eq!(
        g.realms.falls,
        [(id("nordmark"), Tick(0), FallReason::NoHeir)]
    );
    let d = &g.realms.list[&id("nordmark")];
    assert!(d.fall.is_none());
    assert_ne!(d.house, "Эрлинги");
    assert!(g.data.names.houses.contains(&d.house));
    assert_ne!(d.g.world.ruler.name, "Харальд");
    // Its chronicle goes on: the old house's ruler, then the new one.
    let rulers: Vec<_> = d.c.rulers.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(rulers, ["Харальд", d.g.world.ruler.name.as_str()]);
    assert_eq!(d.g.world.heirs.len(), 2);
    let legitimacy = d.g.world.axes[&bd_core::state::AxisId("legitimacy".into())];
    assert_eq!(legitimacy, bd_core::fx::Fx::from_int(30));
    let v = g.world.neighbours[&id("nordmark")].realm.as_ref().unwrap();
    assert_eq!(v.house, d.house);
    let told = news(&g, |n| &n.house);
    assert_eq!(told.len(), 1);
    assert!(told[0].contains("Эрлинг"), "{}", told[0]);
}

/// Stage 27: the preset names relations with other kingdoms only, and the profile of a new
/// kingdom only axes the rules know.
#[test]
fn the_preset_of_the_kingdoms_is_checked() {
    let bad = [
        with_preset(|p| {
            p.replacen(
                r#"relations: {"nordmark": -25}"#,
                r#"relations: {"purpur": -25}"#,
                1,
            )
        }),
        with_preset(|p| {
            p.replacen(
                r#"relations: {"nordmark": -25}"#,
                r#"relations: {"kingdom": -25}"#,
                1,
            )
        }),
        with_preset(|p| p.replacen(r#"axes: {"army": 70,"#, r#"axes: {"armee": 70,"#, 1)),
    ];
    for (f, want) in bad.iter().zip([
        "relation with purpur",
        "relation with kingdom",
        "founded: unknown axis armee",
    ]) {
        let e = batch::load(f, PRESET, MAP, 0).unwrap_err().join("\n");
        assert!(e.contains(want), "{e}");
    }
}
