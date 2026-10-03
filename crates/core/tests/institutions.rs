//! Stage 19: laws-institutions (docs/design/hidden-state.html, section 5).

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::Game;
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

const X: &str = r#"(id: "law_x", group: "g", cost: 10, years: 2,
    anchors: [("serfdom", 40), ("loyalty_nobles", 8)], edges: [("e7", 1), ("e1", 2), ("e4", 1.5)],
    resistance: [("loyalty_people", -10)], treasury: -4, on_complete: [Axis("legitimacy", 5)]),
    (id: "law_y", group: "g", cost: 20, years: 1),"#;

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
    let end = g.reign_end("illness".into());
    let c = sim::run(end, &d, Rng::from_seed(1));
    // Halfway from 20 to 58.
    assert_eq!(
        c.entries[0].snapshot.axes[&ax("loyalty_nobles")],
        Fx::from_int(39)
    );
}

fn treasury(g: &Game) -> Fx {
    g.world.axes[&ax("treasury")]
}

fn wait(g: &mut Game, years: u32) {
    for _ in 0..years {
        g.wait().unwrap();
        g.pending_event = None;
    }
}

fn axis_def<'a>(d: &'a Data, a: &str) -> &'a bd_core::data::AxisDef {
    d.axes.iter().find(|x| x.id.0 == a).unwrap()
}

/// Acceptance: a law costs its price at the start and comes into force after its years, with
/// its one-off effects; the tick it came in is kept. Its upkeep is part of the yearly income.
#[test]
fn a_law_costs_its_price_and_takes_its_years() {
    let d = data_with(X);
    let mut g = game(&d);
    let (before, legitimacy) = (treasury(&g), g.world.axes[&ax("legitimacy")]);
    let upkeep = |g: &Game| bd_core::war::income_parts(&g.world, &g.data).1;
    let idle = upkeep(&g);
    g.start_action("enact_law_x", None).unwrap();
    assert_eq!(treasury(&g), before - Fx::from_int(10));
    wait(&mut g, 1);
    assert!(!g.world.flags.contains("law_x"));
    wait(&mut g, 1);
    assert!(g.world.flags.contains("law_x"));
    assert_eq!(g.world.laws["law_x"], bd_core::time::Tick(2));
    assert_eq!(
        g.world.axes[&ax("legitimacy")],
        legitimacy + Fx::from_int(5)
    );
    assert_eq!(upkeep(&g), idle + Fx::from_int(4));
    // In force, it is not offered again.
    let offered: Vec<_> = g
        .available_actions()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(!offered.contains(&"enact_law_x".to_string()));
    assert!(offered.contains(&"repeal_law_x".to_string()));
}

/// Acceptance: one law of a group in force: the new one ends the old.
#[test]
fn one_law_per_group() {
    let d = data_with(X);
    let mut g = game(&d);
    g.world.axes.insert(ax("treasury"), Fx::from_int(500));
    g.start_action("enact_law_x", None).unwrap();
    wait(&mut g, 2);
    g.start_action("enact_law_y", None).unwrap();
    wait(&mut g, 1);
    assert!(g.world.flags.contains("law_y") && !g.world.flags.contains("law_x"));
    assert!(!g.world.laws.contains_key("law_x"));
    // The succession laws are a group too.
    let succession = d.laws.list.iter().filter(|l| l.group == "Наследование");
    let flags = succession.filter(|l| g.world.flags.contains(&l.id)).count();
    assert_eq!(flags, 1);
}

/// Acceptance: while a law is brought in, the faction against it has its anchor lower; once
/// in force, no more.
#[test]
fn resistance_lasts_while_the_law_is_brought_in() {
    let d = data_with(X);
    let mut g = game(&d);
    let people = |g: &Game| graph::anchor(&g.data, &g.world, axis_def(&g.data, "loyalty_people"));
    assert_eq!(people(&g), Fx::from_int(50));
    g.start_action("enact_law_x", None).unwrap();
    assert_eq!(people(&g), Fx::from_int(40));
    wait(&mut g, 1);
    assert_eq!(people(&g), Fx::from_int(40));
    wait(&mut g, 1);
    assert_eq!(people(&g), Fx::from_int(50));
}

/// Acceptance: a repeal costs half the price and ends the law; a law to `keep` has none.
#[test]
fn a_repeal_costs_half_the_price() {
    let d = data_with(X);
    let mut g = game(&d);
    g.world.axes.insert(ax("treasury"), Fx::from_int(500));
    g.start_action("enact_law_x", None).unwrap();
    wait(&mut g, 2);
    let before = treasury(&g);
    g.start_action("repeal_law_x", None).unwrap();
    assert_eq!(treasury(&g), before - Fx::from_int(5));
    wait(&mut g, 2);
    assert!(!g.world.flags.contains("law_x") && !g.world.laws.contains_key("law_x"));
    assert!(d.actions.iter().all(|a| a.id != "repeal_law_salic"));
    // The simulation's automaton pays it all.
    let repeal = d.actions.iter().find(|a| a.id == "repeal_law_x").unwrap();
    assert_eq!(d.auto_cost(repeal), Fx::from_int(10));
}

/// The ten laws of section 5 of the design with its groups; the succession laws a group of
/// their own, kept (no repeal).
#[test]
fn the_ten_laws_of_the_design() {
    let d = data();
    let group = |g: &str| {
        let ls = d.laws.list.iter().filter(|l| l.group == g);
        ls.map(|l| l.id.as_str()).collect::<Vec<_>>()
    };
    assert_eq!(group("Крестьяне"), ["law_serfdom", "law_free_peasants"]);
    assert_eq!(group("Вера"), ["law_one_faith", "law_tolerance"]);
    assert_eq!(group("Наследование").len(), 6);
    let free = group("");
    assert_eq!(
        free,
        [
            "law_tithe",
            "law_schools",
            "law_charters",
            "law_code",
            "law_granaries",
            "law_fairs"
        ]
    );
    for l in &d.laws.list {
        assert!(!l.name.is_empty() && !l.description.is_empty(), "{}", l.id);
        assert_eq!(l.keep, l.group == "Наследование", "{}", l.id);
    }
    // The succession laws edit the graph as in the design's table.
    let law = |id: &str| d.law(id).unwrap();
    assert!(law("law_primogeniture").anchors.is_empty() && law("law_salic").edges.is_empty());
    assert_eq!(law("law_partition").edges.len(), 2);
}

/// Acceptance: the mark of a law brought in by the player does not fade while the law is in
/// force; the rest of the same decision fades as always, and so does the law's once repealed.
#[test]
fn the_mark_of_a_law_lives_while_the_law_is_in_force() {
    use bd_core::state::MarkKey;
    let d = data_with(X);
    let mut g = game(&d);
    g.world.axes.insert(ax("treasury"), Fx::from_int(500));
    g.start_action("enact_law_x", None).unwrap();
    wait(&mut g, 2);
    let weight = |g: &Game, k: &MarkKey| g.world.marks.get(k).map_or(Fx(0), |t| t[0].weight);
    let (law, legitimacy) = (
        MarkKey::Flag("law_x".into()),
        MarkKey::Axis(ax("legitimacy")),
    );
    assert_eq!(weight(&g, &law), Fx::from_int(1));
    wait(&mut g, 30);
    assert_eq!(weight(&g, &law), Fx::from_int(1));
    assert!(weight(&g, &legitimacy) < Fx(500));
    g.world.flags.remove("law_x");
    wait(&mut g, 1);
    assert!(weight(&g, &law) < Fx::from_int(1));
}

/// One law, no group: nobles +8 in force, the people against it.
const P: &str = r#"(id: "law_p", name: "P", cost: 10, years: 1,
    anchors: [("loyalty_nobles", 8)], resistance: [("loyalty_people", -10)]),"#;

/// Acceptance: the automaton repeals a law only while a faction presses against it (below 30
/// against what it resisted or what lowers it, above 70 against what lowers it), and only with
/// the full price in the treasury; a strong faction the law favours keeps it.
#[test]
fn the_automaton_repeals_a_law_only_under_pressure() {
    let d = data_with(P);
    let auto = sim::AutoChooser {
        weights: [("pressure".to_string(), Fx::from_int(100))].into(),
        noise: Fx(0),
    };
    let pick = |people: i64, nobles: i64, treasury: i64| {
        let mut g = game(&d);
        g.world.flags.insert("law_p".into());
        g.world
            .axes
            .insert(ax("loyalty_people"), Fx::from_int(people));
        g.world
            .axes
            .insert(ax("loyalty_nobles"), Fx::from_int(nobles));
        g.world.axes.insert(ax("treasury"), Fx::from_int(treasury));
        auto.action(&mut g).map(|(id, _)| id)
    };
    let repeal = Some("repeal_law_p".to_string());
    assert_eq!(pick(50, 50, 100), None);
    assert_eq!(pick(29, 50, 100), repeal);
    // Strong nobles want more: a law that raises them, not a repeal.
    assert_eq!(pick(50, 80, 100), Some("enact_law_serfdom".to_string()));
    // The people press, but only half the price is there.
    assert_eq!(pick(29, 50, 9), None);
    // Nobles strong at 80 and the people low: +8 against -10, the law goes.
    assert_eq!(pick(29, 80, 100), repeal);
    // A law of succession is no matter of the factions.
    let mut g = game(&d);
    g.world.axes.insert(ax("loyalty_nobles"), Fx::from_int(10));
    assert_eq!(auto.action(&mut g), None);
}

/// A repeal goes to the chronicle («Отмена закона», the law named in its text), as does a
/// law brought in («Новый закон»).
#[test]
fn the_chronicle_tells_a_law_repealed_and_brought_in() {
    let mut d = data_with(P);
    (d.sim.traits, d.heirs.birth, d.sim.max_years) = (vec![], vec![], 3);
    d.coronation = Default::default();
    (d.sim.auto.noise, d.sim.auto.traits) = (Fx(0), Default::default());
    let texts = |d: &Data, in_force: bool| {
        let mut g = game(d);
        if in_force {
            g.world.flags.insert("law_p".into());
            g.world.axes.insert(ax("loyalty_people"), Fx::from_int(5));
        }
        g.world.axes.insert(ax("treasury"), Fx::from_int(500));
        let end = g.reign_end("illness".into());
        let c = sim::run(end, d, Rng::from_seed(1));
        let told = c.entries.into_iter().map(|e| (e.title, e.text));
        told.collect::<Vec<_>>()
    };
    let has = |told: &[(String, String)], title: &str, law: &str| {
        (told.iter()).any(|(t, x)| t == title && x.contains(&format!("закон «{law}»")))
    };
    d.sim.auto.base = [("pressure".to_string(), Fx::from_int(100))].into();
    let told = texts(&d, true);
    assert!(has(&told, "Отмена закона", "P"), "{told:?}");
    d.sim.auto.base = [("law_p".to_string(), Fx::from_int(100))].into();
    let told = texts(&d, false);
    assert!(has(&told, "Новый закон", "P"), "{told:?}");
    // One of a group replaced by another is no repeal.
    let mut d = data_with(&X.replace("cost: 20, years: 1", "name: \"Y\", cost: 20, years: 1"));
    (d.sim.traits, d.heirs.birth, d.sim.max_years) = (vec![], vec![], 3);
    d.coronation = Default::default();
    (d.sim.auto.noise, d.sim.auto.traits) = (Fx(0), Default::default());
    d.sim.auto.base = [("law_y".to_string(), Fx::from_int(100))].into();
    let mut g = game(&d);
    g.world.flags.insert("law_x".into());
    g.world.axes.insert(ax("treasury"), Fx::from_int(500));
    let end = g.reign_end("illness".into());
    let told: Vec<_> = (sim::run(end, &d, Rng::from_seed(1)).entries.into_iter())
        .map(|e| (e.title, e.text))
        .collect();
    assert!(has(&told, "Новый закон", "Y"), "{told:?}");
    assert!(!told.iter().any(|(t, _)| t == "Отмена закона"), "{told:?}");
}
