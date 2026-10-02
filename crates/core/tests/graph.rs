//! Stage 18: the influence graph.

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{Game, Step};
use bd_core::graph;
use bd_core::sim;
use bd_core::state::{AxisId, Preset, World};
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
    files
        .iter()
        .map(|f| fs::read_to_string(f).unwrap())
        .collect()
}

/// The content of data/.
const DATA: &str = "../../data";
/// main's data/ after stage 17b: no edges, no new nodes, no stability block.
const MAIN: &str = "tests/main/data";

/// The content of the data directory `dir` with this text for its rules.ron.
fn content(dir: &str, rules: &str) -> Data {
    let mut data = bd_core::data::load(rules).unwrap();
    for t in ron_files(&format!("{dir}/events")) {
        data.add_events(&t).unwrap();
    }
    for t in ron_files(&format!("{dir}/events/sim")) {
        data.add_sim_events(&t).unwrap();
    }
    data.add_actions(&read(&format!("{dir}/actions.ron")))
        .unwrap();
    data.add_names(&read(&format!("{dir}/names.ron"))).unwrap();
    data.add_hints(&read(&format!("{dir}/hints.ron"))).unwrap();
    data
}

fn preset(dir: &str, data: &Data) -> Preset {
    let text = read(&format!("{dir}/presets/default.ron"));
    let map = read(&format!("{dir}/maps/default.ron"));
    Preset::load_with_map(&text, &map, data).unwrap()
}

/// FNV-1a, 64 bit.
fn fnv(bytes: &[u8]) -> u64 {
    (bytes.iter()).fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Seeds 0..n, a neutral reign (the middle choice) and its dynasty: the hash of the RON of
/// every reign's last world and chronicle.
fn runs_hash(dir: &str, data: &Data, n: u64) -> u64 {
    let preset = preset(dir, data);
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

/// Golden: main's data (`MAIN`) plays exactly as on main, byte for byte.
#[test]
fn data_without_edges_plays_as_main() {
    let data = content(MAIN, &read(&format!("{MAIN}/rules.ron")));
    assert_eq!(runs_hash(MAIN, &data, 50), 15158972918327858678);
}

/// data/ with `extra` axes and edges added to rules.ron, nothing else.
fn with(axes: &str, edges: &str) -> Data {
    let rules = read(&format!("{DATA}/rules.ron"));
    let rules = rules.replacen("    axes: [\n", &format!("    axes: [\n{axes}\n"), 1);
    let rules = rules.replacen(
        "    influences: [\n",
        &format!("    influences: [\n{edges}\n"),
        1,
    );
    content(DATA, &rules)
}

fn ax(s: &str) -> AxisId {
    AxisId(s.into())
}

fn target(d: &Data, w: &World, id: &str) -> Fx {
    graph::target(d, w, d.axes.iter().find(|a| a.id.0 == id).unwrap())
}

/// Section 2 and 3 of the design: eight hidden nodes, edges e1..e18 (e7 off until its law),
/// and at the start every source stands at its rest: every target is the anchor.
#[test]
fn the_graph_of_the_design_is_silent_at_the_start() {
    let d = content(DATA, &read(&format!("{DATA}/rules.ron")));
    let hidden = (d.axes.iter()).filter(|a| a.hidden && a.reveal.is_some());
    let hidden: Vec<_> = hidden.map(|a| a.id.0.as_str()).collect();
    let nodes = [
        "serfdom",
        "strata",
        "mobility",
        "liberties",
        "trade",
        "grain",
        "literacy",
        "faith",
    ];
    assert_eq!(hidden, nodes);
    for i in 1..=18 {
        assert!(d.influences.iter().any(|e| e.id == format!("e{i}")), "e{i}");
    }
    // e7 only under its law (stage 19).
    let e7 = d.influences.iter().find(|e| e.id == "e7").unwrap();
    assert!(e7.off && e7.k == Fx(400));
    let w = Game::new(d.clone(), &preset(DATA, &d), 0).world;
    for a in d.axes.iter().filter(|a| graph::step(&d, a) > Fx(0)) {
        assert_eq!(target(&d, &w, &a.id.0), a.default, "{}", a.id.0);
    }
}

/// A loop of gain 1.5 runs away from its rest, up to where the curve of its edge goes
/// flat, and no further.
#[test]
fn a_loop_of_gain_over_one_stops_where_its_curve_saturates() {
    let axes = r#"(id: "x", min: 0, max: 100, default: 50, name: "x", hidden: true, step: 5),
        (id: "y", min: 0, max: 100, default: 50, name: "y", hidden: true, step: 5),"#;
    let edges = r#"(from: "x", to: "y", k: 1, rest: 50),
        (from: "y", to: "x", curve: [(40, -15), (50, 0), (60, 15)]),"#;
    let d = with(axes, edges);
    let mut w = Game::new(d.clone(), &preset(DATA, &d), 0).world;
    w.axes.insert(ax("x"), Fx::from_int(52));
    w.axes.insert(ax("y"), Fx::from_int(52));
    let at = |w: &World| {
        (
            w.axes[&ax("x")].0 / Fx::SCALE,
            w.axes[&ax("y")].0 / Fx::SCALE,
        )
    };
    graph::tick(&d, &mut w);
    assert_eq!(at(&w), (53, 52)); // 50 + 1.5 * 2: it grows
    for _ in 0..200 {
        graph::tick(&d, &mut w);
    }
    assert_eq!(at(&w), (65, 65)); // the curve's top, not the axis bound
}

/// The owner adds nodes after playtests: a ninth node and its edges are data only.
#[test]
fn a_node_added_by_data_alone_works() {
    let axes = r#"(id: "justice", min: 0, max: 100, default: 20, name: "Правосудие", hidden: true, step: 1),"#;
    let edges = r#"(id: "j1", from: "liberties", to: "justice", k: 0.5, rest: 15),
        (id: "j2", from: "justice", to: "loyalty_people", k: 0.2, rest: 20),"#;
    let d = with(axes, edges);
    let mut g = Game::new(d.clone(), &preset(DATA, &d), 0);
    assert_eq!(target(&d, &g.world, "justice"), Fx::from_int(20)); // silent at rest
    g.world.axes.insert(ax("liberties"), Fx::from_int(55));
    assert_eq!(target(&d, &g.world, "justice"), Fx::from_int(40));
    for _ in 0..20 {
        graph::tick(&d, &mut g.world);
    }
    // Liberties fall back toward 15 by 0.6 a year; justice climbs by its step of 1, then
    // follows its target down: 20 + 0.5 * (43.6 - 15) as the last tick saw it.
    assert_eq!(g.world.axes[&ax("liberties")], Fx::from_int(43));
    assert_eq!(g.world.axes[&ax("justice")], Fx(34_300));
    let people = ax("loyalty_people");
    let parts = graph::parts(&d, &g.world, &people, graph::InfluenceKind::Target);
    let j2 = parts.into_iter().find(|(i, _)| d.influences[*i].id == "j2");
    assert_eq!(j2.map(|(_, c)| c), Some(Fx(2_860))); // 0.2 * (34.3 - 20)
    // And the game plays on it.
    for _ in 0..30 {
        if let Step::Event(_) = g.wait().unwrap() {
            g.choose(0).unwrap();
        }
    }
}
