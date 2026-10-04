//! Stage 24: the founder's testament (testament.rs): its precept on the automaton, its
//! strength, its cost, the order kept and broken, the wills of the simulation's rulers.

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{DecisionKind, Game};
use bd_core::rng::Rng;
use bd_core::rules::{Choice, Effect, HeirOp};
use bd_core::sim::{self, AutoChooser, Chronicle};
use bd_core::state::{AxisId, Preset, World};
use bd_core::testament::{self, Order, Testament};
use bd_core::time::Tick;
use std::fs;
use std::path::PathBuf;

const PRESET: &str = include_str!("../../../data/presets/default.ron");
const MAP: &str = include_str!("../../../data/maps/default.ron");

fn read(rel: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    fs::read_to_string(dir.join(rel)).unwrap()
}

/// Everything in data/, the simulation events included.
fn content() -> Data {
    let mut data = bd_core::data::load(&read("rules.ron")).unwrap();
    for f in ["death", "heirs", "neighbours", "omens", "reign", "war"] {
        data.add_events(&read(&format!("events/{f}.ron"))).unwrap();
    }
    for f in ["sim", "testament"] {
        data.add_sim_events(&read(&format!("events/sim/{f}.ron")))
            .unwrap();
    }
    data.add_actions(&read("actions.ron")).unwrap();
    data.add_names(&read("names.ron")).unwrap();
    data.add_hints(&read("hints.ron")).unwrap();
    data
}

fn game(data: &Data, seed: u64) -> Game {
    let preset = Preset::load_with_map(PRESET, MAP, data).unwrap();
    Game::new(data.clone(), &preset, seed)
}

fn ax(s: &str) -> AxisId {
    AxisId(s.into())
}

fn precept(id: &str) -> Testament {
    Testament {
        precept: Some(id.into()),
        ..Default::default()
    }
}

/// A world `years` after the founder's death, his testament of `legend`.
fn after(data: &Data, t: Testament, legend: i64, years: u32) -> World {
    let mut w = game(data, 1).world;
    w.testament = Some(Testament {
        legend: Fx::from_int(legend),
        since: Some(Tick(0)),
        ..t
    });
    w.tick = Tick(years);
    w
}

/// The dynasty after a founder who dies at once, at the start, with this testament.
fn dynasty(data: &Data, seed: u64, t: Option<Testament>) -> Chronicle {
    let mut g = game(data, seed);
    g.world.ruler.age = 60;
    if let Some(t) = t {
        g.write_testament(t).unwrap();
    }
    let end = g.reign_end("illness".into());
    sim::run(end, data, Rng::from_seed(seed))
}

fn choice(effects: Vec<Effect>) -> Choice {
    Choice {
        text: String::new(),
        effects,
        cause_tag: String::new(),
        hint: None,
        told: String::new(),
        retold: vec![],
    }
}

/// Acceptance: the precept shifts the automaton by its weights times the strength.
#[test]
fn a_precept_shifts_the_automaton() {
    let data = content();
    let w = after(&data, precept("treasury"), 1, 0);
    let mut base = AutoChooser::for_ruler(&data, &w.ruler);
    base.noise = Fx(0);
    let mindful = testament::chooser(&base, &data, &w);
    let k = testament::strength(&data, &w);
    assert!(k > Fx(0));
    for key in ["treasury", "war"] {
        let p = data
            .testament
            .as_ref()
            .unwrap()
            .precept("treasury")
            .unwrap();
        assert_eq!(
            mindful.weights[key],
            base.weights[key] + p.weights[key] * k,
            "{key}"
        );
    }
    // Gold over glory: the base takes prestige, the mindful the treasury.
    let mut g = game(&data, 1);
    g.world = w;
    let choices = [
        choice(vec![Effect::Axis(ax("treasury"), Fx::from_int(40))]),
        choice(vec![Effect::Axis(ax("prestige"), Fx::from_int(60))]),
    ];
    assert_eq!(base.choose(&mut g, &choices), 1);
    assert_eq!(mindful.choose(&mut g, &choices), 0);
    // No testament, or before the founder's death: nothing changes.
    g.world.testament.as_mut().unwrap().since = None;
    assert_eq!(testament::chooser(&base, &data, &g.world), base);
}

/// Acceptance: the strength falls with the years by `decay` (half in 70) and grows with the
/// legend; the legend grows with prestige, the years of the reign and the laws brought in.
#[test]
fn the_strength_fades_and_grows_with_the_legend() {
    let data = content();
    let s =
        |legend, years| testament::strength(&data, &after(&data, precept("law"), legend, years));
    assert!(s(1, 0) > s(1, 35) && s(1, 35) > s(1, 70) && s(1, 70) > s(1, 140));
    assert_eq!(s(1, 70) * Fx::from_int(2), s(1, 0));
    assert_eq!(s(2, 70), s(1, 0));
    let mut w = game(&data, 1).world;
    let legend = |w: &World| testament::legend(&data, w);
    let l0 = legend(&w);
    w.axes
        .insert(ax("prestige"), w.axes[&ax("prestige")] + Fx::from_int(40));
    let l1 = legend(&w);
    w.tick = Tick(30);
    let l2 = legend(&w);
    w.laws.insert("law_charters".into(), Tick(3));
    let l3 = legend(&w);
    assert!(l0 < l1 && l1 < l2 && l2 < l3, "{l0:?} {l1:?} {l2:?} {l3:?}");
}

/// Acceptance: zeal is higher at low legitimacy; `pious` raises it, `defiant` lowers it.
#[test]
fn zeal_is_higher_at_low_legitimacy() {
    let data = content();
    let zeal = |legitimacy: i64, traits: &[&str]| {
        let mut w = after(&data, precept("faith"), 1, 0);
        w.axes.insert(ax("legitimacy"), Fx::from_int(legitimacy));
        w.ruler.traits = traits.iter().map(|t| t.to_string()).collect();
        testament::parts(&data, &w).unwrap().2
    };
    assert!(zeal(10, &[]) > zeal(50, &[]) && zeal(50, &[]) > zeal(90, &[]));
    assert!(zeal(50, &["pious"]) > zeal(50, &[]));
    assert!(zeal(50, &["defiant"]) < zeal(50, &[]));
    assert!(zeal(90, &["defiant"]) >= Fx(0));
}

/// Acceptance: a testament written young and hale costs the nobles and stability, the more
/// the earlier, and again at every rewrite; written old or sick, nothing.
#[test]
fn an_early_testament_costs_and_a_late_one_does_not() {
    let data = content();
    let mut g = game(&data, 1);
    let cost = |g: &mut Game, age: u32, health: i64| {
        (g.world.ruler.age, g.world.ruler.health) = (age, Fx::from_int(health));
        testament::cost(&data, &g.world)
    };
    let nobles = |c: &[(AxisId, Fx)]| c.iter().find(|(a, _)| a.0 == "loyalty_nobles").map(|c| c.1);
    let (young, older) = (cost(&mut g, 25, 90), cost(&mut g, 45, 90));
    assert!(nobles(&young) < nobles(&older) && nobles(&older) < Some(Fx(0)));
    assert!(young.iter().any(|(a, v)| a.0 == "stability" && *v < Fx(0)));
    assert_eq!(cost(&mut g, 50, 90), vec![]);
    assert_eq!(cost(&mut g, 30, 30), vec![]);
    // Written at 32 and hale, twice: twice the cost, two decisions in the journal.
    (g.world.ruler.age, g.world.ruler.health) = (32, Fx::from_int(90));
    let loyalty = |g: &Game| g.world.axes[&ax("loyalty_nobles")];
    let l0 = loyalty(&g);
    g.write_testament(precept("treasury")).unwrap();
    let l1 = loyalty(&g);
    g.write_testament(precept("sword")).unwrap();
    let l2 = loyalty(&g);
    assert!(l1 < l0 && l2 < l1, "{l0:?} {l1:?} {l2:?}");
    let wills = (g.decisions.iter()).filter(|d| matches!(d.kind, DecisionKind::Testament(_)));
    assert_eq!(wills.count(), 2);
    assert_eq!(
        g.world.testament.as_ref().unwrap().precept.as_deref(),
        Some("sword")
    );
    // Old: free.
    g.world.ruler.age = 60;
    let l3 = loyalty(&g);
    g.write_testament(precept("faith")).unwrap();
    assert_eq!(loyalty(&g), l3);
    // Nothing named, or something unknown: refused.
    assert!(g.write_testament(Testament::default()).is_err());
    assert!(g.write_testament(precept("gold")).is_err());
    let bad = Testament {
        order: Some(Order::KeepLaw("law_nothing".into())),
        ..Default::default()
    };
    assert!(g.write_testament(bad).is_err());
}

/// The order of a law: broken once the law leaves force, never twice.
#[test]
fn a_law_repealed_breaks_the_order_once() {
    let data = content();
    let t = Testament {
        order: Some(Order::KeepLaw("law_charters".into())),
        ..Default::default()
    };
    let mut w = after(&data, t, 1, 10);
    let laws = vec!["law_charters".to_string()];
    w.flags.insert("law_charters".into());
    assert!(!testament::broken(&w, &laws, &[], None));
    w.flags.remove("law_charters");
    assert!(testament::broken(&w, &laws, &[], None));
    w.testament.as_mut().unwrap().broken = Some(Tick(10));
    assert!(!testament::broken(&w, &laws, &[], None));
    // Broken, the order weighs no more on the automaton.
    assert!(!testament::weights(&data, &w, Fx::from_int(1)).contains_key("law_charters"));
}

/// The first entry where two chronicles part, with both.
fn parting<'a>(
    a: &'a Chronicle,
    b: &'a Chronicle,
) -> (&'a sim::ChronicleEntry, &'a sim::ChronicleEntry) {
    let i = (a.entries.iter().zip(&b.entries))
        .position(|(x, y)| x != y)
        .unwrap();
    (&a.entries[i], &b.entries[i])
}

/// Acceptance: an order broken costs legitimacy and the factions, times the strength, and is
/// told; the life of the ruler who broke it says so.
#[test]
fn a_broken_order_costs_and_is_told() {
    // The ruler's own war: a sword precept that craves it, past the Peace order's weight.
    let mut data = content();
    let r = data.testament.as_mut().unwrap();
    let sword = r.precepts.iter_mut().find(|p| p.id == "sword").unwrap();
    sword.weights.insert("war".into(), Fx::from_int(300));
    let title = &data.testament.as_ref().unwrap().texts.breach;
    let peace = |nb| Testament {
        precept: Some("sword".into()),
        order: Some(Order::Peace(nb)),
        ..Default::default()
    };
    let neighbours: Vec<_> = game(&data, 1).world.neighbours.into_keys().collect();
    let (seed, t) = (0..40)
        .flat_map(|s| neighbours.iter().map(move |nb| (s, peace(nb.clone()))))
        .find(|(s, t)| {
            dynasty(&data, *s, Some(t.clone()))
                .entries
                .iter()
                .any(|e| &e.title == title)
        })
        .expect("a war on a neighbour within 40 dynasties");
    let mut free = data.clone();
    free.testament.as_mut().unwrap().breach = vec![];
    let (a, b) = (
        dynasty(&data, seed, Some(t.clone())),
        dynasty(&free, seed, Some(t)),
    );
    let (x, y) = parting(&a, &b);
    assert_eq!((&x.title, &y.title), (title, title));
    for axis in ["legitimacy", "loyalty_nobles", "loyalty_church"] {
        assert!(
            x.snapshot.axes[&ax(axis)] < y.snapshot.axes[&ax(axis)],
            "{axis}"
        );
    }
    assert!(x.snapshot.testament.as_ref().unwrap().broken.is_some());
    let broke = &a
        .rulers
        .iter()
        .find(|r| r.start <= x.tick && x.tick <= r.end)
        .unwrap();
    let life = &data.testament.as_ref().unwrap().texts.life_broken;
    assert!(
        life.iter()
            .any(|l| broke.biography.contains(l.split('{').next().unwrap())
                || broke.biography.contains("нарушен")),
        "{}",
        broke.biography
    );
}

/// A Peace order is broken by the ruler's war, not by the neighbour's: a war that starts
/// from an event his stance offers (declared on us, an invasion) is no breach.
#[test]
fn a_neighbour_attacking_breaks_no_peace() {
    let data = content();
    let ai = &data.neighbour_ai;
    let offered = |id: &str| {
        [&ai.expand, &ai.defend, &ai.trade, &ai.wait]
            .iter()
            .any(|s| s.events.iter().any(|(e, _)| e == id))
    };
    let title = &data.testament.as_ref().unwrap().texts.breach;
    let mut seen = 0;
    for seed in 0..40 {
        let w = game(&data, seed).world;
        for nb in w.neighbours.keys() {
            let t = Testament {
                order: Some(Order::Peace(nb.clone())),
                ..Default::default()
            };
            let c = dynasty(&data, seed, Some(t));
            let attacked = c.entries.iter().position(|e| {
                let war = e.snapshot.war.as_ref();
                e.event.as_deref().is_some_and(offered)
                    && war.is_some_and(|x| x.enemy == *nb && x.started == e.tick)
                    && e.snapshot.testament.as_ref().unwrap().broken.is_none()
            });
            let Some(i) = attacked else { continue };
            seen += 1;
            let during = c.entries[i..]
                .iter()
                .take_while(|e| e.snapshot.war.as_ref().is_some_and(|x| x.enemy == *nb));
            for e in during {
                assert_ne!(&e.title, title, "seed {seed}, {nb:?}, {}", e.tick.0);
            }
        }
    }
    assert!(seen >= 3, "attacks seen: {seen}");
}

/// Acceptance: a choice true to the testament raises legitimacy times the strength and the
/// chronicle says so; the founder's life tells his testament, his heirs' lives the faith.
#[test]
fn faith_to_the_testament_raises_legitimacy_and_is_told() {
    let data = content();
    let t = precept("treasury");
    let mut cold = data.clone();
    cold.testament.as_mut().unwrap().faithful.axes = vec![];
    let told = |c: &Chronicle| {
        (c.entries[1..].iter()).any(|e| e.text.contains("завет") || e.text.contains("наказ"))
    };
    let seed = (0..30)
        .find(|s| told(&dynasty(&data, *s, Some(t.clone()))))
        .expect("a told faith within 30 dynasties");
    let (a, b) = (
        dynasty(&data, seed, Some(t.clone())),
        dynasty(&cold, seed, Some(t)),
    );
    let (x, y) = parting(&a, &b);
    assert!(x.snapshot.axes[&ax("legitimacy")] > y.snapshot.axes[&ax("legitimacy")]);
    let tx = &data.testament.as_ref().unwrap().texts;
    // The testament read at the founder's death, first.
    assert_eq!(a.entries[0].title, tx.read.0);
    assert!(
        a.entries[0]
            .text
            .contains("«Полная казна — крепость державы»"),
        "{}",
        a.entries[0].text
    );
    assert!(
        a.rulers[0].biography.contains("Полная казна"),
        "{}",
        a.rulers[0].biography
    );
    assert!(a.rulers[1..].iter().any(|r| r.biography.contains("завет")));
}

/// Acceptance: the forged will comes only while legitimacy is low.
#[test]
fn a_forged_will_needs_low_legitimacy() {
    let data = content();
    let e = data
        .sim_events
        .iter()
        .find(|e| e.id == "forged_will")
        .unwrap();
    let mut w = game(&data, 1).world;
    assert!(!w.heirs.is_empty());
    for (legitimacy, fires) in [(90, false), (50, false), (31, false), (29, true), (5, true)] {
        w.axes.insert(ax("legitimacy"), Fx::from_int(legitimacy));
        assert_eq!(e.when.eval(&w), fires, "{legitimacy}");
    }
}

/// The simulation's rulers write wills too: a defiant one names his favourite, the rest
/// mostly leave the throne to the law; the heir named is crowned over the law and told so.
#[test]
fn rulers_of_the_simulation_name_heirs_in_wills() {
    let data = content();
    let e = data
        .sim_events
        .iter()
        .find(|e| e.id == "royal_will")
        .unwrap();
    assert_eq!(
        e.choices[0].effects,
        [Effect::HeirOp(HeirOp::TargetBequeath)]
    );
    let mut g = game(&data, 1);
    let mut picks = |traits: &[&str]| {
        g.world.ruler.traits = traits.iter().map(|t| t.to_string()).collect();
        let auto = AutoChooser::for_ruler(&data, &g.world.ruler);
        (0..50)
            .filter(|_| auto.choose(&mut g, &e.choices) == 0)
            .count()
    };
    assert_eq!(picks(&["defiant"]), 50);
    let some = picks(&[]);
    assert!(0 < some && some < 25, "{some}");
    let crowned = &data.sim.texts.crowned.0;
    let told = |c: &Chronicle| {
        (c.entries.iter())
            .find(|e| &e.title == crowned && e.text.contains("завещани"))
            .map(|e| e.tick)
    };
    // 0..100 since stage 26b: the queue of events moved the rng, a will crowns its favourite
    // as often as before (8 of 300 dynasties against 9), the first now at seed 93. Stage 28:
    // the big map moves the rng again, the first at seed 118: 100..200, as many dynasties.
    let (c, tick) = (100..200)
        .find_map(|s| {
            let c = dynasty(&data, s, None);
            told(&c).map(|t| (c, t))
        })
        .expect("a will in 100 dynasties");
    let r = c.rulers.iter().find(|r| r.start == tick).unwrap();
    assert!(r.designated);
}

/// Acceptance: an order kept `testament_years` after the founder's death counts for the
/// legacy of the score; broken sooner, it does not.
#[test]
fn an_order_kept_a_hundred_years_is_a_legacy() {
    let data = content();
    let rules = bd_core::score::load(&read("score.ron"), &data).unwrap();
    let capital = game(&data, 1).world.capital.province;
    let t = Testament {
        order: Some(Order::KeepProvince(capital)),
        ..Default::default()
    };
    let c = (0..20)
        .map(|s| dynasty(&data, s, Some(t.clone())))
        .find(|c| c.years >= 120)
        .expect("a dynasty of 120 years");
    let legacy = |c: &Chronicle, rules: &bd_core::score::ScoreRules| {
        bd_core::score::compute(c, &[], rules).parts["legacy"]
    };
    let mut none = rules.clone();
    none.testament_years = None;
    assert_eq!(legacy(&c, &rules) - legacy(&c, &none), 100);
    // Broken in its tenth year: no legacy.
    let mut broken = c.clone();
    let w = &mut broken.entries.last_mut().unwrap().snapshot;
    w.testament.as_mut().unwrap().broken = Some(Tick(10));
    assert_eq!(legacy(&broken, &rules), legacy(&c, &none));
}
