//! Acceptance of stage 8a: the binary on the real data.

use std::collections::BTreeMap;
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
                      treasury,deserted,treasury_10,treasury_20,treasury_30,";
        let head = lines.next().unwrap();
        assert!(head.starts_with(header), "{head}");
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
        [
            "builder",
            "crown_all",
            "free_towns",
            "scholar",
            "serf_lord",
            "vassal_all",
            "warmonger"
        ]
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

/// Stage 18: the hidden nodes of the graph at the dynasty's 100th and 150th year and at the
/// fall, one column each, and their share of years at a bound.
#[test]
fn batch_reports_the_hidden_nodes() {
    let out = batch(&["--runs", "3"]);
    let mut lines = out.lines();
    let head: Vec<_> = lines.next().unwrap().split(',').collect();
    for col in ["serfdom_100", "serfdom_150", "serfdom_fall", "faith_150"] {
        assert!(head.contains(&col), "{col}: {head:?}");
    }
    // Hidden, but no node: no edge leads to or from the shocks.
    assert!(!head.contains(&"shocks_fall"), "{head:?}");
    let row: Vec<_> = lines.next().unwrap().split(',').collect();
    assert_eq!(row.len(), head.len());
    let fall = head.iter().position(|c| *c == "faith_fall").unwrap();
    assert!(row[fall].parse::<i64>().is_ok(), "{row:?}");
    assert!(out.contains("#   faith "), "{out}");
    assert!(out.contains("# узло-лет на краях "), "{out}");
}

/// Stage 18: `trace --node` tells a node's value, target and edges year by year.
#[test]
fn trace_tells_the_edges_into_a_node() {
    let args = [
        "trace",
        "--seed",
        "42",
        "--script",
        "data/scripts/test.ron",
        "--node",
    ];
    let out = stdout(cli(&[&args[..], &["serfdom"]].concat()));
    let first = out.lines().next().unwrap();
    // «1218 serfdom 30 → 30: e4 +0»: the year, the value, the target, edge e4 from the nobles.
    assert!(
        first.contains(" serfdom ") && first.contains(" → "),
        "{out}"
    );
    assert!(first.contains(": e4 "), "{out}");
    assert!(out.lines().count() > 50, "{out}");
    let bad = Command::new(env!("CARGO_BIN_EXE_cli"))
        .args([&args[..], &["nothing"]].concat())
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("нет оси nothing"));
    // Stage 19: under a law, its anchor shift and its edge multipliers. The founder brings in
    // serfdom: anchor 30 + 30, e4 × 1.5.
    let script = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("serfdom.ron");
    std::fs::write(&script, r#"[Action("enact_law_serfdom", None), Wait(40)]"#).unwrap();
    let script = script.to_str().unwrap();
    let args = [
        "trace", "--seed", "42", "--script", script, "--node", "serfdom",
    ];
    let out = stdout(cli(&args));
    let first = out.lines().next().unwrap();
    let target: i64 = first
        .split(" → ")
        .nth(1)
        .unwrap()
        .split(':')
        .next()
        .unwrap()
        .split('.')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(target >= 60, "{out}");
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

/// `batch --runs 1000` of each strategy at once, from `data` (relative to the repository).
fn batches(strategies: &[&str], data: &str) -> Vec<String> {
    let runs: Vec<_> = (strategies.iter())
        .map(|s| {
            Command::new(env!("CARGO_BIN_EXE_cli"))
                .args(["batch", "--runs", "1000", "--strategy", s, "--data", data])
                .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    (runs.into_iter())
        .map(|c| stdout(c.wait_with_output().unwrap()))
        .collect()
}

/// Stage 18, criterion 5 of docs/design/hidden-state.html: the graph does not explode. On
/// every strategy at most 5% of the node-years lie at a bound (0 or 100), and no median of a
/// node (at the 100th and 150th year, at the fall) does.
/// `cargo test --release -p cli -- --ignored graph_does_not_explode`.
#[test]
#[ignore = "release only, a minute"]
fn graph_does_not_explode() {
    let strategies = ["neutral", "crown_all", "vassal_all", "warmonger", "builder"];
    for (s, out) in strategies.iter().zip(batches(&strategies, "data")) {
        let line = |p: &str| {
            out.lines()
                .find_map(|l| l.strip_prefix(p))
                .unwrap()
                .to_string()
        };
        let share = line("# узло-лет на краях ");
        let (int, frac) = share.trim_end_matches('%').split_once('.').unwrap();
        let permille: u32 = int.parse::<u32>().unwrap() * 10 + frac.parse::<u32>().unwrap();
        assert!(permille <= 50, "{s}: {share}");
        let nodes = (out.lines())
            .skip_while(|l| !l.starts_with("# скрытые узлы"))
            .skip(1)
            .map_while(|l| l.strip_prefix("#   "));
        let mut n = 0;
        for l in nodes {
            n += 1;
            let parts: Vec<_> = l.split(" | ").collect();
            for q in &parts[..3] {
                let median = q.split(" / ").nth(1).unwrap();
                assert!(median != "0" && median != "100", "{s}: {l}");
            }
        }
        assert_eq!(n, 8, "{s}: {out}");
    }
}

/// Stage 18, criterion 6: `neutral` on 1000 games at most 15% slower than on main's data
/// (`crates/core/tests/main/data`): the graph's cost.
/// `cargo test --release -p cli -- --ignored graph_costs`.
#[test]
#[ignore = "release only, half a minute"]
fn graph_costs_at_most_15_percent() {
    // The faster of two runs each, one after the other.
    let time = |data: &str| {
        (0..2)
            .map(|_| {
                let t = std::time::Instant::now();
                let out = batch(&["--runs", "1000", "--data", data]);
                assert!(out.contains("# runs 1000\n"));
                t.elapsed()
            })
            .min()
            .unwrap()
    };
    let (old, new) = (time("crates/core/tests/main/data"), time("data"));
    assert!(
        new.as_millis() * 100 <= old.as_millis() * 115,
        "{old:?} -> {new:?}"
    );
}

/// The `#` summary of a batch: (median score, median reign years, early death %, the share
/// of each fall reason in %).
fn summary(out: &str) -> (i64, i64, u32, BTreeMap<String, i64>) {
    let line = |prefix: &str| {
        let l = out.lines().find(|l| l.starts_with(prefix));
        l.unwrap_or_else(|| panic!("{prefix}: {out}"))[prefix.len()..].to_string()
    };
    let median = |prefix: &str| line(prefix).split(" / ").nth(1).unwrap().parse().unwrap();
    let early = line("# ранняя смерть ")
        .trim_end_matches('%')
        .parse()
        .unwrap();
    let falls = (out.lines())
        .skip_while(|l| *l != "# причины падения:")
        .skip(1)
        .map_while(|l| l.strip_prefix("#   "))
        .map(|l| {
            let (fall, share) = l.split_once(' ').unwrap();
            (
                fall.to_string(),
                share.trim_end_matches('%').parse().unwrap(),
            )
        });
    (
        median("#   счёт "),
        median("#   лет правления "),
        early,
        falls.collect(),
    )
}

/// Stage 17b: two strategies fall differently if the share of some fall reason differs by
/// at least 15 points.
fn falls_differ(a: &BTreeMap<String, i64>, b: &BTreeMap<String, i64>) -> bool {
    let share = |m: &BTreeMap<String, i64>, k: &String| m.get(k).copied().unwrap_or(0);
    (a.keys().chain(b.keys())).any(|k| (share(a, k) - share(b, k)).abs() >= 15)
}

#[test]
fn falls_differ_by_15_points_of_one_reason() {
    let batch = |falls: &str| {
        let head = "#   лет правления 1 / 30 / 50\n#   счёт 1 / 100 / 200\n# ранняя смерть 5%\n";
        summary(&format!("{head}# причины падения:\n{falls}# закон\n")).3
    };
    let a = batch("#   Usurped 40%\n#   Alive 40%\n#   NoHeir 20%\n");
    assert_eq!(a.len(), 3);
    assert!(!falls_differ(&a, &a));
    let b = batch("#   Usurped 26%\n#   Alive 54%\n#   NoHeir 20%\n");
    assert!(!falls_differ(&a, &b));
    let b = batch("#   Usurped 25%\n#   Alive 55%\n#   NoHeir 20%\n");
    assert!(falls_differ(&a, &b));
    // A reason one of them never meets counts as 0%.
    let b = batch("#   Usurped 40%\n#   Alive 25%\n#   NoHeir 20%\n#   NoCrownLand 15%\n");
    assert!(falls_differ(&a, &b));
}

/// Stage 8b acceptance: the calibration criteria on 1000 games per strategy (the tables of
/// docs/calibration.md). Stage 15 adds: the neutral median treasury at the end of the
/// dynasty at most 20% of the cap (10000), and warmonger armies desert in at least 10% of
/// the dynasties. Stage 17b: the fall reasons of neighbours by score differ (`falls_differ`)
/// instead of the dominant one, and warmonger heirs wed in war: its NoHeir at most 5 points
/// above neutral's. `cargo test --release -p cli -- --ignored calibration`.
/// Stage 18: fails on `falls_differ` (crown_all / warmonger 7 points), a question to the
/// design; see docs/calibration.md, stage 18.
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
    // Neighbours by score (vassal_all < crown_all < warmonger): the score at least 15%
    // apart, and (stage 17b) the fall reasons too, by 15 points of one reason.
    let mut by_score: Vec<_> = s[1..].iter().collect();
    by_score.sort_by_key(|x| x.0);
    for w in by_score.windows(2) {
        assert!(w[1].0 * 100 >= w[0].0 * 115, "{s:?}");
        assert!(falls_differ(&w[0].3, &w[1].3), "{s:?}");
    }
    let no_heir = |i: usize| s[i].3.get("NoHeir").copied().unwrap_or(0);
    assert!(no_heir(3) <= no_heir(0) + 5, "{s:?}");
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

/// Stage 16: a batch may start on any law of `heirs.laws` (stage 19: of `laws`); it tells the
/// share of contested successions and of law changes; an unknown law fails.
#[test]
fn batch_starts_on_a_law() {
    let out = batch(&["--runs", "3", "--law", "law_salic"]);
    assert!(out.contains("# спор о престоле: "), "{out}");
    assert!(out.contains("# закон сменён после основателя в "), "{out}");
    assert!(
        out.contains("# воцарения назначенных в обход закона: "),
        "{out}"
    );
    assert!(out.contains("# воцарения бастардов: "), "{out}");
    assert_ne!(out, batch(&["--runs", "3"]));
    // Stage 19: any law of rules.ron `laws`, in place of its group's; the laws at the fall.
    let out = batch(&["--runs", "3", "--law", "law_serfdom"]);
    assert!(out.contains(" law_serfdom "), "{out}");
    assert!(out.contains("# отмена закона в "), "{out}");
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
    // Stage 17: the rightful heir's claim cuts the disputes of absolute primogeniture below
    // the 32% of stage 16 without letting NoHeir soar; male primogeniture has its own risk.
    // Stage 17b: at most 12% under absolute primogeniture, only a child or a weak heir.
    let disputes = |i: usize| -rows[i][3];
    assert!(disputes(0) <= 12, "{rows:?}");
    assert!(fall(&outs[0], "NoHeir") <= 20, "{rows:?}");
    assert!(disputes(1) >= disputes(0) + 10, "{rows:?}");
    // Some heirs are named over the law, under the laws that leave rivals with a claim.
    let named = |o: &str| number(o, "# воцарения назначенных в обход закона: ");
    assert!(outs.iter().any(|o| named(o) > 0));
}
