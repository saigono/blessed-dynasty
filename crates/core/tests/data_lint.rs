//! Content lint: loads every data file and checks references across files.

use bd_core::data::{self, Data, DataError};
use bd_core::game::{Game, Step};
use bd_core::rules::Effect;
use bd_core::sim;
use bd_core::state::Preset;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

fn path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(path(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// `*.ron` files of a data directory, without subdirectories, sorted so the load order
/// is fixed (as `cli` loads them).
fn files(rel: &str) -> Vec<PathBuf> {
    let entries = fs::read_dir(path(rel)).unwrap();
    let mut files: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
    files.retain(|p| p.is_file() && p.extension().is_some_and(|x| x == "ron"));
    files.sort();
    files
}

fn rules() -> Data {
    data::load(&read("rules.ron")).unwrap()
}

/// `rules.ron`, every `events/*.ron`, `actions.ron` and `names.ron` in one `Data`, as the
/// game sees them, plus the simulation's `events/sim/*.ron` in `sim_events`.
fn load_all() -> Data {
    let mut data = rules();
    for f in files("events") {
        let text = fs::read_to_string(&f).unwrap();
        data.add_events(&text)
            .unwrap_or_else(|e| panic!("{f:?}: {e:?}"));
    }
    for f in files("events/sim") {
        let text = fs::read_to_string(&f).unwrap();
        data.add_sim_events(&text)
            .unwrap_or_else(|e| panic!("{f:?}: {e:?}"));
    }
    data.add_actions(&read("actions.ron")).unwrap();
    data.add_names(&read("names.ron")).unwrap();
    data
}

fn preset(text: &str, data: &Data) -> Preset {
    Preset::load_with_map(text, &read("maps/default.ron"), data).unwrap()
}

fn hints() -> BTreeMap<String, String> {
    ron::from_str(&read("hints.ron")).unwrap()
}

fn has_digits(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit())
}

#[test]
fn all_files_load() {
    let data = load_all();
    for f in files("presets") {
        let text = fs::read_to_string(&f).unwrap();
        Preset::load_with_map(&text, &read("maps/default.ron"), &data)
            .unwrap_or_else(|e| panic!("{f:?}: {e:?}"));
    }
    assert!(!hints().is_empty());
}

/// Simulation events share the id space of the reign: sim::run puts them in one pool.
#[test]
fn ids_are_unique() {
    let data = load_all();
    let all = data.events.iter().chain(&data.sim_events);
    let events: BTreeSet<_> = all.map(|e| &e.id).collect();
    let actions: BTreeSet<_> = data.actions.iter().map(|a| &a.id).collect();
    assert_eq!(events.len(), data.events.len() + data.sim_events.len());
    assert_eq!(actions.len(), data.actions.len());
}

/// Every cause_tag of events (simulation ones included) and actions has a hint, and every
/// hint has a tag. A hint is told as a sentence of its own (sim.rs adds the capital and the
/// full stop): lowercase start, no final punctuation, no numbers.
#[test]
fn every_cause_tag_has_a_hint() {
    let data = load_all();
    let hints = hints();
    let events = data.events.iter().chain(&data.sim_events);
    let tags = events.flat_map(|e| &e.choices).map(|c| &c.cause_tag);
    let tags: BTreeSet<_> = tags
        .chain(data.actions.iter().map(|a| &a.cause_tag))
        .collect();
    let missing: BTreeSet<_> = tags.iter().filter(|t| !hints.contains_key(**t)).collect();
    assert!(missing.is_empty(), "no hint for {missing:?}");
    let unused: Vec<_> = hints.keys().filter(|k| !tags.contains(k)).collect();
    assert!(unused.is_empty(), "hints of no cause_tag: {unused:?}");
    let bad: Vec<_> = (hints.values())
        .filter(|h| {
            let first = h.chars().next().is_some_and(char::is_lowercase);
            !first || h.ends_with(['.', '!', '?', ',', ' ']) || has_digits(h)
        })
        .collect();
    assert!(bad.is_empty(), "hints out of the chronicle format: {bad:?}");
}

#[test]
fn every_spawned_event_exists() {
    let data = load_all();
    let all = || data.events.iter().chain(&data.sim_events);
    let ids: BTreeSet<_> = all().map(|e| e.id.as_str()).collect();
    let choices = all().flat_map(|e| &e.choices);
    let effects = choices.flat_map(|c| &c.effects);
    let effects = effects.chain(data.actions.iter().flat_map(|a| &a.on_complete));
    let missing: Vec<_> = effects
        .filter_map(|e| match e {
            Effect::SpawnEvent(id, _) if !ids.contains(id.as_str()) => Some(id),
            _ => None,
        })
        .collect();
    assert!(missing.is_empty(), "spawned but undefined: {missing:?}");
}

/// Events `rules.ron` names: the war start, neighbour AI events (simulation ones included),
/// death and abdication.
#[test]
fn every_event_named_by_the_rules_exists() {
    let data = load_all();
    let ai = &data.neighbour_ai;
    let stances = [&ai.expand, &ai.defend, &ai.trade, &ai.wait];
    let named = (stances.iter().flat_map(|s| &s.events)).map(|(id, _)| id);
    let death = data.death.risks.iter().map(|r| &r.event);
    let fixed = [
        &data.war.start_event,
        &data.death.event,
        &data.abdication.event,
    ];
    let all = data.events.iter().chain(&data.sim_events);
    let missing: BTreeSet<_> = (named.chain(death).chain(fixed))
        .filter(|id| !all.clone().any(|e| e.id == **id))
        .collect();
    assert!(missing.is_empty(), "named but undefined: {missing:?}");
}

/// The brief of stage 9: 30 events in the pool, 2-3 choices each, every choice hinted
/// without numbers. Chain steps (weight 0) follow the same shape.
#[test]
fn reign_events_follow_the_brief() {
    let mut data = rules();
    data.add_events(&read("events/reign.ron")).unwrap();
    let pool = data.events.iter().filter(|e| e.weight > 0).count();
    assert_eq!(pool, 30);
    for e in &data.events {
        assert!((2..=3).contains(&e.choices.len()), "{}", e.id);
        for c in &e.choices {
            let hint = c
                .hint
                .as_deref()
                .unwrap_or_else(|| panic!("{}: no hint", e.id));
            assert!(!has_digits(hint), "{}: {hint}", e.id);
        }
    }
}

/// Every choice the player can see carries a hint without numbers (reign.ron style).
#[test]
fn every_reign_choice_is_hinted() {
    let data = load_all();
    for e in &data.events {
        for c in &e.choices {
            let hint = c.hint.as_deref();
            assert!(hint.is_some_and(|h| !has_digits(h)), "{}: {hint:?}", e.id);
        }
    }
}

/// The brief of stage 9: twelve simulation events, 2-3 choices each, every one important
/// enough for the chronicle.
#[test]
fn sim_events_follow_the_brief() {
    let data = load_all();
    assert_eq!(data.sim_events.len(), 14);
    for e in &data.sim_events {
        assert!((2..=3).contains(&e.choices.len()), "{}", e.id);
        assert!(e.importance >= data.sim.threshold, "{}", e.id);
    }
}

/// Stage 12: a lost war costs the nobles' loyalty: every peace of `war_defeat` and a lost
/// battle with the steppe (`nomad_raid`, the failed branch).
#[test]
fn lost_wars_cost_the_nobles() {
    let data = load_all();
    let nobles = |es: &[Effect]| {
        es.iter()
            .any(|e| matches!(e, Effect::Axis(a, d) if a.0 == "loyalty_nobles" && d.0 <= -10_000))
    };
    let event = |id: &str| {
        let all = data.events.iter().chain(&data.sim_events);
        all.into_iter().find(|e| e.id == id).unwrap().clone()
    };
    for c in &event("war_defeat").choices {
        assert!(nobles(&c.effects), "{}", c.cause_tag);
    }
    let fought = &event("nomad_raid").choices[0].effects;
    let lost = fought.iter().find_map(|e| match e {
        Effect::Chance(c) => Some(&c.otherwise),
        _ => None,
    });
    assert!(nobles(lost.unwrap()));
}

#[test]
fn name_pools() {
    let names = load_all().names;
    for list in [&names.rulers, &names.heirs, &names.vassals] {
        assert_eq!(list.len(), 30);
    }
    // A name twice in a pool is rejected.
    let dup = r#"(rulers: ["Ульрих", "Ульрих"], heirs: [], vassals: [])"#;
    assert!(matches!(rules().add_names(dup), Err(DataError::Invalid(_))));
}

/// Stand-in for `cli batch --strategy neutral` (stage 8b): 1000 seeds, each reign to its end
/// (at most 60 years), the middle choice `(len - 1) / 2` as in `cli`, no actions. Every
/// reign event must fire at least once.
#[test]
fn every_reign_event_fires_under_neutral_play() {
    let data = load_all();
    let preset = preset(&read("presets/default.ron"), &data);
    let mut fired = BTreeSet::new();
    for seed in 0..1000 {
        let mut g = Game::new(data.clone(), &preset, seed);
        for _ in 0..60 {
            match g.wait().unwrap() {
                Step::Event(v) => {
                    g.choose((v.choices.len() - 1) / 2).unwrap();
                    fired.insert(v.event_id);
                }
                Step::Idle => {}
                Step::ReignEnded(_) => break,
            }
        }
    }
    let mut reign = rules();
    reign.add_events(&read("events/reign.ron")).unwrap();
    let silent: Vec<_> = (reign.events.iter())
        .filter(|e| !fired.contains(&e.id))
        .map(|e| &e.id)
        .collect();
    assert!(silent.is_empty(), "never fired: {silent:?}");
}

/// The same for the simulation: 1000 dynasties, each after a neutral reign of its seed; every
/// simulation event reaches the chronicle at least once.
#[test]
#[ignore = "about a minute in release; stage 9 acceptance, run with --release --ignored"]
fn every_sim_event_fires_in_a_thousand_dynasties() {
    let data = load_all();
    let preset = preset(&read("presets/default.ron"), &data);
    let mut fired: BTreeMap<String, u32> = BTreeMap::new();
    for seed in 0..1000 {
        let mut g = Game::new(data.clone(), &preset, seed);
        let end = loop {
            match g.wait().unwrap() {
                Step::Event(v) => g.choose((v.choices.len() - 1) / 2).unwrap(),
                Step::Idle => {}
                Step::ReignEnded(end) => break end,
            }
        };
        for e in sim::run(end, &data, g.rng.clone()).entries {
            *fired.entry(e.event.unwrap_or_default()).or_default() += 1;
        }
    }
    let silent: Vec<_> = (data.sim_events.iter())
        .filter(|e| !fired.contains_key(&e.id))
        .map(|e| &e.id)
        .collect();
    assert!(
        silent.is_empty(),
        "never fired: {silent:?}; fired: {fired:?}"
    );
    eprintln!("{fired:?}");
}

/// What the player reads instead of ids: a name for every axis, a text for every way a
/// reign ends (a `RulerDies` cause, nested ones too, or abdication) and every fall.
#[test]
fn reign_ends_falls_and_axes_have_display_texts() {
    fn deaths<'a>(effects: &'a [Effect], out: &mut BTreeSet<&'a str>) {
        for e in effects {
            match e {
                Effect::RulerDies(cause) => {
                    out.insert(cause);
                }
                Effect::Chance(c) => {
                    deaths(&c.then, out);
                    deaths(&c.otherwise, out);
                }
                Effect::IfFriendly(es) => deaths(es, out),
                _ => {}
            }
        }
    }
    let data = load_all();
    let nameless: Vec<_> = data.axes.iter().filter(|a| a.name.is_empty()).collect();
    assert!(nameless.is_empty(), "{nameless:?}");
    let mut causes = BTreeSet::from([data.abdication.event.as_str()]);
    let all = data.events.iter().chain(&data.sim_events);
    for c in all.flat_map(|e| &e.choices) {
        deaths(&c.effects, &mut causes);
    }
    let texts = &data.sim.texts;
    let missing: Vec<_> = (causes.iter())
        .filter(|c| !texts.reign_ends.contains_key(**c))
        .collect();
    assert!(missing.is_empty(), "no text for {missing:?}");
    assert!(causes.len() >= 4, "{causes:?}");
    use sim::FallReason::*;
    for f in [NoHeir, CapitalLost, Usurped, NoCrownLand, Alive] {
        assert!(texts.falls.iter().any(|(r, _)| *r == f), "{f:?}");
    }
}

/// Stage 18: the loops a law could push into a runaway. A loop's gain is the product of its
/// edges' k; an edge with a curve is flat beyond its points, so outside the curves it adds 0.
/// Stage 19: each edge at the largest multiplier a law puts on it (at least 1).
/// Loops whose gain there is 1.5 or more, with it.
fn runaway_loops(data: &Data) -> Vec<(String, bd_core::fx::Fx)> {
    use bd_core::fx::Fx;
    let edge = |id: &String| data.influences.iter().find(|e| e.id == *id).unwrap();
    let most = |e: &bd_core::graph::Influence| {
        let named = data.laws.list.iter().flat_map(|l| &l.edges);
        let ks = named.filter(|(id, _)| *id == e.id).map(|(_, k)| *k);
        ks.fold(Fx::from_int(1), Fx::max)
    };
    let gain = |e: &bd_core::graph::Influence| match e.curve.is_empty() {
        true => e.k * most(e),
        false => Fx(0),
    };
    (data.loops.iter())
        .map(|(name, ids)| {
            (
                name.clone(),
                (ids.iter().map(edge)).fold(Fx::from_int(1), |g, e| g * gain(e)),
            )
        })
        .filter(|(_, g)| *g >= Fx(1_500))
        .collect()
}

#[test]
fn no_loop_runs_away_outside_its_curves() {
    let data = load_all();
    assert_eq!(data.loops.len(), 5);
    assert!(
        runaway_loops(&data).is_empty(),
        "{:?}",
        runaway_loops(&data)
    );
    // П1 with e2 straight and steep: 2.5 * 0.6 = 1.5.
    let mut steep = data.clone();
    let e2 = steep.influences.iter_mut().find(|e| e.id == "e2").unwrap();
    (e2.curve, e2.k) = (vec![], bd_core::fx::Fx(-2_500));
    assert_eq!(runaway_loops(&steep).len(), 1);
    // Or a law scaling e3 of П1 (0.6 * 0.75 at most now) by 3.4.
    let mut steep = data.clone();
    let e2 = steep.influences.iter_mut().find(|e| e.id == "e2").unwrap();
    (e2.curve, e2.k) = (vec![], bd_core::fx::Fx(-750));
    assert!(runaway_loops(&steep).is_empty());
    steep.laws.list[0]
        .edges
        .push(("e3".into(), bd_core::fx::Fx(3_400)));
    assert_eq!(runaway_loops(&steep).len(), 1);
}

/// Stage 18: stability is derived and an effect writing it goes to the shocks. Stage 20 moves
/// these writes to the nodes behind them; until then, no new ones.
#[test]
fn direct_writes_to_stability_do_not_grow() {
    fn walk<'a>(es: &'a [Effect], out: &mut Vec<&'a Effect>) {
        for e in es {
            match e {
                Effect::Chance(c) => {
                    walk(&c.then, out);
                    walk(&c.otherwise, out);
                }
                Effect::IfFriendly(es) => walk(es, out),
                Effect::Marry { then, otherwise } => {
                    walk(then, out);
                    walk(otherwise, out);
                }
                _ => out.push(e),
            }
        }
    }
    let data = load_all();
    let stability = &data.stability.as_ref().expect("derived").axis;
    let mut writes: Vec<String> = vec![];
    let events = data.events.iter().chain(&data.sim_events);
    let lists = (events.flat_map(|e| e.choices.iter().map(move |c| (&e.id, &c.effects))))
        .chain(data.actions.iter().map(|a| (&a.id, &a.on_complete)));
    for (id, effects) in lists {
        let mut all = vec![];
        walk(effects, &mut all);
        let direct = all
            .iter()
            .filter(|e| matches!(e, Effect::Axis(a, _) if a == stability));
        writes.extend(direct.map(|_| id.clone()));
    }
    eprintln!("warning: stability written directly (as shocks): {writes:?}");
    assert!(
        writes.len() <= 22,
        "new direct writes to stability: {writes:?}"
    );
}
