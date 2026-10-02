//! Content lint: loads every data file and checks references across files.

use bd_core::data::{self, Data, DataError};
use bd_core::game::{Game, Step};
use bd_core::rules::Effect;
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
/// game sees them.
fn load_all() -> Data {
    let mut data = rules();
    for f in files("events") {
        let text = fs::read_to_string(&f).unwrap();
        data.add_events(&text)
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

#[test]
fn ids_are_unique() {
    let data = load_all();
    let events: BTreeSet<_> = data.events.iter().map(|e| &e.id).collect();
    let actions: BTreeSet<_> = data.actions.iter().map(|a| &a.id).collect();
    assert_eq!(events.len(), data.events.len());
    assert_eq!(actions.len(), data.actions.len());
}

#[test]
fn every_cause_tag_has_a_hint() {
    let data = load_all();
    let hints = hints();
    let choices = data.events.iter().flat_map(|e| &e.choices);
    let tags = choices.map(|c| &c.cause_tag);
    let tags = tags.chain(data.actions.iter().map(|a| &a.cause_tag));
    let missing: BTreeSet<_> = tags.filter(|t| !hints.contains_key(*t)).collect();
    assert!(missing.is_empty(), "no hint for {missing:?}");
    let numbered: Vec<_> = hints.values().filter(|h| has_digits(h)).collect();
    assert!(numbered.is_empty(), "hints with numbers: {numbered:?}");
}

#[test]
fn every_spawned_event_exists() {
    let data = load_all();
    let ids: BTreeSet<_> = data.events.iter().map(|e| e.id.as_str()).collect();
    let choices = data.events.iter().flat_map(|e| &e.choices);
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

/// Events `rules.ron` names: the war start, neighbour AI events, death and abdication.
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
    let missing: BTreeSet<_> = (named.chain(death).chain(fixed))
        .filter(|id| !data.events.iter().any(|e| e.id == **id))
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
