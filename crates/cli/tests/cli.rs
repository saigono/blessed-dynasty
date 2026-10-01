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
