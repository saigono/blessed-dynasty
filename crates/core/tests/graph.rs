//! Stage 18: the influence graph.

use bd_core::data::Data;
use bd_core::game::{Game, Step};
use bd_core::state::Preset;
use bd_core::sim;
use std::fs;
use std::path::PathBuf;

fn dir(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(dir(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn ron_files(rel: &str) -> Vec<String> {
    let mut files: Vec<_> = (fs::read_dir(dir(rel)).unwrap())
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "ron"))
        .collect();
    files.sort();
    files.iter().map(|f| fs::read_to_string(f).unwrap()).collect()
}

/// `rules` and `actions` from the given paths, the rest of the content from data/.
fn content(rules: &str, actions: &str) -> Data {
    let mut data = bd_core::data::load(&read(rules)).unwrap();
    for t in ron_files("../../data/events") {
        data.add_events(&t).unwrap();
    }
    for t in ron_files("../../data/events/sim") {
        data.add_sim_events(&t).unwrap();
    }
    data.add_actions(&read(actions)).unwrap();
    data.add_names(&read("../../data/names.ron")).unwrap();
    data.add_hints(&read("../../data/hints.ron")).unwrap();
    data
}

fn preset(data: &Data) -> Preset {
    let text = read("../../data/presets/default.ron");
    Preset::load_with_map(&text, &read("../../data/maps/default.ron"), data).unwrap()
}

/// FNV-1a, 64 bit.
fn fnv(bytes: &[u8]) -> u64 {
    (bytes.iter()).fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Seeds 0..n, a neutral reign (the middle choice) and its dynasty: the hash of the RON of
/// every reign's last world and chronicle.
fn runs_hash(data: &Data, n: u64) -> u64 {
    let preset = preset(data);
    let mut text = String::new();
    for seed in 0..n {
        let mut g = Game::new(data.clone(), &preset, seed);
        let end = loop {
            match g.wait().unwrap() {
                Step::Event(v) => g.choose((v.choices.len() - 1) / 2).unwrap(),
                Step::Idle => {}
                Step::ReignEnded(end) => break end,
            }
        };
        text += &ron::to_string(&end.world).unwrap();
        text += &ron::to_string(&sim::run(end, &g.data, g.rng.clone())).unwrap();
    }
    fnv(text.as_bytes())
}

/// Golden: main's rules.ron and actions.ron (after stage 17b, kept in tests/main/: no edges,
/// no new nodes, no stability block) play exactly as on main, byte for byte.
#[test]
fn data_without_edges_plays_as_main() {
    let data = content("tests/main/rules.ron", "tests/main/actions.ron");
    assert_eq!(runs_hash(&data, 50), 15158972918327858678);
}
