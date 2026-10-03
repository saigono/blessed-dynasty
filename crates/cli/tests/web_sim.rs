//! Stage 23 acceptance: the editor's core (`web-sim`) gives the output of `cli batch` byte
//! for byte.

use std::collections::BTreeMap;
use std::process::Command;

/// The files `cli` reads: the `.ron` of `data/`, `data/events`, `data/events/sim`, the
/// preset and the map.
fn files() -> BTreeMap<String, String> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/");
    let mut out = BTreeMap::new();
    for dir in ["", "events/", "events/sim/", "presets/", "maps/"] {
        for e in std::fs::read_dir(format!("{root}{dir}")).unwrap() {
            let p = e.unwrap().path();
            if p.is_file() && p.extension().is_some_and(|x| x == "ron") {
                let name = p.file_name().unwrap().to_string_lossy();
                out.insert(format!("{dir}{name}"), std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    out
}

fn cli_batch(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_cli"))
        .args([&["batch"], args].concat())
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn web_sim_batch_is_cli_batch_byte_for_byte() {
    let loaded = web_sim::load(&serde_json::to_string(&files()).unwrap());
    assert_eq!(loaded, r#"{"errors":[],"lint":[]}"#);
    let web = |strategy: &str, runs: u32, law: Option<&str>| {
        let out = web_sim::batch(strategy, runs, 0, law.map(String::from));
        let out: serde_json::Value = serde_json::from_str(&out).unwrap();
        out["text"].as_str().unwrap().to_string()
    };
    let cli = cli_batch(&["--runs", "100", "--seed-start", "0"]);
    assert_eq!(web("neutral", 100, None), cli);
    let args = [
        "--runs",
        "5",
        "--strategy",
        "warmonger",
        "--law",
        "law_serfdom",
    ];
    assert_eq!(web("warmonger", 5, Some("law_serfdom")), cli_batch(&args));
}
