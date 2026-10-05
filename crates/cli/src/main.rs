//! Dev runner: one game from a seed and a script or strategy, and its replay from a journal.

use bd_core::batch::{self, NEUTRAL, chooser, dynasty, parse, play, play_script, score_rules};
use bd_core::batch::{target_name, world_hash};
use bd_core::fx::Fx;
use bd_core::game::{Decision, DecisionKind, Game};
use bd_core::link::{self, replay};
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
    /// Stage 11b: reads a dump of the `games` table (`wrangler d1 execute --json`, a JSON
    /// array of rows or a CSV with a header) and plays the links of build `--build` again:
    /// the falls, the median years and score, the founder's decisions. Rows of other builds
    /// are only counted.
    Stats {
        file: PathBuf,
        /// The build of the data at hand; by default this binary's `BD_VERSION`.
        #[arg(long, default_value = link::BUILD)]
        build: String,
        #[command(flatten)]
        files: Files,
    },
}

/// A row of the `games` table (deploy/stats-worker/schema.sql): what the game sent.
#[derive(Deserialize)]
struct Row {
    version: String,
    link: String,
    fall: String,
    years: u32,
    score: i64,
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
        Cmd::Stats { file, build, files } => {
            let rows = rows(&read(&file)?)?;
            let texts = read_files(&files)?;
            print!("{}", stats(&rows, &build, &files, &texts)?);
            Ok(())
        }
    }
}

/// The rows of a dump: a JSON array of rows, or of result sets `{"results": [rows]}` as
/// `wrangler d1 execute --json` prints them; else a CSV with a header.
fn rows(text: &str) -> Result<Vec<Row>, String> {
    use serde_json::Value;
    if text.trim_start().starts_with('[') {
        let all: Vec<Value> = serde_json::from_str(text).map_err(|e| format!("JSON: {e}"))?;
        let all = all.into_iter().flat_map(|v| match v.get("results") {
            Some(Value::Array(rows)) => rows.clone(),
            _ => vec![v],
        });
        return (all.map(serde_json::from_value))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("строка выгрузки: {e}"));
    }
    // ponytail: no field of the table holds a comma, so a plain split; quotes trimmed.
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let cells = |l: &str| -> Vec<String> {
        l.split(',').map(|c| c.trim().trim_matches('"').to_string()).collect()
    };
    let head = cells(lines.next().ok_or("пустая выгрузка")?);
    let col = |name: &str| (head.iter().position(|h| h == name)).ok_or(format!("нет столбца {name}"));
    let cols = [col("version")?, col("link")?, col("fall")?, col("years")?, col("score")?];
    lines
        .map(|l| {
            let c = cells(l);
            let [version, link, fall, years, score] =
                cols.map(|i| c.get(i).cloned().unwrap_or_default());
            let num = |v: &str| v.parse::<i64>().map_err(|_| format!("не число «{v}» в «{l}»"));
            Ok(Row {
                years: num(&years)? as u32,
                score: num(&score)?,
                version,
                link,
                fall,
            })
        })
        .collect()
}

/// The report of `cli stats`: rows of `build` played again from their links.
fn stats(rows: &[Row], build: &str, f: &Files, texts: &batch::Files) -> Result<String, String> {
    let preset = f.preset.file_stem().unwrap_or_default().to_string_lossy();
    let rules = score_rules(texts, &load(f, texts, 0)?)?;
    let (ours, others): (Vec<&Row>, Vec<&Row>) = rows.iter().partition(|r| r.version == build);
    // Each row is its own game: by its index, on the threads of `batch`.
    let play = |i: u64| -> Result<(Game, sim::Chronicle, score::Score), String> {
        let text = ours[i as usize].link.as_str();
        let l = link::decode(text.split_once("#p=").map_or(text, |(_, p)| p))?;
        if l.preset_id != preset {
            return Err(format!("пресет {}, а не {preset}", l.preset_id));
        }
        let mut g = load(f, texts, l.seed)?;
        l.play(&mut g)?;
        match dynasty(&g, &rules) {
            (Some(c), Some(s)) => Ok((g, c, s)),
            _ => Err("правление не закончено".into()),
        }
    };
    let games = batch::par_seeds(0..ours.len() as u64, batch::threads(), play, |_| {});

    let mut out = format!(
        "Партий: {}, сборки {build}: {}, других сборок: {}\n",
        rows.len(),
        ours.len(),
        others.len()
    );
    let (mut falls, mut decisions) = (BTreeMap::new(), BTreeMap::new());
    let (mut years, mut scores, mut broken, mut differ) = (vec![], vec![], vec![], 0);
    for (row, game) in ours.iter().zip(&games) {
        let (g, c, s) = match game {
            Ok(game) => game,
            Err(e) => {
                broken.push(format!("  {}: {e}", row.link));
                continue;
            }
        };
        let fall = format!("{:?}", c.fall);
        differ += usize::from((&row.fall, row.years, row.score) != (&fall, c.years, s.total));
        *falls.entry(fall).or_insert(0) += 1;
        years.push(c.years as i64);
        scores.push(s.total);
        let mut seen = std::collections::BTreeSet::new();
        for d in &g.decisions {
            let key = match &d.kind {
                DecisionKind::ActionStarted { action_id, .. } => format!("действие {action_id}"),
                DecisionKind::EventChoice {
                    event_id,
                    choice_idx,
                    ..
                } => format!("событие {event_id}: вариант {choice_idx}"),
                DecisionKind::Abdicate => "отречение".into(),
                DecisionKind::Testament(_) => "завещание".into(),
            };
            let e = decisions.entry(key.clone()).or_insert((0, 0));
            e.0 += 1;
            e.1 += usize::from(seen.insert(key));
        }
    }
    out += &format!("Не открылись: {}\n", broken.len());
    out += &(broken.iter().map(|b| b.clone() + "\n").collect::<String>());
    out += &format!("Не сошлись с присланным: {differ}\n");
    out += "Причины падения:\n";
    for (fall, n) in &falls {
        out += &format!("  {fall}: {n}\n");
    }
    out += &format!("Медиана лет: {}\n", median(&mut years));
    out += &format!("Медиана счёта: {}\n", median(&mut scores));
    out += "Решения основателя (раз, в партиях):\n";
    let mut decisions: Vec<_> = decisions.into_iter().collect();
    decisions.sort_by_key(|(k, (n, _))| (std::cmp::Reverse(*n), k.clone()));
    for (key, (n, games)) in decisions {
        out += &format!("  {key}: {n}, {games}\n");
    }
    out += "Другие сборки:\n";
    let mut builds = BTreeMap::new();
    for r in &others {
        *builds.entry(r.version.as_str()).or_insert(0) += 1;
    }
    for (b, n) in builds {
        out += &format!("  {b}: {n}\n");
    }
    Ok(out)
}

/// The middle value, or the mean of the two middle ones; 0 of none.
fn median(v: &mut [i64]) -> i64 {
    v.sort();
    match v.len() {
        0 => 0,
        n => (v[(n - 1) / 2] + v[n / 2]) / 2,
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
    // An entry told in the one before it (stage 26c) is not shown.
    let entries = c.entries.iter().filter(|e| !e.joined).map(|e| {
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
