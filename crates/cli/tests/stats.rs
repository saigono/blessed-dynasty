//! Stage 11b: `cli stats` over a dump of the games table, and the receiver's own check.

use bd_core::batch::{chooser, dynasty, load, play, score_rules};
use bd_core::game::{DecisionKind, Game};
use bd_core::link;
use std::collections::BTreeMap;
use std::process::Command;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// The files `cli` reads by their path under data/.
fn files() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for dir in ["", "events/", "events/sim/", "presets/", "maps/"] {
        for e in std::fs::read_dir(format!("{ROOT}/data/{dir}")).unwrap() {
            let p = e.unwrap().path();
            if p.is_file() && p.extension().is_some_and(|x| x == "ron") {
                let name = p.file_name().unwrap().to_string_lossy();
                out.insert(format!("{dir}{name}"), std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    out
}

/// A game of `seed` by `strategy` to the fall: its link, fall, years and score.
fn game(seed: u64, strategy: &str) -> (Game, String, String, u32, i64) {
    let f = files();
    let mut g = load(&f, "presets/default.ron", "maps/default.ron", seed).unwrap();
    let auto = chooser(&f, &g, strategy).unwrap();
    play(&mut g, auto.as_ref(), &mut vec![]).unwrap();
    let (c, s) = dynasty(&g, &score_rules(&f, &g).unwrap());
    let (c, s) = (c.expect("the reign ended"), s.unwrap());
    let text = link::encode("default", seed, &g);
    (g, text, format!("{:?}", c.fall), c.years, s.total)
}

fn stats(dump: &str, name: &str) -> String {
    let path = std::env::temp_dir().join(format!("bd-stats-{}-{name}", std::process::id()));
    std::fs::write(&path, dump).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_cli"))
        .args(["stats", path.to_str().unwrap(), "--build", "t1"])
        .current_dir(ROOT)
        .output()
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    String::from_utf8(out.stdout).unwrap()
}

/// Two games of the build at hand (one link a whole URL, one bare) and one of another build,
/// as `wrangler d1 execute --json` prints them and as a CSV: the same, right numbers.
#[test]
fn stats_replay_the_links_of_the_build() {
    let (g1, l1, f1, y1, s1) = game(3, "neutral");
    let (g2, l2, f2, y2, s2) = game(4, "builder");
    let rows = [
        ("t1", format!("https://user.github.io/bd/#p={l1}"), &f1, y1, s1),
        ("t1", l2.clone(), &f2, y2, s2),
        ("old", "#p=AAAA".to_string(), &f1, 1, 1),
    ];
    let json = rows.iter().enumerate().map(|(i, (v, l, f, y, s))| {
        format!(r#"{{"id":{i},"date":"2026-10-05 12:00:00","version":"{v}","link":"{l}","fall":"{f}","years":{y},"score":{s}}}"#)
    });
    let json = format!(
        r#"[{{"results":[{}],"success":true,"meta":{{}}}}]"#,
        json.collect::<Vec<_>>().join(",")
    );
    let csv = rows.iter().enumerate().map(|(i, (v, l, f, y, s))| {
        format!("{i},2026-10-05 12:00:00,{v},{l},{f},{y},{s}\n")
    });
    let csv = format!("id,date,version,link,fall,years,score\n{}", csv.collect::<String>());
    let out = stats(&json, "json");
    assert_eq!(stats(&csv, "csv"), out);

    assert!(out.starts_with("Партий: 3, сборки t1: 2, других сборок: 1\n"), "{out}");
    assert!(out.contains("Не открылись: 0\nНе сошлись с присланным: 0\n"), "{out}");
    let falls = if f1 == f2 {
        format!("  {f1}: 2\n")
    } else {
        let (a, b) = if f1 < f2 { (&f1, &f2) } else { (&f2, &f1) };
        format!("  {a}: 1\n  {b}: 1\n")
    };
    assert!(out.contains(&format!("Причины падения:\n{falls}Медиана")), "{out}");
    let years = (y1 as i64 + y2 as i64) / 2;
    assert!(out.contains(&format!("Медиана лет: {years}\n")), "{out}");
    assert!(out.contains(&format!("Медиана счёта: {}\n", (s1 + s2) / 2)), "{out}");
    assert!(out.ends_with("Другие сборки:\n  old: 1\n"), "{out}");

    // Every decision of the two founders counted once, the builder's actions among them.
    let decisions = out.split("(раз, в партиях):\n").nth(1).unwrap();
    let decisions = decisions.lines().take_while(|l| l.starts_with("  "));
    let counts: Vec<(&str, usize, usize)> = decisions
        .map(|l| {
            let (key, n) = l.trim().rsplit_once(": ").unwrap();
            let (n, games) = n.split_once(", ").unwrap();
            (key, n.parse().unwrap(), games.parse().unwrap())
        })
        .collect();
    let total: usize = counts.iter().map(|c| c.1).sum();
    assert_eq!(total, g1.decisions.len() + g2.decisions.len());
    assert!(counts.iter().all(|&(_, n, games)| (1..=2).contains(&games) && games <= n));
    let built = g2.decisions.iter().find_map(|d| match &d.kind {
        DecisionKind::ActionStarted { action_id, .. } => Some(format!("действие {action_id}")),
        _ => None,
    });
    let built = built.expect("the builder acts");
    assert!(counts.iter().any(|c| c.0 == built), "{built}: {out}");

    // A row with a link that does not open, and one whose score is not the replay's.
    let bare = format!(
        r##"[{{"version":"t1","link":"#p=AAAA","fall":"NoHeir","years":1,"score":1}},
            {{"version":"t1","link":"{l2}","fall":"{f2}","years":{y2},"score":{}}}]"##,
        s2 + 1
    );
    let out = stats(&bare, "bare");
    assert!(out.contains("Не открылись: 1\n  #p=AAAA: "), "{out}");
    assert!(out.contains("Не сошлись с присланным: 1\n"), "{out}");
    assert!(out.contains(&format!("Медиана счёта: {s2}\n")), "{out}");
}

/// deploy/stats-worker/test.mjs: 400 on a body too large or broken. Skipped without node.
#[test]
fn the_worker_refuses_bad_bodies() {
    let Ok(out) = Command::new("node")
        .arg("deploy/stats-worker/test.mjs")
        .current_dir(ROOT)
        .output()
    else {
        eprintln!("no node: worker.js not checked");
        return;
    };
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "worker.js: ok\n");
}
