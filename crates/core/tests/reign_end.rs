//! Stage 5: end of the reign, death risk, abdication, heirs.

use bd_core::data::{Data, DataError, by_age};
use bd_core::fx::Fx;
use bd_core::game::{DecisionKind, Game, GameError, PendingEvent, Step};
use bd_core::rng::Rng;
use bd_core::rules::{Choice, Ctx, Effect, Event, EventTarget, HeirOp, Predicate, Sign, Target};
use bd_core::state::{AxisId, Heir, HeirStatus, NeighbourId, Preset, World};
use bd_core::time::{Tick, Years};

const RULES: &str = include_str!("../../../data/rules.ron");
const PRESET: &str = include_str!("../../../data/presets/default.ron");
const MAP: &str = include_str!("../../../data/maps/default.ron");
const EVENTS: [&str; 4] = [
    include_str!("../../../data/events/reign.ron"),
    include_str!("../../../data/events/neighbours.ron"),
    include_str!("../../../data/events/death.ron"),
    include_str!("../../../data/events/heirs.ron"),
];
const ACTIONS: &str = include_str!("../../../data/actions.ron");

fn ax(s: &str) -> AxisId {
    AxisId(s.into())
}

fn content() -> Data {
    let mut data = bd_core::data::load(RULES).unwrap();
    EVENTS.iter().for_each(|e| data.add_events(e).unwrap());
    data.add_actions(ACTIONS).unwrap();
    data
}

/// Rules only, no random events, no death risk.
fn bare() -> Data {
    let mut data = bd_core::data::load(RULES).unwrap();
    data.quiet_weight = 0;
    data.death.base = vec![];
    data.death.health_k = Fx(0);
    data
}

fn game(data: Data, seed: u64) -> Game {
    let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
    Game::new(data, &preset, seed)
}

/// The preset has one heir, Конрад (6, ability 50, claim 70); tests of two heirs add
/// Агнесса (5, ability 55, claim 40).
fn two_heirs(w: &mut World) {
    w.heirs.push(Heir {
        name: "Агнесса".into(),
        age: 5,
        ability: Fx::from_int(55),
        claim: Fx::from_int(40),
        status: HeirStatus::Home,
    });
}

fn event(id: &str, effects: Vec<Effect>) -> Event {
    Event {
        id: id.into(),
        title: id.into(),
        text: String::new(),
        when: Predicate::All(vec![]),
        weight: 0,
        once: false,
        cooldown_years: Years(0),
        importance: 1,
        sign: Sign::Bad,
        target: EventTarget::None,
        choices: vec![Choice {
            text: id.into(),
            effects,
            cause_tag: id.into(),
            hint: None,
        }],
    }
}

fn force(g: &mut Game, id: &str, choice: usize) {
    g.pending_event = Some(PendingEvent {
        event_id: id.into(),
        target: None,
    });
    g.choose(choice).unwrap();
}

/// Plays with the middle choice `(len - 1) / 2` of every event, as `cli` neutral does, and no
/// actions; reign length in years.
fn middle_reign(start: &Game, seed: u64) -> u32 {
    let mut g = start.clone();
    g.rng = Rng::from_seed(seed);
    loop {
        match g.wait().unwrap() {
            Step::Event(v) => g.choose((v.choices.len() - 1) / 2).unwrap(),
            Step::Idle => {}
            Step::ReignEnded(end) => return end.tick.year(g.world.time_unit),
        }
        assert!(g.world.tick.0 < 200, "the ruler never dies");
    }
}

#[test]
fn reign_length_calibration() {
    let start = game(content(), 0);
    let mut years: Vec<u32> = (0..1000).map(|s| middle_reign(&start, s)).collect();
    years.sort();
    let early = years.iter().filter(|&&y| y < 10).count();
    let median = years[500];
    eprintln!(
        "early {early}, median {median}, p10 {} p90 {}",
        years[100], years[900]
    );
    assert!(early <= 100, "early deaths {early} of 1000");
    assert!((25..=40).contains(&median), "median reign {median}");
}

#[test]
fn ruler_dies_ends_the_reign() {
    let mut data = bare();
    data.events = vec![event("death", vec![Effect::RulerDies("illness".into())])];
    data.actions = content().actions;
    let mut g = game(data, 1);
    g.wait().unwrap();
    force(&mut g, "death", 0);
    let world = g.world.clone();
    // Calls before `wait` reports the end already fail.
    assert_eq!(
        g.start_action("royal_progress", None),
        Err(GameError::ReignEnded)
    );
    match g.wait().unwrap() {
        Step::ReignEnded(end) => {
            assert_eq!(end.cause, "illness");
            assert_eq!(end.tick, Tick(1));
            assert_eq!(end.world, world);
        }
        step => panic!("{step:?}"),
    }
    assert_eq!(g.wait(), Err(GameError::ReignEnded));
    assert_eq!(g.choose(0), Err(GameError::ReignEnded));
    assert_eq!(
        g.start_action("royal_progress", None),
        Err(GameError::ReignEnded)
    );
    assert_eq!(g.abdicate(), Err(GameError::ReignEnded));
    assert_eq!(g.world, world, "the world is frozen");
}

#[test]
fn action_can_end_the_reign() {
    let mut data = bare();
    let mut a = content().actions.remove(0);
    a.on_complete = vec![Effect::RulerDies("duel".into())];
    a.requires = Predicate::All(vec![]);
    a.min_crown_power = Fx(0);
    data.actions = vec![a.clone()];
    let mut g = game(data, 1);
    g.start_action(&a.id, None).unwrap();
    g.wait().unwrap();
    let Step::ReignEnded(end) = g.wait().unwrap() else {
        panic!()
    };
    assert_eq!((end.cause.as_str(), end.tick), ("duel", Tick(2)));
}

/// Abdicates in a fresh game with this bureaucracy and first-heir ability.
fn abdicate(bureaucracy: i64, ability: i64) -> Game {
    let mut g = game(content(), 1);
    g.world
        .axes
        .insert(ax("bureaucracy"), Fx::from_int(bureaucracy));
    g.world.heirs[0].ability = Fx::from_int(ability);
    g.abdicate().unwrap();
    let Step::Event(v) = g.wait().unwrap() else {
        panic!()
    };
    assert_eq!(v.event_id, "abdication");
    assert_eq!(
        g.world.tick,
        Tick(0),
        "abdication fires without time passing"
    );
    g.choose(0).unwrap();
    g
}

#[test]
fn abdication_with_weak_institutions_breaks_the_claim() {
    let crisis = content().heirs.crisis_claim;
    let mut g = abdicate(20, 80);
    let Step::ReignEnded(end) = g.wait().unwrap() else {
        panic!()
    };
    assert_eq!(end.cause, "abdication");
    let w = &end.world;
    assert!(w.heirs[0].claim < crisis, "{}", w.heirs[0].claim);
    assert!(w.flags.contains("abdicated") && w.flags.contains("succession_contested"));
    // A mature heir alone does not help, nor do institutions alone.
    assert!(abdicate(20, 80).world.heirs[0].claim < crisis);
    assert!(abdicate(90, 30).world.heirs[0].claim < crisis);
}

#[test]
fn abdication_with_strong_institutions_keeps_the_claim() {
    let g = abdicate(90, 80);
    assert_eq!(g.world.heirs[0].claim, Fx::from_int(70));
    assert!(g.world.flags.contains("abdicated"));
    assert!(!g.world.flags.contains("succession_contested"));
    assert!(g.ended.is_some());
}

#[test]
fn abdication_can_be_cancelled() {
    let mut g = game(content(), 1);
    g.abdicate().unwrap();
    assert_eq!(g.abdicate(), Err(GameError::EventPending));
    let last = g.decisions.last().unwrap();
    assert_eq!(
        (&last.kind, last.cause_tag.as_str()),
        (&DecisionKind::Abdicate, "abdication")
    );
    g.choose(1).unwrap();
    assert!(g.ended.is_none());
    assert!(!matches!(g.wait().unwrap(), Step::ReignEnded(_)));
    // Without the event in the data there is nothing to fire.
    assert_eq!(game(bare(), 1).abdicate(), Err(GameError::Unknown));
}

/// Ability of the first heir after `years` in this status, law and births off.
fn grown(status: HeirStatus, years: u32) -> Fx {
    let mut data = bare();
    data.heirs.birth = vec![];
    let mut g = game(data, 1);
    g.world.heirs[0].status = status;
    for _ in 0..years {
        g.wait().unwrap();
    }
    g.world.heirs[0].ability
}

#[test]
fn hostage_grows_slower_than_home() {
    let hostage = HeirStatus::Hostage(NeighbourId("nordmark".into()));
    let (home, away) = (grown(HeirStatus::Home, 5), grown(hostage, 5));
    let studying = grown(HeirStatus::Studying("Монастырь".into()), 5);
    assert!(away < home && home < studying, "{away} {home} {studying}");
    // Heir 0 is 6: growth stops at adult_age 16.
    assert_eq!(grown(HeirStatus::Home, 10), grown(HeirStatus::Home, 20));
    // Grows at ages 7..=15.
    assert_eq!(grown(HeirStatus::Home, 10), Fx::from_int(68));
}

/// Claims of heirs 0 and 1 after `years` under the law flag.
fn claims(law: &str, years: u32, hostage: bool) -> (Fx, Fx) {
    let mut data = bare();
    data.heirs.birth = vec![];
    let mut g = game(data, 1);
    two_heirs(&mut g.world);
    g.world.flags.retain(|f| !f.starts_with("law_"));
    g.world.flags.insert(law.into());
    if hostage {
        g.world.heirs[0].status = HeirStatus::Hostage(NeighbourId("nordmark".into()));
    }
    for _ in 0..years {
        g.wait().unwrap();
    }
    (g.world.heirs[0].claim, g.world.heirs[1].claim)
}

#[test]
fn claims_follow_the_law() {
    // Start: Конрад 70, Агнесса 40; step 2 a year.
    assert_eq!(
        claims("law_primogeniture", 1, false),
        (Fx::from_int(72), Fx::from_int(38))
    );
    assert_eq!(
        claims("law_primogeniture", 30, false),
        (Fx::from_int(75), Fx::from_int(35))
    );
    assert_eq!(
        claims("law_none", 30, false),
        (Fx::from_int(50), Fx::from_int(50))
    );
    // Elective: toward ability * 0.9.
    let (a, b) = claims("law_elective", 1, false);
    assert_eq!((a, b), (Fx::from_int(68), Fx::from_int(42)));
    // No law flag: claims stay.
    assert_eq!(
        claims("no_law", 5, false),
        (Fx::from_int(70), Fx::from_int(40))
    );
    // A hostage loses claim even where the law pulls it up.
    assert!(claims("law_primogeniture", 3, true).0 < Fx::from_int(70));
}

#[test]
fn law_actions_switch_the_flag() {
    let mut data = bare();
    data.actions = content().actions;
    let mut g = game(data, 1);
    g.world.axes.insert(ax("treasury"), Fx::from_int(1000));
    let ids: Vec<_> = g
        .available_actions()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(!ids.contains(&"change_succession_law_primogeniture".to_string()));
    g.start_action("change_succession_law_none", None).unwrap();
    g.wait().unwrap();
    g.wait().unwrap();
    let laws: Vec<_> = g
        .world
        .flags
        .iter()
        .filter(|f| f.starts_with("law_"))
        .collect();
    assert_eq!(laws, ["law_none"]);
}

fn births(married: bool) -> usize {
    let mut data = bare();
    data.heirs.birth = vec![(0, Fx::from_int(100))];
    data.heirs.unmarried = Fx(0);
    let mut g = game(data, 1);
    if !married {
        g.world.flags.remove("married");
    }
    for _ in 0..5 {
        g.wait().unwrap();
    }
    g.world.heirs.len() - 1
}

#[test]
fn births_need_marriage_and_age() {
    assert_eq!(births(true), 5);
    assert_eq!(births(false), 0);
    let table = [(16, Fx::from_int(20)), (40, Fx::from_int(10)), (50, Fx(0))];
    assert_eq!(by_age(&table, 10), Fx(0));
    assert_eq!(by_age(&table, 16), Fx::from_int(20));
    assert_eq!(by_age(&table, 45), Fx::from_int(10));
    assert_eq!(by_age(&table, 90), Fx(0));
}

/// Ticks until the death roll queues an event, with this rule tweak, up to 100.
fn first_death_event(tweak: impl Fn(&mut Data), setup: impl Fn(&mut World)) -> Option<String> {
    let mut data = bare();
    data.events = ["illness", "assassination", "war_wound"]
        .map(|id| {
            let mut e = event(id, vec![]);
            e.cooldown_years = Years(3);
            e
        })
        .to_vec();
    tweak(&mut data);
    let mut g = game(data, 7);
    setup(&mut g.world);
    for _ in 0..100 {
        if let Step::Event(v) = g.wait().unwrap() {
            return Some(v.event_id);
        }
    }
    None
}

#[test]
fn death_roll() {
    let none = |_: &mut World| {};
    // No risk at all: nothing ever happens.
    assert_eq!(first_death_event(|_| {}, none), None);
    // Certain base risk: the illness event, never instant death.
    let certain = |d: &mut Data| d.death.base = vec![(0, Fx::from_int(1000))];
    assert_eq!(first_death_event(certain, none).as_deref(), Some("illness"));
    // Health counts: (100 - 0) * 10 per mille is certain.
    let frail = |d: &mut Data| d.death.health_k = Fx::from_int(10);
    let sick = |w: &mut World| w.ruler.health = Fx(0);
    assert_eq!(first_death_event(frail, sick).as_deref(), Some("illness"));
    let healthy = |w: &mut World| w.ruler.health = Fx::from_int(100);
    assert_eq!(first_death_event(frail, healthy), None);
    // A plot spawns its own event; while illness cools down, its risk is skipped.
    let plot = |d: &mut Data| {
        d.death.base = vec![(0, Fx::from_int(1000))];
        d.death.risks[1].per_mille = Fx::from_int(1000);
    };
    let plotted = |w: &mut World| {
        w.flags.insert("plot".into());
        w.last_fired.insert("illness".into(), Tick(0));
    };
    assert_eq!(
        first_death_event(plot, plotted).as_deref(),
        Some("assassination")
    );
    // War wounds need a war: before stage 4 the risk never applies.
    let war = |d: &mut Data| d.death.risks[0].per_mille = Fx::from_int(1000);
    assert_eq!(first_death_event(war, none), None);
}

#[test]
fn chance_percent() {
    let mut w = game(bare(), 1).world;
    w.axes.insert(ax("loyalty_nobles"), Fx::from_int(50));
    w.flags.insert("guard".into());
    let chance = |text: &str| match ron::from_str::<Effect>(text).unwrap() {
        Effect::Chance(c) => c.percent(&w),
        _ => panic!(),
    };
    let c = r#"Chance((percent: 20, axes: [("loyalty_nobles", 0.6)], bonus: [(Flag("guard"), 25), (Flag("no"), 50)], then: []))"#;
    assert_eq!(chance(c), Fx::from_int(75));
    assert_eq!(
        chance("Chance((percent: 150, then: []))"),
        Fx::from_int(100)
    );
    assert_eq!(chance("Chance((percent: -5, then: []))"), Fx(0));
}

#[test]
fn chance_picks_a_branch() {
    let outcome = |percent: i64| {
        let mut data = bare();
        let chance = format!(
            r#"Chance((percent: {percent}, then: [SetFlag("saved")], otherwise: [RulerDies("illness")]))"#
        );
        data.events = vec![event("e", vec![ron::from_str(&chance).unwrap()])];
        let mut g = game(data, 1);
        force(&mut g, "e", 0);
        (g.world.flags.contains("saved"), g.ended)
    };
    assert_eq!(outcome(100), (true, None));
    assert_eq!(outcome(0), (false, Some("illness".into())));
    // Nested effects are checked on load.
    let bad = r#"[(id: "x", title: "", text: "", when: All([]), weight: 0, once: false,
        cooldown_years: 0, importance: 0, target: None, choices: [(text: "", cause_tag: "x",
        effects: [Chance((percent: 1, then: [Axis("loyalty", 1)]))])])]"#;
    assert!(matches!(bare().add_events(bad), Err(DataError::Invalid(_))));
    let bad = bad.replace(
        r#"then: [Axis("loyalty", 1)]"#,
        r#"axes: [("nothing", 1)], then: []"#,
    );
    assert!(matches!(
        bare().add_events(&bad),
        Err(DataError::Invalid(_))
    ));
}

#[test]
fn heir_predicates() {
    let mut w = game(bare(), 1).world;
    two_heirs(&mut w);
    let p = |w: &World, text: &str| ron::from_str::<Predicate>(text).unwrap().eval(w);
    // Конрад 6 (claim 70), Агнесса 5 (claim 40).
    assert!(
        p(&w, "HeirAge(0, 6, 6)") && !p(&w, "HeirAge(1, 6, 99)") && !p(&w, "HeirAge(2, 0, 99)")
    );
    assert!(p(&w, "ClaimGapBelow(30.001)") && !p(&w, "ClaimGapBelow(30)"));
    w.heirs[1].claim = Fx::from_int(95);
    assert!(p(&w, "ClaimGapBelow(26)"));
    w.heirs.truncate(1);
    assert!(!p(&w, "ClaimGapBelow(100)"));
}

#[test]
fn heir_ops() {
    let data = bare();
    let mut w = game(bare(), 1).world;
    two_heirs(&mut w);
    let mut queue = vec![];
    let mut run = |w: &mut World, text: &str, target: Option<Target>| {
        let e: Effect = ron::from_str(text).unwrap();
        let mut ctx = Ctx {
            data: &data,
            queue: &mut queue,
            target: target.as_ref(),
        };
        e.apply(w, &mut ctx);
    };
    run(&mut w, "HeirOp(Claim(1, 70))", None);
    assert_eq!(w.heirs[1].claim, Fx::from_int(100));
    run(&mut w, "HeirOp(TargetClaim(-80))", Some(Target::Heir(0)));
    assert_eq!(w.heirs[0].claim, Fx(0));
    run(&mut w, "HeirOp(TargetAbility(5))", Some(Target::Heir(1)));
    assert_eq!(w.heirs[1].ability, Fx::from_int(60));
    run(
        &mut w,
        r#"HeirOp(TargetStatus(Hostage("nordmark")))"#,
        Some(Target::Heir(1)),
    );
    assert_eq!(
        w.heirs[1].status,
        HeirStatus::Hostage(NeighbourId("nordmark".into()))
    );
    // No heir target: no-op.
    let before = w.clone();
    run(&mut w, "HeirOp(TargetAbility(5))", None);
    run(
        &mut w,
        "HeirOp(TargetClaim(5))",
        Some(Target::Neighbour(NeighbourId("nordmark".into()))),
    );
    run(&mut w, "HeirOp(Claim(9, 5))", None);
    assert_eq!(w, before);
}

#[test]
fn content_loads() {
    let data = content();
    for id in ["illness", "assassination", "war_wound", "abdication"] {
        assert!(data.events.iter().any(|e| e.id == id), "{id}");
    }
    for id in [
        "heir_tutor",
        "heir_marriage",
        "heir_dispute",
        "heir_death",
        "succession_dispute",
    ] {
        assert!(data.events.iter().any(|e| e.id == id), "{id}");
    }
    let w = game(data.clone(), 1).world;
    assert!(w.flags.contains("law_primogeniture") && w.flags.contains("married"));
    // Flags come from the preset; without the field the world starts with none.
    let bare = PRESET.replace("flags: [\"law_primogeniture\", \"married\"],", "");
    assert_ne!(bare, PRESET);
    let preset = Preset::load_with_map(&bare, MAP, &data).unwrap();
    assert!(World::from_preset(&data, &preset).flags.is_empty());
    // A bad death risk predicate is rejected.
    let bad = RULES.replacen("when: Flag(\"plot\")", "when: AxisAbove(\"nothing\", 1)", 1);
    assert!(matches!(
        bd_core::data::load(&bad),
        Err(DataError::Invalid(_))
    ));
    let bad = RULES.replacen("(\"bureaucracy\", 60)", "(\"nothing\", 60)", 1);
    assert!(matches!(
        bd_core::data::load(&bad),
        Err(DataError::Invalid(_))
    ));
}

#[test]
fn war_wound_waits_for_war() {
    let mut g = game(content(), 1);
    g.queue.push((Tick(1), "war_wound".into()));
    g.world.axes.insert(ax("treasury"), Fx(0)); // keeps the festival out
    for _ in 0..3 {
        if let Step::Event(v) = g.wait().unwrap() {
            assert_ne!(v.event_id, "war_wound");
            g.choose(0).unwrap();
        }
    }
}

#[test]
fn death_events_are_deterministic() {
    let start = game(content(), 0);
    let a: Vec<u32> = (0..20).map(|s| middle_reign(&start, s)).collect();
    let b: Vec<u32> = (0..20).map(|s| middle_reign(&start, s)).collect();
    assert_eq!(a, b);
    assert!(a.iter().any(|&y| y != a[0]));
}

#[test]
fn death_goes_before_other_events() {
    let mut data = bare();
    data.death.base = vec![(0, Fx::from_int(1000))];
    data.events = vec![event("illness", vec![]), event("other", vec![])];
    let mut g = game(data, 1);
    let other = PendingEvent {
        event_id: "other".into(),
        target: None,
    };
    let nordmark = NeighbourId("nordmark".into());
    g.neighbour_events = vec![(nordmark.clone(), other.clone()), (nordmark, other)];
    g.queue.push((Tick(1), "other".into()));
    let Step::Event(v) = g.wait().unwrap() else {
        panic!()
    };
    assert_eq!(v.event_id, "illness");
}

/// Fires `e` from the random pool in a game where it is the only event.
fn fire(mut e: Event, setup: impl Fn(&mut World)) -> (Game, Option<bd_core::game::EventView>) {
    let mut data = bare();
    e.weight = 1;
    data.events = vec![e];
    data.heirs.birth = vec![];
    let mut g = game(data, 3);
    setup(&mut g.world);
    match g.wait().unwrap() {
        Step::Event(v) => (g, Some(v)),
        _ => (g, None),
    }
}

#[test]
fn event_targets_an_heir() {
    let mut e = event("sick", vec![Effect::HeirOp(HeirOp::TargetRemove)]);
    e.target = EventTarget::Heir(6, 12);
    e.text = "{heir} болен".into();
    // Конрад 6 -> 7 after the first tick, Агнесса 5 -> 6: both fit, the pick is random.
    let (mut g, v) = fire(e.clone(), two_heirs);
    let v = v.unwrap();
    let Some(Target::Heir(i)) = v.target else {
        panic!("{v:?}")
    };
    let name = g.world.heirs[i as usize].name.clone();
    assert_eq!(v.text, format!("{name} болен"));
    g.choose(0).unwrap();
    assert_eq!(g.world.heirs.len(), 1);
    assert_ne!(g.world.heirs[0].name, name);
    // Only Агнесса fits: always her.
    let (_, v) = fire(e.clone(), |w| {
        two_heirs(w);
        w.heirs[0].age = 20;
    });
    assert_eq!(v.unwrap().target, Some(Target::Heir(1)));
    // Nobody of that age: the event cannot fire.
    let (_, v) = fire(e, |w| w.heirs.iter_mut().for_each(|h| h.age = 30));
    assert_eq!(v, None);
}

/// Every neighbour hostile, stronger than the border and raiding every year.
fn raided() -> Game {
    let mut data = bare();
    data.add_events(EVENTS[1]).unwrap();
    data.neighbour_ai.expand.events = vec![("neighbour_raid".into(), 100)];
    let spawn = Effect::SpawnEvent("next".into(), Years(2));
    data.events
        .extend([event("first", vec![spawn]), event("next", vec![])]);
    let mut g = game(data, 1);
    for n in g.world.neighbours.values_mut() {
        (n.relation, n.strength) = (Fx::from_int(-80), Fx::from_int(1000));
    }
    g
}

#[test]
fn deferred_events_beat_neighbours() {
    let mut g = raided();
    force(&mut g, "first", 0);
    let mut fired = vec![];
    for _ in 0..3 {
        let Step::Event(v) = g.wait().unwrap() else {
            panic!()
        };
        fired.push((g.world.tick.0, v.event_id));
        g.choose(0).unwrap();
    }
    assert_eq!(fired[1], (2, "next".to_string()));
    assert_eq!(fired[0].1, "neighbour_raid");
}

#[test]
fn one_waiting_event_per_neighbour() {
    let mut g = raided();
    for _ in 0..10 {
        if let Step::Event(_) = g.wait().unwrap() {
            g.choose(0).unwrap();
        }
    }
    let mut ids: Vec<_> = g.neighbour_events.iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(ids.len(), 2, "three raids a year, one fired a tick");
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 2);
}
