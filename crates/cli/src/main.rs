//! Dev runner: one game from a seed and a script or strategy, and its replay from a journal.

use bd_core::game::{Decision, DecisionKind, Game, ReignEnd, Step};
use bd_core::rules::Target;
use bd_core::score::{self, ScoreRules};
use bd_core::fx::Fx;
use bd_core::sim::{self, AutoChooser, FallReason};
use bd_core::state::Preset;
use bd_core::time::{Tick, Years};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Plays a reign by a script or a strategy and prints the decision journal; once the
    /// reign has ended, the chronicle of the dynasty and its score.
    Run {
        #[arg(long)]
        seed: u64,
        #[command(flatten)]
        files: Files,
        /// A list of `ScriptStep` in RON, see data/scripts/.
        #[arg(long, conflicts_with = "strategy")]
        script: Option<PathBuf>,
        /// `neutral` or a strategy of data/strategies.ron.
        #[arg(long, default_value = NEUTRAL)]
        strategy: String,
        #[arg(long)]
        json: bool,
    },
    /// Replays the journal from `run --json` and prints the final World hash.
    Replay {
        #[arg(long)]
        seed: u64,
        #[command(flatten)]
        files: Files,
        #[arg(long)]
        decisions: PathBuf,
    },
    /// Plays seeds seed_start..seed_start+runs by a strategy, each reign and its dynasty to
    /// the end; prints a CSV row per game, then a summary as `#` lines.
    Batch {
        #[arg(long)]
        runs: u64,
        #[arg(long, default_value = NEUTRAL)]
        strategy: String,
        #[arg(long, default_value_t = 0)]
        seed_start: u64,
        #[command(flatten)]
        files: Files,
    },
    /// Plays a script, then the neutral strategy until the reign ends, and prints for every
    /// chronicle entry the chain decision -> mark -> event.
    Trace {
        #[arg(long)]
        seed: u64,
        #[command(flatten)]
        files: Files,
        #[arg(long)]
        script: PathBuf,
    },
}

#[derive(clap::Args)]
struct Files {
    #[arg(long, default_value = "data/presets/default.ron")]
    preset: PathBuf,
    #[arg(long, default_value = "data/maps/default.ron")]
    map: PathBuf,
    /// Holds rules.ron, actions.ron, names.ron, hints.ron, score.ron, events/*.ron and
    /// events/sim/*.ron.
    #[arg(long, default_value = "data")]
    data: PathBuf,
}

/// The middle choice of every event, no actions; every other strategy is a set of
/// `AutoChooser` weights in data/strategies.ron.
const NEUTRAL: &str = "neutral";

/// `AutoChooser` weights added to the founder's own (`AutoChooser::for_ruler`).
type Strategies = BTreeMap<String, BTreeMap<String, Fx>>;

#[derive(Deserialize, Debug, PartialEq)]
enum ScriptStep {
    /// Up to n ticks; stops early when an event fires.
    Wait(u32),
    Action(String, Option<Target>),
    Choose(usize),
    /// The first choice with this cause tag, else choice 0.
    ChooseByTag(String),
    Abdicate,
}

/// What `replay` reads from the `run --json` output.
#[derive(Deserialize)]
struct Journal {
    decisions: Vec<Decision>,
    tick: Tick,
}

/// A safety cap for the neutral strategy; the ruler dies long before.
const MAX_YEARS: Years = Years(150);

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("ошибка: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.cmd {
        Cmd::Run {
            seed,
            files,
            script,
            strategy,
            json,
        } => {
            let mut g = load(&files, seed)?;
            let rules = score_rules(&files, &g)?;
            match script {
                Some(path) => play_script(&mut g, &parse(&read(&path)?)?, false)?,
                None => {
                    let auto = chooser(&files, &g, &strategy)?;
                    play(&mut g, auto.as_ref())?
                }
            }
            let (chronicle, score) = dynasty(&g, &rules);
            let (w, hash) = (&g.world, world_hash(&g));
            let date = w.tick.date(w.time_unit, w.start_year);
            if json {
                let out = serde_json::json!({
                    "seed": seed,
                    "tick": w.tick,
                    "date": date,
                    "world_hash": hash,
                    "reign_end": g.ended,
                    "decisions": g.decisions,
                    "chronicle": chronicle.as_ref().map(chronicle_json),
                    "score": score,
                });
                println!("{out:#}");
                return Ok(());
            }
            println!("Seed {seed}, начало {}", w.start_year);
            println!("Решения:");
            for d in &g.decisions {
                println!("  {}", describe(&g, d));
            }
            println!("Итог: {date} (тик {}), хэш мира {hash}", w.tick.0);
            match &g.ended {
                Some(cause) => println!("Конец правления: {cause}"),
                None => println!("Правление продолжается"),
            }
            match (&chronicle, &score) {
                (Some(c), Some(s)) => print_dynasty(&g, c, s),
                _ => println!("Хроника: недоступно (правление продолжается)"),
            }
            Ok(())
        }
        Cmd::Replay {
            seed,
            files,
            decisions,
        } => {
            let mut g = load(&files, seed)?;
            let text = read(&decisions)?;
            let j: Journal = serde_json::from_str(&text).map_err(|e| format!("журнал: {e}"))?;
            replay(&mut g, &j.decisions, j.tick)?;
            println!("{}", world_hash(&g));
            Ok(())
        }
        Cmd::Batch {
            runs,
            strategy,
            seed_start,
            files,
        } => {
            let start = load(&files, 0)?;
            let rules = score_rules(&files, &start)?;
            let auto = chooser(&files, &start, &strategy)?;
            let rows = (seed_start..seed_start + runs)
                .map(|seed| batch_row(&start, seed, auto.as_ref(), &rules))
                .collect::<Result<Vec<_>, _>>()?;
            print!("{}", batch_report(&rows));
            Ok(())
        }
        Cmd::Trace {
            seed,
            files,
            script,
        } => {
            let mut g = load(&files, seed)?;
            let rules = score_rules(&files, &g)?;
            play_script(&mut g, &parse(&read(&script)?)?, true)?;
            play(&mut g, None)?;
            let (Some(c), _) = dynasty(&g, &rules) else {
                return Err("правление не кончилось".into());
            };
            print!("{}", trace(&g, &c));
            Ok(())
        }
    }
}

fn score_rules(f: &Files, g: &Game) -> Result<ScoreRules, String> {
    let text = read(&f.data.join("score.ron"))?;
    score::load(&text, &g.data).map_err(|e| format!("score.ron: {e:?}"))
}

/// None for `neutral`; otherwise the founder's `AutoChooser` plus the strategy's weights.
fn chooser(f: &Files, g: &Game, name: &str) -> Result<Option<AutoChooser>, String> {
    if name == NEUTRAL {
        return Ok(None);
    }
    let all: Strategies = ron::from_str(&read(&f.data.join("strategies.ron"))?)
        .map_err(|e| format!("strategies.ron: {e}"))?;
    let extra = all.get(name).ok_or(format!("нет стратегии {name}"))?;
    let mut auto = AutoChooser::for_ruler(&g.data, &g.world.ruler);
    for (k, v) in extra {
        let w = auto.weights.entry(k.clone()).or_default();
        *w = *w + *v;
    }
    Ok(Some(auto))
}

/// One game of `batch`: (seed, reign years, dynasty years, score, fall).
type Row = (u64, u32, u32, i64, FallReason);

/// Years before which the founder's death counts as early (DESIGN 4.3).
const EARLY_YEARS: u32 = 10;

fn batch_row(
    start: &Game,
    seed: u64,
    auto: Option<&AutoChooser>,
    rules: &ScoreRules,
) -> Result<Row, String> {
    let mut g = Game {
        rng: bd_core::rng::Rng::from_seed(seed),
        ..start.clone()
    };
    play(&mut g, auto)?;
    let reign = g.world.tick.year(g.world.time_unit);
    let (Some(c), Some(s)) = dynasty(&g, rules) else {
        return Err(format!("seed {seed}: правитель жив через {} лет", MAX_YEARS.0));
    };
    Ok((seed, reign, c.years, s.total, c.fall))
}

/// The CSV, then `#` lines: quartiles of the dynasty years, score and reign years, the share
/// of early deaths, the fall reasons by frequency.
fn batch_report(rows: &[Row]) -> String {
    let mut out = String::from("seed,reign_years,dynasty_years,score,fall_reason,early_death\n");
    for (seed, reign, years, score, fall) in rows {
        let early = *reign < EARLY_YEARS;
        out += &format!("{seed},{reign},{years},{score},{fall:?},{early}\n");
    }
    let quartiles = |mut v: Vec<i64>| {
        v.sort();
        let at = |q: usize| v.get(v.len() * q / 4).copied().unwrap_or(0);
        format!("{} / {} / {}", at(1), at(2), at(3))
    };
    out += &format!("# runs {}\n# квартили (25 / 50 / 75%):\n", rows.len());
    out += &format!("#   лет династии {}\n", quartiles(rows.iter().map(|r| r.2 as i64).collect()));
    out += &format!("#   счёт {}\n", quartiles(rows.iter().map(|r| r.3).collect()));
    out += &format!("#   лет правления {}\n", quartiles(rows.iter().map(|r| r.1 as i64).collect()));
    let early = rows.iter().filter(|r| r.1 < EARLY_YEARS).count();
    out += &format!("# ранняя смерть {}%\n# причины падения:\n", percent(early, rows.len()));
    let mut falls: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *falls.entry(format!("{:?}", r.4)).or_default() += 1;
    }
    let mut falls: Vec<_> = falls.into_iter().collect();
    falls.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (fall, n) in falls {
        out += &format!("#   {fall} {}%\n", percent(n, rows.len()));
    }
    out
}

fn percent(n: usize, of: usize) -> usize {
    (n * 100).checked_div(of).unwrap_or(0)
}

/// Every chronicle entry, and under it each decision behind it: when, its tag and target,
/// the weight of its marks left at the entry, the event.
fn trace(g: &Game, c: &sim::Chronicle) -> String {
    let w = &g.world;
    let mut out = String::new();
    for e in &c.entries {
        let date = e.tick.date(w.time_unit, w.start_year);
        let event = e.event.as_deref().unwrap_or("-");
        out += &format!("{date} {} [{event}]\n", e.title);
        if e.causes.is_empty() {
            out += "  без решений основателя\n";
        }
        for t in &e.causes {
            let d = &g.decisions[t.decision_idx];
            let target = match &d.kind {
                DecisionKind::EventChoice { target, .. }
                | DecisionKind::ActionStarted { target, .. } => target_name(target),
                DecisionKind::Abdicate => String::new(),
            };
            out += &format!(
                "  решение #{} (тик {}, {}{target}) → метка ({}) → {event}\n",
                t.decision_idx, d.tick.0, t.cause_tag, t.weight
            );
        }
    }
    out
}

fn target_name(t: &Option<Target>) -> String {
    match t {
        Some(Target::Province(id)) => format!(", {}", id.0),
        Some(Target::Neighbour(id)) => format!(", {}", id.0),
        Some(Target::Heir(i)) => format!(", наследник {i}"),
        None => String::new(),
    }
}

fn read(p: &Path) -> Result<String, String> {
    fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

/// RON as in the data files: `Province("capital")` for `Some(Province("capital"))`.
fn parse(text: &str) -> Result<Vec<ScriptStep>, String> {
    let options =
        ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    options.from_str(text).map_err(|e| format!("скрипт: {e}"))
}

fn load(f: &Files, seed: u64) -> Result<Game, String> {
    let dir = &f.data;
    let rules = read(&dir.join("rules.ron"))?;
    let mut data = bd_core::data::load(&rules).map_err(|e| format!("rules.ron: {e:?}"))?;
    for p in ron_files(&dir.join("events"))? {
        let res = data.add_events(&read(&p)?);
        res.map_err(|e| format!("{}: {e:?}", p.display()))?;
    }
    for p in ron_files(&dir.join("events/sim"))? {
        let res = data.add_sim_events(&read(&p)?);
        res.map_err(|e| format!("{}: {e:?}", p.display()))?;
    }
    let res = data.add_hints(&read(&dir.join("hints.ron"))?);
    res.map_err(|e| format!("hints.ron: {e:?}"))?;
    let res = data.add_actions(&read(&dir.join("actions.ron"))?);
    res.map_err(|e| format!("actions.ron: {e:?}"))?;
    let res = data.add_names(&read(&dir.join("names.ron"))?);
    res.map_err(|e| format!("names.ron: {e:?}"))?;
    let preset = Preset::load_with_map(&read(&f.preset)?, &read(&f.map)?, &data);
    let preset = preset.map_err(|e| format!("пресет: {e:?}"))?;
    Ok(Game::new(data, &preset, seed))
}

/// The `.ron` files of a directory, sorted: read_dir order is up to the OS.
fn ron_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let files = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files: Vec<PathBuf> = files.filter_map(|e| Some(e.ok()?.path())).collect();
    files.retain(|p| p.extension().is_some_and(|x| x == "ron"));
    files.sort();
    Ok(files)
}

/// The dynasty after an ended reign (the simulation goes on with the game's rng) and its
/// score; None while the reign goes on.
fn dynasty(g: &Game, rules: &ScoreRules) -> (Option<sim::Chronicle>, Option<score::Score>) {
    let Some(cause) = g.ended.clone() else {
        return (None, None);
    };
    let end = ReignEnd {
        cause,
        tick: g.world.tick,
        world: g.world.snapshot(),
    };
    let c = sim::run(end, &g.data, g.rng.clone());
    let s = score::compute(&c, &g.decisions, rules);
    (Some(c), Some(s))
}

/// The chronicle without the entry snapshots: JSON maps need string keys, and `World.marks`
/// has none.
fn chronicle_json(c: &sim::Chronicle) -> serde_json::Value {
    let entries = c.entries.iter().map(|e| {
        serde_json::json!({
            "tick": e.tick,
            "event": e.event,
            "title": e.title,
            "text": e.text,
            "hint": e.hint,
            "importance": e.importance,
            "causes": e.causes,
        })
    });
    serde_json::json!({
        "entries": entries.collect::<Vec<_>>(),
        "fall": c.fall,
        "years": c.years,
        "rulers": c.rulers,
    })
}

fn print_dynasty(g: &Game, c: &sim::Chronicle, s: &score::Score) {
    let w = &g.world;
    println!("Хроника:");
    for e in &c.entries {
        let date = e.tick.date(w.time_unit, w.start_year);
        let hint = e.hint.as_deref().map_or(String::new(), |h| format!(" {h}"));
        println!("  {date} {}. {}{hint}", e.title, e.text);
    }
    println!(
        "Династия: {} лет, правителей {}, конец {:?}",
        c.years,
        c.rulers.len(),
        c.fall
    );
    println!("Счёт: {}", s.total);
    for (part, points) in &s.parts {
        println!("  {part}: {points}");
    }
    println!("Решающие решения:");
    for d in &s.decisive {
        println!("  {} ({})", describe(g, &d.decision), d.weight);
    }
}

/// `soft`: a Wait while an event waits takes its middle choice instead of failing.
fn play_script(g: &mut Game, script: &[ScriptStep], soft: bool) -> Result<(), String> {
    for (i, step) in script.iter().enumerate() {
        // The rest of the script has no reign to act in.
        if g.ended.is_some() {
            break;
        }
        let res = match step {
            ScriptStep::Wait(_) if g.pending_event.is_some() && !soft => {
                Err("событие ждёт выбора".into())
            }
            ScriptStep::Wait(n) => wait(g, *n, soft),
            ScriptStep::Action(id, target) => g.start_action(id, target.clone()).map_err(err),
            ScriptStep::Choose(idx) => g.choose(*idx).map_err(err),
            ScriptStep::ChooseByTag(tag) => match pending_choices(g) {
                Some(choices) => {
                    let idx = choices.iter().position(|c| c.cause_tag == *tag);
                    g.choose(idx.unwrap_or(0)).map_err(err)
                }
                None => Err("нет события".into()),
            },
            ScriptStep::Abdicate => g.abdicate().map_err(err),
        };
        res.map_err(|e| format!("шаг {} {step:?}: {e}", i + 1))?;
    }
    Ok(())
}

/// Up to n ticks, stopping at an event; `soft` takes the middle choice of a waiting event
/// first and stops at none.
fn wait(g: &mut Game, n: u32, soft: bool) -> Result<(), String> {
    for _ in 0..n {
        if soft && let Some(choices) = pending_choices(g) {
            g.choose((choices.len() - 1) / 2).map_err(err)?;
        }
        match g.wait().map_err(err)? {
            Step::Idle => {}
            Step::Event(_) if soft => {}
            Step::Event(_) | Step::ReignEnded(_) => break,
        }
    }
    Ok(())
}

/// Until the reign ends (at most `MAX_YEARS`): `auto` starts its best action before every tick
/// and makes every choice; without it (`neutral`) no actions and the middle choice.
fn play(g: &mut Game, auto: Option<&AutoChooser>) -> Result<(), String> {
    let end = MAX_YEARS.ticks(g.world.time_unit);
    while g.world.tick < end && g.ended.is_none() {
        if let Some(a) = auto
            && g.pending_event.is_none()
            && let Some((id, target)) = a.action(g)
        {
            g.start_action(&id, target).map_err(err)?;
        }
        match g.wait().map_err(err)? {
            Step::Idle => {}
            Step::Event(v) => {
                let idx = match auto {
                    Some(a) => a.choose(g, &v.choices),
                    None => (v.choices.len() - 1) / 2,
                };
                g.choose(idx).map_err(err)?
            }
            Step::ReignEnded(_) => break,
        }
    }
    Ok(())
}

/// Repeats the decisions at their ticks, then waits until `end`.
fn replay(g: &mut Game, decisions: &[Decision], end: Tick) -> Result<(), String> {
    for d in decisions {
        advance(g, d.tick)?;
        let res = match &d.kind {
            DecisionKind::ActionStarted { action_id, target } => {
                g.start_action(action_id, target.clone()).map_err(err)
            }
            DecisionKind::Abdicate => g.abdicate().map_err(err),
            DecisionKind::EventChoice {
                event_id,
                choice_idx,
                ..
            } => match &g.pending_event {
                Some(p) if p.event_id == *event_id => g.choose(*choice_idx).map_err(err),
                p => Err(format!("ждали событие {event_id}, а есть {p:?}")),
            },
        };
        res.map_err(|e| format!("журнал не совпадает на тике {}: {e}", d.tick.0))?;
    }
    advance(g, end)
}

/// An event before `tick` means the journal skipped a choice.
fn advance(g: &mut Game, tick: Tick) -> Result<(), String> {
    while g.world.tick < tick {
        if let Some(p) = &g.pending_event {
            let (id, now) = (&p.event_id, g.world.tick.0);
            return Err(format!(
                "журнал не совпадает: событие {id} на тике {now} без выбора"
            ));
        }
        g.wait().map_err(err)?;
    }
    Ok(())
}

fn pending_choices(g: &Game) -> Option<&[bd_core::rules::Choice]> {
    let p = g.pending_event.as_ref()?;
    let e = g.data.events.iter().find(|e| e.id == p.event_id)?;
    Some(&e.choices)
}

fn err(e: bd_core::game::GameError) -> String {
    format!("{e:?}")
}

/// FNV-1a over the Debug dump: maps are BTreeMaps and `Fx` prints as an integer, so the
/// text, and the hash, depend only on the World.
fn world_hash(g: &Game) -> String {
    let text = format!("{:?}", g.world);
    let hash = (text.bytes()).fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

fn describe(g: &Game, d: &Decision) -> String {
    let date = d.tick.date(g.world.time_unit, g.world.start_year);
    let target = |t: &Option<Target>| target_name(t).replacen(",", " →", 1);
    let what = match &d.kind {
        DecisionKind::EventChoice {
            event_id,
            choice_idx,
            target: t,
        } => format!("событие {event_id}{}: вариант {choice_idx}", target(t)),
        DecisionKind::ActionStarted {
            action_id,
            target: t,
        } => format!("действие {action_id}{}", target(t)),
        DecisionKind::Abdicate => "отречение".into(),
    };
    format!("{date} {what} [{}]", d.cause_tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = include_str!("../../../data/rules.ron");
    const PRESET: &str = include_str!("../../../data/presets/default.ron");
    const MAP: &str = include_str!("../../../data/maps/default.ron");

    /// An event every tick, with tags "a", "b", "c"; one free action "act".
    fn game() -> Game {
        let mut data = bd_core::data::load(RULES).unwrap();
        data.quiet_weight = 0;
        let choice = |t: &str| format!("(text: \"{t}\", effects: [], cause_tag: \"{t}\")");
        let choices = ["a", "b", "c"].map(choice).join(", ");
        data.add_events(&format!(
            "[(id: \"e\", title: \"\", text: \"\", when: All([]), weight: 1, once: false, \
             cooldown_years: 0, importance: 1, target: None, choices: [{choices}])]"
        ))
        .unwrap();
        data.add_actions(
            "[(id: \"act\", name: \"\", duration_years: 1, cost: 0, requires: All([]), \
             min_crown_power: 0, target: None, on_complete: [], cause_tag: \"act\")]",
        )
        .unwrap();
        let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
        Game::new(data, &preset, 7)
    }

    fn tags(g: &Game) -> Vec<&str> {
        g.decisions.iter().map(|d| d.cause_tag.as_str()).collect()
    }

    fn script(text: &str) -> Vec<ScriptStep> {
        parse(text).unwrap()
    }

    #[test]
    fn script_steps() {
        let mut g = game();
        let s = script(
            "[Action(\"act\", None), Wait(5), ChooseByTag(\"c\"), Wait(1), ChooseByTag(\"x\"), \
             Wait(1), Choose(1)]",
        );
        play_script(&mut g, &s, false).unwrap();
        // Each Wait stopped at the event of its first tick.
        assert_eq!(g.world.tick, Tick(3));
        assert_eq!(tags(&g), ["act", "c", "a", "b"]);
        assert_eq!(
            script("[Action(\"x\", Province(\"capital\"))]"),
            [ScriptStep::Action(
                "x".into(),
                Some(Target::Province(bd_core::state::ProvinceId(
                    "capital".into()
                )))
            )]
        );
    }

    #[test]
    fn script_errors() {
        let fails = |text: &str| play_script(&mut game(), &script(text), false).unwrap_err();
        assert!(fails("[Choose(0)]").contains("NoEvent"));
        assert!(fails("[ChooseByTag(\"a\")]").contains("нет события"));
        assert!(fails("[Wait(1), Wait(1)]").contains("событие ждёт выбора"));
        assert!(fails("[Wait(1), Choose(3)]").contains("BadChoice"));
        assert!(fails("[Action(\"nope\", None)]").contains("Unknown"));
        assert!(fails("[Wait(1), Abdicate]").contains("EventPending"));
        assert!(parse("[Jump]").is_err());
    }

    #[test]
    fn soft_wait_takes_the_middle_and_goes_on() {
        let mut g = game();
        play_script(&mut g, &script("[Wait(5), Wait(1)]"), true).unwrap();
        // An event every tick: each one waiting when the next tick comes takes the middle.
        assert_eq!(g.world.tick, Tick(6));
        assert_eq!(tags(&g), ["b"; 5]);
        assert!(g.pending_event.is_some());
    }

    #[test]
    fn batch_summary() {
        let row = |seed, reign, score, fall| (seed, reign, 100, score, fall);
        let rows = [
            row(0, 5, 10, FallReason::NoHeir),
            row(1, 30, 30, FallReason::Usurped),
            row(2, 40, 20, FallReason::Usurped),
            row(3, 20, 40, FallReason::Alive),
        ];
        let out = batch_report(&rows);
        let mut lines = out.lines();
        assert_eq!(lines.next(), Some("seed,reign_years,dynasty_years,score,fall_reason,early_death"));
        assert_eq!(lines.next(), Some("0,5,100,10,NoHeir,true"));
        let summary: Vec<_> = lines.skip(3).collect();
        assert_eq!(
            summary,
            [
                "# runs 4",
                "# квартили (25 / 50 / 75%):",
                "#   лет династии 100 / 100 / 100",
                "#   счёт 20 / 30 / 40",
                "#   лет правления 20 / 30 / 40",
                "# ранняя смерть 25%",
                "# причины падения:",
                "#   Usurped 50%",
                "#   Alive 25%",
                "#   NoHeir 25%",
            ]
        );
    }

    #[test]
    fn neutral_takes_the_middle_and_no_actions() {
        let mut g = game();
        play(&mut g, None).unwrap();
        assert_eq!(g.world.tick, MAX_YEARS.ticks(g.world.time_unit));
        assert_eq!(g.decisions.len(), MAX_YEARS.0 as usize);
        assert!(tags(&g).iter().all(|t| *t == "b"));
    }

    #[test]
    fn replay_matches_the_game() {
        let mut g = game();
        let s = script("[Action(\"act\", None), Wait(1), Choose(2), Wait(1), Choose(0), Wait(1)]");
        play_script(&mut g, &s, false).unwrap();
        let mut r = game();
        replay(&mut r, &g.decisions, g.world.tick).unwrap();
        assert_eq!(r.world, g.world);
        assert_eq!(r.decisions, g.decisions);
        assert_eq!(world_hash(&r), world_hash(&g));
        assert_ne!(world_hash(&r), world_hash(&game()));

        // A choice missing from the journal, or one for another event, is a mismatch.
        let e = replay(&mut game(), &g.decisions[2..], g.world.tick).unwrap_err();
        assert!(e.contains("без выбора"), "{e}");
        let mut wrong = g.decisions.clone();
        if let DecisionKind::EventChoice { event_id, .. } = &mut wrong[1].kind {
            *event_id = "other".into();
        }
        let e = replay(&mut game(), &wrong, g.world.tick).unwrap_err();
        assert!(e.contains("ждали событие other"), "{e}");
    }

    #[test]
    fn the_dynasty_follows_an_ended_reign() {
        let mut g = game();
        let rules = score::load(include_str!("../../../data/score.ron"), &g.data).unwrap();
        assert_eq!(dynasty(&g, &rules), (None, None));
        g.ended = Some("illness".into());
        let (c, s) = dynasty(&g, &rules);
        let c = c.unwrap();
        assert!(!c.entries.is_empty());
        assert_eq!(s.unwrap(), score::compute(&c, &g.decisions, &rules));
    }
}
