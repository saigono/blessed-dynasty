//! Content lint: loads every data file and checks references across files.

use bd_core::batch::{par_seeds, threads};
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
fn every_building_has_its_icon() {
    let mut data = load_all();
    assert_eq!(lint::buildings(&data), Vec::<String>::new());
    data.buildings.retain(|b| b.id != "dikes");
    assert_eq!(lint::buildings(&data).len(), 1);
}

#[test]
fn every_spawned_event_exists() {
    assert_eq!(lint::spawned(&load_all()), Vec::<String>::new());
}

/// Acceptance (stage 29b): every event the ruler sees, asked or a message, has a picture, and
/// every picture named, the simulation's too, is a file of assets/sprites/events.
#[test]
fn every_event_has_an_existing_picture() {
    let data = load_all();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/sprites/events");
    let no_picture: Vec<&str> = (data.events.iter())
        .filter(|e| e.image.is_empty())
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(no_picture, Vec::<&str>::new());
    let missing: Vec<String> = (data.events.iter().chain(&data.sim_events))
        .filter(|e| !e.image.is_empty() && !dir.join(format!("{}.jpg", e.image)).is_file())
        .map(|e| format!("{}: {}", e.id, e.image))
        .collect();
    assert_eq!(missing, Vec::<String>::new());
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
    // Stage 24: royal_will, forged_will. Stage 28: province_breakaway.
    assert_eq!(data.sim_events.len(), 18);
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
    let fired_of = |seed| {
        let mut g = Game::new(data.clone(), &preset, seed);
        let mut fired = vec![];
        for _ in 0..60 {
            match g.wait().unwrap() {
                Step::Event(v) => {
                    g.choose((v.choices.len() - 1) / 2).unwrap();
                    fired.push(v.event_id);
                }
                Step::Idle => {}
                Step::ReignEnded(_) => break,
            }
        }
        fired
    };
    let fired: BTreeSet<String> = par_seeds(0..2000, threads(), fired_of, |_| {})
        .into_iter()
        .flatten()
        .collect();
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
/// without a founder's law faith never falls to the schism. Stage 26c: 2000 dynasties, not
/// 1000: «Подложное завещание» comes about once in 700, and the compound events moved the
/// thousand it fell twice in to one it never does.
#[test]
#[ignore = "two minutes in release; stage 9 acceptance, run with --release --ignored"]
fn every_sim_event_fires_in_a_thousand_dynasties() {
    let data = load_all();
    let preset = preset(&read("presets/default.ron"), &data);
    let chronicle = |seed| {
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
        // Ids only: an entry carries a world snapshot, 2000 chronicles of them outgrow memory.
        let entries = sim::run(end, &data, g.rng.clone()).entries;
        entries
            .into_iter()
            .map(|e| e.event.unwrap_or_default())
            .collect::<Vec<_>>()
    };
    let mut fired: BTreeMap<String, u32> = BTreeMap::new();
    for e in par_seeds(0..2000, threads(), chronicle, |_| {})
        .into_iter()
        .flatten()
    {
        *fired.entry(e).or_default() += 1;
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
    for f in [NoHeir, Conquered, Usurped, NoCrownLand, Alive] {
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
/// houses and neighbours of the preset, the epithets, the houses and rulers of the kingdoms.
#[test]
fn every_name_declines() {
    let data = load_all();
    let p = preset(&read("presets/default.ron"), &data);
    let world = bd_core::game::Game::new(data.clone(), &p, 0).world;
    assert_eq!(lint::names(&data, &world), Vec::<String>::new());
    assert_eq!(lint::lint(&data, &world), Vec::<String>::new());
    let mut bare = data.clone();
    bare.names.cases.remove("Эрлинги");
    assert_eq!(
        lint::names(&bare, &world),
        ["Эрлинги: нет падежей (names.ron forms)"]
    );
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

/// The effects of `es`, the nested ones of chances, friends and suits included.
fn all_effects(es: &[Effect]) -> Vec<&Effect> {
    let mut out = vec![];
    for e in es {
        out.push(e);
        match e {
            Effect::Chance(c) => out.extend(
                all_effects(&c.then)
                    .into_iter()
                    .chain(all_effects(&c.otherwise)),
            ),
            Effect::IfFriendly(es) => out.extend(all_effects(es)),
            Effect::Marry { then, otherwise } => {
                out.extend(all_effects(then).into_iter().chain(all_effects(otherwise)))
            }
            _ => {}
        }
    }
    out
}

/// Bug of playtest 02 (stage 25): an event that may take its heir away targets no one
/// younger than `sim.heir_death_age`, so its death is an heir's everywhere, as the
/// chronicle tells it («Беда с наследником» once hunted with babies).
#[test]
fn an_heir_taken_by_an_event_is_of_heir_death_age() {
    use bd_core::rules::{EventTarget, HeirOp};
    let data = load_all();
    let age = data.sim.heir_death_age;
    let mut checked = 0;
    for e in data.events.iter().chain(&data.sim_events) {
        let effects = e.choices.iter().flat_map(|c| all_effects(&c.effects));
        if !effects
            .into_iter()
            .any(|x| *x == Effect::HeirOp(HeirOp::TargetRemove))
        {
            continue;
        }
        let lo = match e.target {
            EventTarget::Heir(lo, _) | EventTarget::UnmarriedHeir(lo, _) => lo,
            _ => panic!("{}: removes a target heir without one", e.id),
        };
        assert!(
            lo >= age,
            "{}: heirs from {lo}, told as heirs from {age}",
            e.id
        );
        checked += 1;
    }
    assert!(checked >= 2, "heir_death and heir_first_campaign at least");
}

/// The default game of `seed` with event `id` waiting at `target`.
fn waiting(seed: u64, id: &str, target: Option<bd_core::rules::Target>) -> Game {
    let data = load_all();
    let preset = preset(&read("presets/default.ron"), &data);
    let mut g = Game::new(data, &preset, seed);
    g.pending_event = Some(bd_core::game::PendingEvent {
        event_id: id.into(),
        target,
        neighbour: None,
    });
    g
}

/// Whether event `id` could fire at `target` now: its `when`, and for a province its filter.
fn could_fire(g: &Game, id: &str, target: &Option<bd_core::rules::Target>) -> bool {
    use bd_core::rules::{EventTarget, Target};
    let e = g.data.events.iter().find(|e| e.id == id).unwrap();
    let w = &g.world;
    let here = match (&e.target, target) {
        (EventTarget::RandomProvince(f), Some(Target::Province(p))) => {
            f.matches(&w.provinces[p], w)
        }
        _ => true,
    };
    let marked =
        (e.unmarked.as_ref()).is_some_and(|m| target.as_ref().is_some_and(|t| w.marked(t, m)));
    e.when.eval(w) && here && !marked
}

/// Plays `g` for `years`, event `id` the likeliest and off cooldown, its other events by
/// their first choice and `id` by its last one. Returns the targets `id` fired at.
fn fired_at(g: &mut Game, id: &str, years: u32) -> Vec<bd_core::rules::Target> {
    for e in &mut g.data.events {
        if e.id == id {
            (e.weight, e.cooldown_years) = (1_000_000, bd_core::time::Years(0));
        }
    }
    let mut at = vec![];
    for _ in 0..years {
        match g.wait() {
            Ok(Step::Event(v)) => {
                let fire = v.event_id == id;
                at.extend(v.target.clone().filter(|_| fire));
                let last = v.choices.len() - 1;
                assert!(
                    g.choose(if fire { last } else { 0 }).is_ok(),
                    "the reign ended"
                );
            }
            Ok(Step::ReignEnded { .. }) | Err(_) => break,
            Ok(_) => {}
        }
    }
    at
}

/// Chooses `choice` of the event waiting in `g` and checks its target is marked `mark`.
fn decide(g: &mut Game, choice: &str, mark: &str) -> bd_core::rules::Target {
    let p = g.pending_event.clone().unwrap();
    let e = g.data.events.iter().find(|e| e.id == p.event_id).unwrap();
    let i = e.choices.iter().position(|c| c.text == choice).unwrap();
    g.choose(i).unwrap();
    let t = p.target.unwrap();
    assert!(g.world.marked(&t, mark), "{t:?}");
    t
}

/// Acceptance, bug of playtest 02 (stage 25): a town rebuilt in stone does not burn again;
/// the other crown provinces still do.
#[test]
fn a_town_rebuilt_in_stone_burns_no_more() {
    use bd_core::rules::Target;
    let capital = Target::Province(bd_core::state::ProvinceId("capital".into()));
    let mut g = waiting(1, "cap_fire", Some(capital.clone()));
    decide(&mut g, "Строить заново только из камня", "stone");
    // 50 years (30 before stage 28: more neighbours, and their events take more years).
    let at = fired_at(&mut g, "cap_fire", 50);
    assert!(!at.contains(&capital), "{at:?}");
    assert!(at.len() >= 5, "{at:?}");
}

/// Acceptance (stage 25): a neighbour with a treaty sends no other embassy, the others do;
/// a war with it breaks the treaty.
#[test]
fn a_treaty_is_signed_once_until_a_war() {
    use bd_core::rules::Target;
    let nordmark = Target::Neighbour(bd_core::state::NeighbourId("nordmark".into()));
    let mut g = waiting(5, "nb_embassy", Some(nordmark.clone()));
    decide(&mut g, "Подписать договор с соседом", "treaty");
    let at = fired_at(&mut g, "nb_embassy", 20);
    assert!(!at.contains(&nordmark) && !at.is_empty(), "{at:?}");
    g.pending_event = Some(bd_core::game::PendingEvent {
        event_id: "war_declared".into(),
        target: Some(nordmark.clone()),
        neighbour: None,
    });
    g.choose(2).unwrap();
    assert!(!g.world.marked(&nordmark, "treaty"));
}

/// Acceptance (stage 25): an heir with an appanage does not ask again, his brother does.
#[test]
fn an_heir_with_an_appanage_asks_no_more() {
    use bd_core::rules::Target;
    let mut g = waiting(1, "heir_appanage", None);
    let brother = g.data.newborn(g.world.next_heir_id);
    g.world.add_heir(brother);
    for h in &mut g.world.heirs {
        h.age = 18;
    }
    let first = Target::Heir(g.world.heirs[0].id);
    g.pending_event.as_mut().unwrap().target = Some(first.clone());
    decide(&mut g, "Дать наследнику удел в кормление", "appanage");
    let at = fired_at(&mut g, "heir_appanage", 10);
    assert!(!at.contains(&first) && !at.is_empty(), "{at:?}");
}

/// Bug of playtest 02 (stage 25): every other decision for good is remembered too, and
/// its event does not ask again (at that place, for a province).
#[test]
fn decisions_for_good_are_remembered() {
    use bd_core::rules::Target;
    let province = |p: &str| Some(Target::Province(bd_core::state::ProvinceId(p.into())));
    for (id, target, choice) in [
        ("cap_guild_charter", None, "Даровать хартию вольностей"),
        ("cap_guild_charter", None, "Продать хартию за серебро"),
        ("fac_church_demands", None, "Платить церкви десятину"),
        ("omen_search_decree", None, "Издать указ о бессрочном сыске"),
        ("dis_flood", province("berg"), "Насыпать вдоль реки валы"),
        (
            "prov_pilgrimage",
            province("holm"),
            "Поставить у источника обитель",
        ),
    ] {
        let mut g = waiting(1, id, target.clone());
        for (a, v) in [
            ("loyalty_church", 50),
            ("loyalty_nobles", 70),
            ("serfdom", 50),
        ] {
            g.world.axes.insert(
                bd_core::state::AxisId(a.into()),
                bd_core::fx::Fx::from_int(v),
            );
        }
        assert!(could_fire(&g, id, &target), "{id}");
        let e = g.data.events.iter().find(|e| e.id == id).unwrap();
        let i = e.choices.iter().position(|c| c.text == choice).unwrap();
        g.choose(i).unwrap();
        assert!(!could_fire(&g, id, &target), "{id}: {choice}");
    }
    // A cathedral consecrated is not asked for again.
    let mut g = waiting(1, "cap_cathedral_consecrated", None);
    g.world.flags.insert("cathedral_building".into());
    g.choose(0).unwrap();
    assert!(!could_fire(&g, "cap_cathedral", &None));
}

/// Stage 26c acceptance: the minimums of variants hold (an event's text 3, a told 3, a record
/// of the simulation 5, a slot of a life 5, an epithet's names 2), and a text short of its
/// minimum is named.
#[test]
fn texts_have_their_minimum_of_variants() {
    let data = load_all();
    assert_eq!(lint::variants(&data), Vec::<String>::new());
    let mut short = data.clone();
    let fire = short
        .events
        .iter_mut()
        .find(|e| e.id == "cap_fire")
        .unwrap();
    fire.texts.pop();
    fire.choices[0].retold.pop();
    let t = &mut short.sim.texts;
    t.variants.get_mut("crowned").unwrap().truncate(3);
    t.life.soon.pop();
    t.epithets[0].also.clear();
    assert_eq!(
        lint::variants(&short),
        [
            "cap_fire: текст: вариантов 2, нужно не меньше 3",
            "cap_fire: told «Отстроить посад из казны»: вариантов 2, нужно не меньше 3",
            "sim.texts.crowned: вариантов 4, нужно не меньше 5",
            "sim.texts.life.soon: вариантов 4, нужно не меньше 5",
            "Собиратель земель: имена: вариантов 1, нужно не меньше 2",
        ]
    );
}

/// Stage 26c: a compound event (one waiting for its causes, `FiredWithin`) recalls them in
/// every way the chronicle may tell it, not only in its text.
#[test]
fn compound_events_recall_their_causes_in_every_told() {
    let data = load_all();
    let stories: Vec<_> = (data.events.iter())
        .filter(|e| format!("{:?}", e.when).contains("FiredWithin"))
        .collect();
    assert!(stories.len() >= 6, "{}", stories.len());
    for e in stories {
        let told = (e.choices.iter()).flat_map(|c| std::iter::once(&c.told).chain(&c.retold));
        for t in told {
            assert!(t.contains("{prev_"), "{}: {t}", e.id);
        }
    }
}

/// Stage 28 acceptance: the big map is one piece, its neighbours are mutual and are exactly
/// the outlines that share a side, no two outlines overlap (no crossing sides, no corner on
/// another's side or inside it), and the empire starts beyond the buffers, bordering none of
/// our land.
#[test]
fn the_big_map_is_whole_and_the_empire_starts_far() {
    use bd_core::state::{Holder, NeighbourId};
    let data = load_all();
    let map = preset(&read("presets/default.ron"), &data).map;
    let by: BTreeMap<_, _> = (map.provinces.iter())
        .map(|p| (p.id.0.as_str(), p))
        .collect();
    assert!(by.len() >= 40, "{}", by.len());
    let listed: BTreeSet<(&str, &str)> = (map.provinces.iter())
        .flat_map(|p| p.neighbours.iter().map(|n| (p.id.0.as_str(), n.0.as_str())))
        .collect();
    for (a, b) in &listed {
        assert!(listed.contains(&(*b, *a)), "{a} -> {b} only");
    }
    let mut seen = BTreeSet::from(["capital"]);
    let mut todo = vec!["capital"];
    while let Some(p) = todo.pop() {
        for n in &by[p].neighbours {
            if seen.insert(n.0.as_str()) {
                todo.push(n.0.as_str());
            }
        }
    }
    assert_eq!(seen.len(), by.len(), "not connected");
    // Sides of the outlines; a side of two outlines makes them neighbours.
    type Pt = (i32, i32);
    let mut sides: BTreeMap<(Pt, Pt), Vec<&str>> = BTreeMap::new();
    for (id, poly) in &map.polygons {
        for (i, a) in poly.iter().enumerate() {
            let b = poly[(i + 1) % poly.len()];
            sides
                .entry((*a.min(&b), *a.max(&b)))
                .or_default()
                .push(&id.0);
        }
    }
    let mut shared = BTreeSet::new();
    for ids in sides.values() {
        assert!(ids.len() <= 2, "{ids:?}");
        if let [a, b] = ids[..] {
            shared.extend([(a, b), (b, a)]);
        }
    }
    assert_eq!(shared, listed);
    let orient = |a: Pt, b: Pt, c: Pt| {
        let (a, b, c) = (
            (a.0 as i64, a.1 as i64),
            (b.0 as i64, b.1 as i64),
            (c.0 as i64, c.1 as i64),
        );
        ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).signum()
    };
    let keys: Vec<_> = sides.keys().collect();
    for (i, &&(a, b)) in keys.iter().enumerate() {
        for &&(c, d) in &keys[i + 1..] {
            let apart = [a, b].contains(&c) || [a, b].contains(&d);
            let cross =
                orient(a, b, c) * orient(a, b, d) < 0 && orient(c, d, a) * orient(c, d, b) < 0;
            assert!(apart || !cross, "{a:?}-{b:?} crosses {c:?}-{d:?}");
        }
    }
    let corners: BTreeSet<Pt> = map.polygons.values().flatten().copied().collect();
    for &(a, b) in sides.keys() {
        for &p in &corners {
            let within = p.0 >= a.0.min(b.0)
                && p.0 <= a.0.max(b.0)
                && p.1 >= a.1.min(b.1)
                && p.1 <= a.1.max(b.1);
            let on = p != a && p != b && orient(a, b, p) == 0 && within;
            assert!(!on, "{p:?} on {a:?}-{b:?}");
        }
    }
    for (id, poly) in &map.polygons {
        for &p in corners.iter().filter(|p| !poly.contains(p)) {
            let crossings = (0..poly.len()).filter(|&i| {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                let d = (b.1 - a.1) as i64;
                (a.1 > p.1) != (b.1 > p.1)
                    && (((b.0 - a.0) as i64 * (p.1 - a.1) as i64 - (p.0 - a.0) as i64 * d)
                        * d.signum())
                        > 0
            });
            assert!(crossings.count() % 2 == 0, "{p:?} inside {}", id.0);
        }
    }
    let empire = Holder::Foreign(NeighbourId("kadar".into()));
    let ours = |h: &Holder| !matches!(h, Holder::Foreign(_));
    for p in map.provinces.iter().filter(|p| p.holder == empire) {
        for n in &p.neighbours {
            assert!(
                !ours(&by[n.0.as_str()].holder),
                "{} borders our {}",
                p.id.0,
                n.0
            );
        }
    }
    assert!((10..=12).contains(&map.provinces.iter().filter(|p| p.holder == empire).count()));
}

/// Stage 28b acceptance: our kingdom and every kingdom founded later start on partition, the
/// custom of the land.
#[test]
fn the_preset_starts_on_partition() {
    let data = load_all();
    let p = preset(&read("presets/default.ron"), &data);
    let law = |flags: &BTreeSet<String>| {
        let laws = data.heirs.laws.iter().filter(|l| flags.contains(&l.flag));
        laws.map(|l| l.flag.clone()).collect::<Vec<_>>()
    };
    assert_eq!(law(&p.flags), ["law_partition"]);
    let founded = p.realms.unwrap().founded.unwrap();
    assert_eq!(law(&founded.flags), ["law_partition"]);
}

/// Stage 28b acceptance: the firmer a succession law holds the crown together, the dearer
/// it is and the more loyal the nobles it needs to start: partition, then seniority and
/// election, then male primogeniture and the Salic law, then absolute primogeniture, the
/// longest to bring in and the most resisted by the nobles, needing their loyalty above 60.
#[test]
fn succession_laws_are_priced_by_the_ladder() {
    use bd_core::fx::Fx;
    use bd_core::rules::Predicate;
    let data = rules();
    let law = |id: &str| data.law(id).unwrap();
    let nobles = |a: &bd_core::state::AxisId| a.0 == "loyalty_nobles";
    // The loyalty of the nobles a law needs to start, if any.
    let need = |id: &str| match &law(id).requires {
        Predicate::All(ps) => ps.iter().find_map(|p| match p {
            Predicate::AxisAbove(a, v) if nobles(a) => Some(*v),
            _ => None,
        }),
        _ => None,
    };
    let resisted = |id: &str| {
        let r = law(id).resistance.iter().filter(|(a, _)| nobles(a));
        r.fold(Fx(0), |s, (_, v)| s + *v)
    };
    let ladder: [&[&str]; 4] = [
        &["law_partition"],
        &["law_seniority", "law_elective"],
        &["law_male", "law_salic"],
        &["law_primogeniture"],
    ];
    for w in ladder.windows(2) {
        for (a, b) in w[0].iter().flat_map(|a| w[1].iter().map(move |b| (*a, *b))) {
            assert!(law(a).cost < law(b).cost, "{a} {b}");
            assert!(law(a).years <= law(b).years, "{a} {b}");
            assert!(need(a) <= need(b), "{a} {b}");
        }
    }
    for id in ["law_partition", "law_seniority", "law_elective"] {
        assert_eq!(need(id), None, "{id}");
    }
    let top = "law_primogeniture";
    assert!(need(top) >= Some(Fx::from_int(59)), "{:?}", need(top));
    for l in (data.laws.list.iter()).filter(|l| l.group == law(top).group && l.id != top) {
        assert!(law(top).years > l.years, "{}", l.id);
        assert!(resisted(top) < resisted(&l.id), "{}", l.id);
        assert!(need(top) > need(&l.id), "{}", l.id);
    }
}
