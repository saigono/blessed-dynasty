//! Content lint: loads every data file and checks references across files.

use bd_core::data::{self, Data, DataError};
use bd_core::game::{Game, Step};
use bd_core::lint;
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
    data.add_hints(&read("hints.ron")).unwrap();
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
/// full stop): lowercase start, no final punctuation, no numbers. The chain templates
/// (`chain:` keys, stage 20) follow the same format and are checked by
/// `chain_templates_cover_the_graph`.
#[test]
fn every_cause_tag_has_a_hint() {
    assert_eq!(lint::hints(&load_all()), Vec::<String>::new());
}

#[test]
fn every_spawned_event_exists() {
    assert_eq!(lint::spawned(&load_all()), Vec::<String>::new());
}

/// Events `rules.ron` names: the war start, neighbour AI events (simulation ones included),
/// death and abdication.
#[test]
fn every_event_named_by_the_rules_exists() {
    assert_eq!(lint::named(&load_all()), Vec::<String>::new());
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
    assert_eq!(lint::told(&load_all()), Vec::<String>::new());
}

/// The brief of stage 9: twelve simulation events, 2-3 choices each, every one important
/// enough for the chronicle. Stage 20 adds the schism.
#[test]
fn sim_events_follow_the_brief() {
    let data = load_all();
    // Stage 24: royal_will, forged_will.
    assert_eq!(data.sim_events.len(), 17);
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
/// simulation event and every omen reaches the chronicle at least once. Stage 20: every
/// fourth dynasty has «Городские вольности» in force from the start (as `cli batch --law`):
/// without a founder's law faith never falls to the schism.
#[test]
#[ignore = "about a minute in release; stage 9 acceptance, run with --release --ignored"]
fn every_sim_event_fires_in_a_thousand_dynasties() {
    let data = load_all();
    let preset = preset(&read("presets/default.ron"), &data);
    let mut fired: BTreeMap<String, u32> = BTreeMap::new();
    for seed in 0..1000 {
        let mut g = Game::new(data.clone(), &preset, seed);
        if seed % 4 == 0 {
            g.world.flags.insert("law_charters".into());
            g.world.laws.insert("law_charters".into(), g.world.tick);
        }
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
    let omens = data.events.iter().filter(|e| e.omen);
    let silent: Vec<_> = (data.sim_events.iter().chain(omens))
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

/// Stage 18: stability is derived and an effect writing it goes to the shocks. Stage 20 moved
/// the writes of brigands, drought and famine to the nodes behind them (trade, grain); the
/// schism's shock is by design (docs/design/hidden-state.html, section 4); no new ones.
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

/// Stage 20: the catastrophes of `symptoms` and their symptoms exist, every symptom is an
/// omen, and every omen foretells one.
#[test]
fn symptoms_are_omens_of_known_catastrophes() {
    let data = load_all();
    let all = || data.events.iter().chain(&data.sim_events);
    let event = |id: &String| all().find(|e| e.id == *id);
    assert_eq!(data.symptoms.len(), 3);
    for (catastrophe, symptoms) in &data.symptoms {
        assert!(event(catastrophe).is_some_and(|e| !e.omen), "{catastrophe}");
        for s in symptoms {
            assert!(event(s).is_some_and(|e| e.omen), "{s}");
        }
    }
    let foretold = |id: &String| data.symptoms.iter().any(|(_, s)| s.contains(id));
    let stray: Vec<_> = all().filter(|e| e.omen && !foretold(&e.id)).collect();
    assert!(stray.is_empty(), "{stray:?}");
}

/// Stage 20: the chain of a chronicle entry is told by the `chain:` keys of hints.ron: both
/// ways of every axis an edge of the graph targets or comes from, the start and the joint;
/// a lead names a known event. Format as for the hints.
#[test]
fn chain_templates_cover_the_graph() {
    let data = load_all();
    let hints = hints();
    let chain: BTreeMap<&str, &String> = (hints.iter())
        .filter_map(|(k, v)| Some((k.strip_prefix("chain:")?, v)))
        .collect();
    let target = bd_core::graph::InfluenceKind::Target;
    let edges = data.influences.iter().filter(|e| e.kind == target);
    let nodes: BTreeSet<String> = edges
        .flat_map(|e| [e.from.0.clone(), e.to.0.clone()])
        .collect();
    for n in &nodes {
        for way in ["+", "-"] {
            assert!(chain.contains_key(format!("{n}{way}").as_str()), "{n}{way}");
        }
    }
    for k in ["then", "since_decision", "since_law", "since_mark"] {
        assert!(chain.contains_key(k), "{k}");
    }
    let all = || data.events.iter().chain(&data.sim_events);
    for k in chain.keys() {
        let node = k
            .strip_suffix(['+', '-'])
            .is_some_and(|n| nodes.contains(n));
        let known = ["then", "since_decision", "since_law", "since_mark"].contains(k);
        assert!(node || known || all().any(|e| e.id == *k), "chain:{k}");
    }
    let bad: Vec<_> = (chain.values())
        .filter(|h| {
            let first = h.chars().next().is_some_and(char::is_lowercase);
            !first || h.ends_with(['.', '!', '?', ',', ' ']) || has_digits(h)
        })
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
}

/// Stage 22: every choice tells the chronicle what was done, in words, without numbers.
#[test]
fn every_choice_is_told() {
    let mut data = load_all();
    data.events[0].choices[0].told = "в 1200 году".into();
    data.sim_events[0].choices[0].told = String::new();
    data.events[1].choices[0].hint = None;
    assert_eq!(lint::told(&data).len(), 3, "{:?}", lint::told(&data));
}

/// Every template the game and the simulation fill: events, choices, `told`, the texts of
/// the simulation, the epithets and the life.
#[test]
fn templates_use_known_names_cases_and_two_forms() {
    let data = load_all();
    let all = lint::template_texts(&data);
    assert!(all.len() > 500, "{}", all.len());
    assert_eq!(lint::templates(&data), Vec::<String>::new());
    // The check itself.
    assert_eq!(
        lint::bad_braces("{ruler.род} {heir:он|она} {year}").len(),
        0
    );
    assert_eq!(lint::bad_braces("{king} {ruler.зв} {heir:он}").len(), 3);
    let mut broken = data.clone();
    broken.sim.texts.life.same_year.clear();
    broken.events[0].title = "{king}".into();
    assert_eq!(lint::templates(&broken).len(), 2);
}

/// Every name a text may decline has its six cases: the pools, the lands of the map, the
/// houses and neighbours of the preset, the epithets.
#[test]
fn every_name_declines() {
    let data = load_all();
    let world =
        bd_core::state::World::from_preset(&data, &preset(&read("presets/default.ron"), &data));
    assert_eq!(lint::names(&data, &world), Vec::<String>::new());
    assert_eq!(lint::lint(&data, &world), Vec::<String>::new());
}

/// A name written without its cases stays in the nominative, and the lint names it.
#[test]
fn a_name_without_cases_falls_back_to_the_nominative_and_is_reported() {
    let mut data = rules();
    let names = r#"(rulers: ["Ульрих||а|у|а|ом|е", "Тассило"], heirs: [], vassals: [])"#;
    data.add_names(names).unwrap();
    let n = &data.names;
    assert_eq!(n.rulers, ["Ульрих", "Тассило"]);
    assert_eq!(n.declined("Ульрих", 1), "Ульриха");
    assert_eq!(n.declined("Тассило", 1), "Тассило");
    let named = [("ruler", "Тассило", None)];
    assert_eq!(bd_core::text::fill("у {ruler.род}", n, &named), "у Тассило");
    assert_eq!(
        n.undeclined(n.rulers.iter().map(|s| s.as_str())),
        ["Тассило"]
    );
    // A spec of the wrong length is an error.
    let broken = r#"(rulers: ["Ульрих||а|у"], heirs: [], vassals: [])"#;
    assert!(matches!(
        rules().add_names(broken),
        Err(DataError::Invalid(_))
    ));
}

/// The deeds an epithet counts are ones a reign counts (`Epithet`), and every way a reign
/// ends has its phrase in a life.
#[test]
fn epithets_count_known_deeds_and_lives_tell_every_end() {
    let mut data = load_all();
    assert_eq!(lint::epithets(&data), Vec::<String>::new());
    data.sim.texts.epithets[0].deeds.push("nothing".into());
    data.sim.texts.life.falls.clear();
    // The deed, and the four falls a life tells.
    assert_eq!(
        lint::epithets(&data).len(),
        5,
        "{:?}",
        lint::epithets(&data)
    );
}

/// Stage 23: each check of `lint` names what breaks it.
#[test]
fn lint_reports_broken_references_and_hints() {
    let mut data = load_all();
    data.events[0].choices[0].cause_tag = "no_such_tag".into();
    data.hints
        .insert("orphan".into(), "Заглавная и с точкой 1.".into());
    let hints = lint::hints(&data);
    // No hint for the tag; a hint of no tag, out of format; the old tag's hint unused unless
    // another choice has it.
    assert!(
        hints.iter().any(|h| h.starts_with("no_such_tag: ")),
        "{hints:?}"
    );
    assert_eq!(
        hints.iter().filter(|h| h.starts_with("orphan: ")).count(),
        2
    );
    data.events[0].choices[0]
        .effects
        .push(Effect::SpawnEvent("ghost".into(), bd_core::time::Years(1)));
    assert_eq!(lint::spawned(&data).len(), 1);
    data.war.start_event = "ghost_war".into();
    assert_eq!(lint::named(&data), ["rules.ron: нет события ghost_war"]);
}
