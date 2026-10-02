//! Acceptance of stage 8a: the binary on the real data.

use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_cli"))
        .args(args)
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{args:?}: {err}");
    out
}

fn stdout(out: Output) -> String {
    String::from_utf8(out.stdout).unwrap()
}

fn run(script: &str, json: bool) -> Output {
    let path = format!("data/scripts/{script}.ron");
    let mut args = vec!["run", "--seed", "42", "--script", &path];
    if json {
        args.push("--json");
    }
    cli(&args)
}

#[test]
fn run_script_prints_decisions() {
    let out = stdout(run("test", false));
    let decisions = out.lines().skip_while(|l| *l != "Решения:").skip(1);
    let decisions: Vec<_> = decisions.take_while(|l| l.starts_with("  ")).collect();
    assert!(!decisions.is_empty(), "{out}");
    assert!(out.contains("Хроника: недоступно"), "{out}");
}

#[test]
fn replay_hash_is_stable() {
    replay_matches("test");
}

#[test]
fn abdication_ends_the_reign_and_replays() {
    let out = stdout(run("abdicate", false));
    assert!(out.contains("отречение [abdication]"), "{out}");
    assert!(out.contains("Конец правления: abdication"), "{out}");
    let json = stdout(run("abdicate", true));
    assert!(json.contains("\"reign_end\": \"abdication\""), "{json}");
    replay_matches("abdicate");
}

#[test]
fn neutral_runs_until_the_reign_ends() {
    let out = stdout(cli(&["run", "--seed", "42", "--strategy", "neutral"]));
    assert!(out.contains("Конец правления: "), "{out}");
}

/// Replays `data/scripts/{script}.ron` with seed 42 twice; both give the original hash.
fn replay_matches(script: &str) {
    let journal = format!("{}/{script}.json", env!("CARGO_TARGET_TMPDIR"));
    let run = run(script, true);
    std::fs::write(&journal, &run.stdout).unwrap();
    let replay = || stdout(cli(&["replay", "--seed", "42", "--decisions", &journal]));
    let hash = replay();
    assert_eq!(hash.trim().len(), 16, "{hash}");
    assert_eq!(hash, replay());
    let run = stdout(run);
    let expected = format!("\"world_hash\": \"{}\"", hash.trim());
    assert!(run.contains(&expected), "{run}");
}

fn batch(args: &[&str]) -> String {
    stdout(cli(&[&["batch"], args].concat()))
}

/// Stage 8b acceptance: the same seeds give the same CSV and summary.
#[test]
fn batch_of_ten_is_deterministic() {
    for strategy in ["neutral", "warmonger"] {
        let args = ["--runs", "10", "--strategy", strategy, "--seed-start", "5"];
        let out = batch(&args);
        assert_eq!(out, batch(&args));
        let mut lines = out.lines();
        let header = "seed,reign_years,dynasty_years,score,fall_reason,early_death,army,\
                      treasury,deserted,treasury_10,treasury_20,treasury_30";
        assert_eq!(lines.next(), Some(header));
        let rows: Vec<_> = lines.clone().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(rows.len(), 10, "{out}");
        assert!(
            rows[0].starts_with("5,") && rows[9].starts_with("14,"),
            "{out}"
        );
        assert!(
            out.contains("# runs 10\n")
                && out.contains("#   счёт ")
                && out.contains("#   армия в конце "),
            "{out}"
        );
        assert!(
            out.contains("# ранняя смерть ") && out.contains("# причины падения:"),
            "{out}"
        );
    }
}

#[test]
fn every_strategy_plays_and_an_unknown_one_fails() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/strategies.ron"
    ))
    .unwrap();
    let all: std::collections::BTreeMap<String, ron::Value> = ron::from_str(&text).unwrap();
    assert_eq!(
        all.keys().collect::<Vec<_>>(),
        ["builder", "crown_all", "vassal_all", "warmonger"]
    );
    for s in all.keys() {
        assert!(batch(&["--runs", "1", "--strategy", s]).contains("# runs 1\n"));
    }
    let out = Command::new(env!("CARGO_BIN_EXE_cli"))
        .args(["batch", "--runs", "1", "--strategy", "nope"])
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("нет стратегии nope"));
}

#[test]
fn trace_links_entries_to_decisions() {
    let args = ["trace", "--seed", "42", "--script", "data/scripts/test.ron"];
    let out = stdout(cli(&args));
    assert_eq!(out, stdout(cli(&args)));
    // Every entry: a header line, then its causes or the note that it has none.
    let first = out.lines().next().unwrap();
    assert!(first.ends_with("Новое правление [-]"), "{out}");
    let chain = out
        .lines()
        .find(|l| l.starts_with("  решение #"))
        .expect("a cause");
    assert!(
        chain.contains("(тик ") && chain.contains(") → метка ("),
        "{chain}"
    );
    assert!(out.contains("  без решений основателя\n"), "{out}");
}

/// Stage 8b acceptance: 1000 games in under 60 s. Only meaningful in release:
/// `cargo test --release -p cli -- --ignored batch_of_a_thousand`.
#[test]
#[ignore = "release only, about a minute"]
fn batch_of_a_thousand_is_fast() {
    let t = std::time::Instant::now();
    let out = batch(&["--runs", "1000"]);
    let took = t.elapsed();
    assert!(out.contains("# runs 1000\n"));
    assert!(took.as_secs() < 60, "{took:?}");
}

/// The `#` summary of a batch: (median score, median reign years, early death %, the most
/// frequent fall reason).
fn summary(out: &str) -> (i64, i64, u32, String) {
    let line = |prefix: &str| {
        let l = out.lines().find(|l| l.starts_with(prefix));
        l.unwrap_or_else(|| panic!("{prefix}: {out}"))[prefix.len()..].to_string()
    };
    let median = |prefix: &str| line(prefix).split(" / ").nth(1).unwrap().parse().unwrap();
    let early = line("# ранняя смерть ")
        .trim_end_matches('%')
        .parse()
        .unwrap();
    let falls = out
        .lines()
        .skip_while(|l| *l != "# причины падения:")
        .nth(1)
        .unwrap();
    let dominant = falls.split_whitespace().nth(1).unwrap().to_string();
    (
        median("#   счёт "),
        median("#   лет правления "),
        early,
        dominant,
    )
}

/// Stage 8b acceptance: the calibration criteria on 1000 games per strategy (the tables of
/// docs/calibration.md). Stage 15 adds: the neutral median treasury at the end of the
/// dynasty at most 20% of the cap (10000), and warmonger armies desert in at least 10% of
/// the dynasties. `cargo test --release -p cli -- --ignored calibration`.
#[test]
#[ignore = "release only, a few minutes"]
fn calibration_criteria_hold() {
    let strategies = ["neutral", "crown_all", "vassal_all", "warmonger"];
    let runs: Vec<_> = (strategies.iter())
        .map(|s| {
            Command::new(env!("CARGO_BIN_EXE_cli"))
                .args(["batch", "--runs", "1000", "--strategy", s])
                .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let outs: Vec<String> = (runs.into_iter())
        .map(|c| stdout(c.wait_with_output().unwrap()))
        .collect();
    let s: Vec<_> = outs.iter().map(|o| summary(o)).collect();
    let (_, reign, early, _) = &s[0];
    assert!(*early <= 10, "{s:?}");
    assert!((25..=40).contains(reign), "{s:?}");
    let mut scores: Vec<i64> = s[1..].iter().map(|x| x.0).collect();
    scores.sort();
    assert!(scores.windows(2).all(|w| w[1] * 100 >= w[0] * 115), "{s:?}");
    let falls: std::collections::BTreeSet<_> = s[1..].iter().map(|x| &x.3).collect();
    assert_eq!(falls.len(), 3, "{s:?}");
    let after = |out: &str, prefix: &str| -> String {
        let l = out.lines().find(|l| l.starts_with(prefix)).unwrap();
        l[prefix.len()..].to_string()
    };
    let treasury: i64 = (after(&outs[0], "#   казна в конце ").split(" / ").nth(1))
        .unwrap()
        .parse()
        .unwrap();
    assert!(treasury <= 2000, "{treasury}");
    let deserted: u32 = (after(&outs[3], "# дезертирство в ").split('%').next())
        .unwrap()
        .parse()
        .unwrap();
    assert!(deserted >= 10, "{deserted}");
}

/// Stage 16: a batch may start on any law of `heirs.laws`; it tells the share of contested
/// successions and of law changes; an unknown law fails.
#[test]
fn batch_starts_on_a_law() {
    let out = batch(&["--runs", "3", "--law", "law_salic"]);
    assert!(out.contains("# спор о престоле: "), "{out}");
    assert!(out.contains("# закон сменён после основателя в "), "{out}");
    assert_ne!(out, batch(&["--runs", "3"]));
    let out = Command::new(env!("CARGO_BIN_EXE_cli"))
        .args(["batch", "--runs", "1", "--law", "law_nope"])
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("нет закона law_nope"));
}

/// Stage 16 acceptance: 1000 neutral dynasties on each law (the table of docs/calibration.md);
/// every law has a risk profile of its own: none is best on every row (median years of the
/// dynasty, NoHeir, Usurped, contested successions).
/// `cargo test --release -p cli -- --ignored law_profiles`.
#[test]
#[ignore = "release only, a minute"]
fn law_profiles_differ() {
    let laws = [
        "law_primogeniture",
        "law_male",
        "law_salic",
        "law_seniority",
        "law_elective",
        "law_partition",
    ];
    let runs: Vec<_> = (laws.iter())
        .map(|l| {
            Command::new(env!("CARGO_BIN_EXE_cli"))
                .args(["batch", "--runs", "1000", "--law", l])
                .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let outs: Vec<String> = (runs.into_iter())
        .map(|c| stdout(c.wait_with_output().unwrap()))
        .collect();
    let number = |out: &str, prefix: &str| -> i64 {
        let l = out.lines().find(|l| l.starts_with(prefix));
        let l = l.unwrap_or_else(|| panic!("{prefix}: {out}"));
        let digits = l[prefix.len()..]
            .split(|c: char| !c.is_ascii_digit())
            .next();
        digits.unwrap().parse().unwrap_or(0)
    };
    let fall = |out: &str, f: &str| {
        let l = out.lines().find(|l| l.starts_with(&format!("#   {f} ")));
        l.map_or(0, |l| number(l, &format!("#   {f} ")))
    };
    // Every row as "higher is better".
    let rows: Vec<Vec<i64>> = (outs.iter())
        .map(|o| {
            let median = o
                .lines()
                .find(|l| l.starts_with("#   лет династии "))
                .unwrap();
            let median = median.split(" / ").nth(1).unwrap().parse().unwrap();
            vec![
                median,
                -fall(o, "NoHeir"),
                -fall(o, "Usurped"),
                -number(o, "# спор о престоле: "),
            ]
        })
        .collect();
    for (i, r) in rows.iter().enumerate() {
        let best_everywhere = (0..r.len()).all(|k| rows.iter().all(|o| r[k] >= o[k]));
        assert!(
            !best_everywhere,
            "{} is best on every row: {rows:?}",
            laws[i]
        );
    }
    // The risks the laws are for: Salic dies out, seniority quarrels, partition breaks up.
    let worst = |k: usize| (0..rows.len()).min_by_key(|&i| rows[i][k]).unwrap();
    assert_eq!(laws[worst(1)], "law_salic", "{rows:?}");
    assert_eq!(laws[worst(3)], "law_seniority", "{rows:?}");
    assert!(fall(&outs[5], "NoCrownLand") > fall(&outs[0], "NoCrownLand"));
}
