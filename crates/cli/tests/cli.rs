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

#[test]
fn run_script_prints_decisions() {
    let out = stdout(cli(&[
        "run",
        "--seed",
        "42",
        "--script",
        "data/scripts/test.ron",
    ]));
    let decisions = out.lines().skip_while(|l| *l != "Решения:").skip(1);
    let decisions: Vec<_> = decisions.take_while(|l| l.starts_with("  ")).collect();
    assert!(!decisions.is_empty(), "{out}");
    assert!(out.contains("Хроника: недоступно"), "{out}");
}

#[test]
fn replay_hash_is_stable() {
    let journal = format!("{}/journal.json", env!("CARGO_TARGET_TMPDIR"));
    let run = cli(&[
        "run",
        "--seed",
        "42",
        "--script",
        "data/scripts/test.ron",
        "--json",
    ]);
    std::fs::write(&journal, &run.stdout).unwrap();
    let replay = || stdout(cli(&["replay", "--seed", "42", "--decisions", &journal]));
    let hash = replay();
    assert_eq!(hash.trim().len(), 16, "{hash}");
    assert_eq!(hash, replay());
    // And it is the hash of the game that wrote the journal.
    let run = stdout(run);
    assert!(
        run.contains(&format!("\"world_hash\": \"{}\"", hash.trim())),
        "{run}"
    );
}
