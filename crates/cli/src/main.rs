//! Dev runner: one game from a seed and a script or strategy, and its replay from a journal.

use bd_core::batch::{self, NEUTRAL, chooser, dynasty, parse, play, play_script, score_rules};
use bd_core::batch::{target_name, world_hash};
use bd_core::fx::Fx;
use bd_core::game::{Decision, DecisionKind, Game};
use bd_core::link::replay;
use bd_core::rules::Target;
use bd_core::score;
use bd_core::sim;
use bd_core::time::Tick;
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
        /// Played first, as by `trace`; the strategy goes on from where it ends.
        #[arg(long)]
        script: Option<PathBuf>,
        #[arg(long, default_value_t = 0)]
        seed_start: u64,
        /// A law of rules.ron `laws` in force from the start, in place of its group's.
        #[arg(long)]
        law: Option<String>,
        #[command(flatten)]
        files: Files,
    },
    /// Plays a script, then the neutral strategy until the reign ends, and prints for every
    /// chronicle entry the chain decision -> mark -> event; with `--node`, instead, every
    /// simulated year of that axis: its value, its target and what each edge adds to it.
    Trace {
        #[arg(long)]
        seed: u64,
        #[command(flatten)]
        files: Files,
        #[arg(long)]
        script: PathBuf,
        #[arg(long)]
        node: Option<String>,
        /// A foreign kingdom by its id (`nordmark`): its chronicle instead of ours.
        #[arg(long)]
        realm: Option<String>,
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

/// What `replay` reads from the `run --json` output.
#[derive(Deserialize)]
struct Journal {
    decisions: Vec<Decision>,
    tick: Tick,
}

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
            let texts = read_files(&files)?;
            let mut g = load(&files, &texts, seed)?;
            let rules = score_rules(&texts, &g)?;
            match script {
                Some(path) => play_script(&mut g, &parse(&read(&path)?)?, false, &mut vec![])?,
                None => {
                    let auto = chooser(&texts, &g, &strategy)?;
                    play(&mut g, auto.as_ref(), &mut vec![])?
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
            let mut g = load(&files, &read_files(&files)?, seed)?;
            let text = read(&decisions)?;
            let j: Journal = serde_json::from_str(&text).map_err(|e| format!("журнал: {e}"))?;
            replay(&mut g, &j.decisions, j.tick)?;
            println!("{}", world_hash(&g));
            Ok(())
        }
        Cmd::Batch {
            runs,
            strategy,
            script,
            seed_start,
            law,
            files,
        } => {
            let texts = read_files(&files)?;
            let mut start = load(&files, &texts, 0)?;
            if let Some(law) = law {
                batch::set_law(&mut start, &law)?;
            }
            let rules = score_rules(&texts, &start)?;
            let auto = chooser(&texts, &start, &strategy)?;
            let script = match script {
                Some(path) => parse(&read(&path)?)?,
                None => vec![],
            };
            let seeds = seed_start..seed_start + runs;
            let (out, _) = batch::batch(&start, seeds, &script, auto.as_ref(), &rules, |_| {})?;
            print!("{out}");
            Ok(())
        }
        Cmd::Trace {
            seed,
            files,
            script,
            node,
            realm,
        } => {
            let texts = read_files(&files)?;
            let g = load(&files, &texts, seed)?;
            let rules = score_rules(&texts, &g)?;
            let script = parse(&read(&script)?)?;
            let (node, realm) = (node.as_deref(), realm.as_deref());
            print!("{}", batch::trace_of(g, &rules, &script, node, realm)?);
            Ok(())
        }
    }
}

fn read(p: &Path) -> Result<String, String> {
    fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

/// The `.ron` files of `data`, `data/events` and `data/events/sim` by their path under
/// `data`, the preset and the map by theirs.
fn read_files(f: &Files) -> Result<batch::Files, String> {
    let mut files = batch::Files::new();
    for dir in ["", "events", "events/sim"] {
        let dir = f.data.join(dir);
        let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for p in entries.filter_map(|e| Some(e.ok()?.path())) {
            if p.is_file() && p.extension().is_some_and(|x| x == "ron") {
                let key = p.strip_prefix(&f.data).expect("under data");
                files.insert(key.to_string_lossy().replace('\\', "/"), read(&p)?);
            }
        }
    }
    files.insert(f.preset.display().to_string(), read(&f.preset)?);
    files.insert(f.map.display().to_string(), read(&f.map)?);
    Ok(files)
}

fn load(f: &Files, texts: &batch::Files, seed: u64) -> Result<Game, String> {
    let (preset, map) = (f.preset.display().to_string(), f.map.display().to_string());
    batch::load(texts, &preset, &map, seed).map_err(|e| e.join("\n"))
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
        "axes": c.axes.iter().map(|(a, v)| (a.0.clone(), v.0 / Fx::SCALE)).collect::<BTreeMap<_, _>>(),
        "deserted": c.deserted,
        "kin": c.kin,
    })
}

fn print_dynasty(g: &Game, c: &sim::Chronicle, s: &score::Score) {
    print!("{}", batch::chronicle_text(g, c));
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
        DecisionKind::Testament(_) => "завещание".into(),
    };
    format!("{date} {what} [{}]", d.cause_tag)
}
