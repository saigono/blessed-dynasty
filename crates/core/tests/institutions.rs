//! Stage 19: laws-institutions (docs/design/hidden-state.html, section 5).

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{Game, ReignEnd};
use bd_core::graph::{self, InfluenceKind};
use bd_core::rng::Rng;
use bd_core::sim;
use bd_core::state::{AxisId, Preset, World};
use std::fs;
use std::path::PathBuf;

fn read(rel: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    fs::read_to_string(dir.join(rel)).unwrap()
}

/// rules.ron with these laws added to the list, actions and names.
fn data_with(laws: &str) -> Data {
    let rules = read("rules.ron").replacen("        list: [", &format!("        list: [{laws}"), 1);
    let mut data = bd_core::data::load(&rules).unwrap();
    data.add_actions(&read("actions.ron")).unwrap();
    data.add_names(&read("names.ron")).unwrap();
    data
}

fn data() -> Data {
    data_with("")
}

fn game(data: &Data) -> Game {
    let preset = read("presets/default.ron");
    let preset = Preset::load_with_map(&preset, &read("maps/default.ron"), data).unwrap();
    Game::new(data.clone(), &preset, 1)
}

fn ax(s: &str) -> AxisId {
    AxisId(s.into())
}

fn target(d: &Data, w: &World, a: &str) -> Fx {
    graph::target(d, w, d.axes.iter().find(|x| x.id.0 == a).unwrap())
}

/// The contribution of edge `id` into its node now.
fn edge(d: &Data, w: &World, id: &str) -> Fx {
    let i = d.influences.iter().position(|e| e.id == id).unwrap();
    let e = &d.influences[i];
    let parts = graph::parts(d, w, &e.to, InfluenceKind::Target);
    parts.into_iter().find(|(j, _)| *j == i).unwrap().1
}

const X: &str = r#"(id: "law_x", cost: 10, years: 1, anchors: [("serfdom", 40), ("loyalty_nobles", 8)],
    edges: [("e7", 1), ("e1", 2), ("e4", 1.5)]),"#;

/// Acceptance: a law in force shifts the anchor of a node, and the node steps toward it.
#[test]
fn a_law_shifts_the_anchor_of_a_node() {
    let d = data_with(X);
    let mut g = game(&d);
    assert_eq!(target(&d, &g.world, "serfdom"), Fx::from_int(30));
    g.world.flags.insert("law_x".into());
    assert_eq!(target(&d, &g.world, "serfdom"), Fx::from_int(70));
    // Nobles at 50, their anchor 58: +1 a year toward it (e5 still silent at serfdom 30).
    assert_eq!(target(&d, &g.world, "loyalty_nobles"), Fx::from_int(58));
    graph::tick(&d, &mut g.world);
    assert_eq!(g.world.axes[&ax("serfdom")], Fx(30_400));
    assert_eq!(g.world.axes[&ax("loyalty_nobles")], Fx::from_int(41)); // 40 in the preset
    // A succession law of the design: partition, nobles +8.
    let d = data();
    let mut w = game(&d).world;
    w.flags.remove("law_primogeniture");
    w.flags.insert("law_partition".into());
    assert_eq!(target(&d, &w, "loyalty_nobles"), Fx::from_int(58));
}

/// Acceptance: a law in force scales an edge; an edge `off` counts only under a law naming it.
#[test]
fn a_law_scales_an_edge() {
    let d = data_with(X);
    let mut w = game(&d).world;
    w.axes.insert(ax("loyalty_nobles"), Fx::from_int(90));
    w.axes.insert(ax("strata"), Fx::from_int(70));
    w.lagged.clear(); // no smoothing yet: sources read as they are
    // e4 by its curve: 40 * (90 - 50) / 50 = 32; e7: 0.4 * (70 - 50), but off.
    assert_eq!(edge(&d, &w, "e4"), Fx::from_int(32));
    assert_eq!(edge(&d, &w, "e7"), Fx(0));
    w.flags.insert("law_x".into());
    assert_eq!(edge(&d, &w, "e4"), Fx::from_int(48));
    assert_eq!(edge(&d, &w, "e7"), Fx::from_int(8));
    // Two laws multiply: partition puts e4 at 1.25.
    w.flags.remove("law_primogeniture");
    w.flags.insert("law_partition".into());
    assert_eq!(edge(&d, &w, "e4"), Fx::from_int(60));
}

/// A law names known plain axes and known edges; ids are unique; a succession law takes its
/// name from `heirs.laws`.
#[test]
fn laws_are_checked_on_load() {
    let load = |law: &str| {
        let rules =
            read("rules.ron").replacen("        list: [", &format!("        list: [{law}"), 1);
        bd_core::data::load(&rules)
    };
    assert!(load(X).is_ok());
    for bad in [
        r#"(id: "y", cost: 0, years: 1, anchors: [("nothing", 1)]),"#,
        r#"(id: "y", cost: 0, years: 1, resistance: [("loyalty", -10)]),"#, // derived
        r#"(id: "y", cost: 0, years: 1, edges: [("e99", 2)]),"#,
        r#"(id: "y", cost: 0, years: 1, requires: AxisAbove("nothing", 1)),"#,
        r#"(id: "y", cost: 0, years: 1, on_complete: [Axis("nothing", 1)]),"#,
        r#"(id: "law_male", cost: 0, years: 1),"#, // twice
    ] {
        assert!(load(bad).is_err(), "{bad}");
    }
    let d = data();
    assert_eq!(d.law("law_salic").unwrap().name, "Салический закон");
    assert!(d.law("law_salic").unwrap().description.contains("90"));
}

/// A coronation turns the factions a share of the way back to their anchor under the laws in
/// force, not to their default.
#[test]
fn a_coronation_resets_the_factions_toward_the_anchor_of_the_laws() {
    let mut d = data_with(X);
    (d.sim.traits, d.heirs.birth, d.sim.max_years) = (vec![], vec![], 1);
    d.coronation = Default::default();
    d.coronation.reset = Fx(500);
    let mut g = game(&d);
    g.world.flags.insert("law_x".into());
    g.world.axes.insert(ax("loyalty_nobles"), Fx::from_int(20));
    let end = ReignEnd {
        cause: "illness".into(),
        tick: g.world.tick,
        world: g.world.clone(),
    };
    let c = sim::run(end, &d, Rng::from_seed(1));
    // Halfway from 20 to 58.
    assert_eq!(
        c.entries[0].snapshot.axes[&ax("loyalty_nobles")],
        Fx::from_int(39)
    );
}
