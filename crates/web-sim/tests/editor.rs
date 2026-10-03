//! Stage 23 acceptance: the core of the content editor on the data of `data/`, edited as the
//! editor edits it (the text of one record replaced).

use std::collections::BTreeMap;

/// Every `.ron` of `data/`, by its path under it.
fn files() -> BTreeMap<String, String> {
    let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data"));
    let mut out = BTreeMap::new();
    let mut dirs = vec![root.clone()];
    while let Some(dir) = dirs.pop() {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "ron") {
                let key = p
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(key, std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    out
}

/// The first `from` of `file` replaced.
fn edit(files: &mut BTreeMap<String, String>, file: &str, from: &str, to: &str) {
    let text = &files[file];
    assert!(text.contains(from), "{file}: {from}");
    files.insert(file.into(), text.replacen(from, to, 1));
}

fn load(files: &BTreeMap<String, String>) -> serde_json::Value {
    let out = web_sim::load(&serde_json::to_string(files).unwrap());
    serde_json::from_str(&out).unwrap()
}

#[test]
fn the_data_loads_clean() {
    assert_eq!(
        load(&files()),
        serde_json::json!({ "errors": [], "lint": [] })
    );
}

#[test]
fn broken_ron_is_an_error_with_the_path_to_the_field() {
    let mut f = files();
    edit(
        &mut f,
        "events/reign.ron",
        "cooldown_years: 6,",
        "cooldown_years: \"шесть\",",
    );
    let out = load(&f);
    let errors = out["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{out}");
    let e = errors[0].as_str().unwrap();
    assert!(e.starts_with("events/reign.ron:"), "{e}");
    assert!(e.ends_with("[[0].cooldown_years]"), "{e}");
    // A missing field: the record it is missing from.
    let mut f = files();
    edit(
        &mut f,
        "events/reign.ron",
        "        title: \"Интриги при дворе\",\n",
        "",
    );
    let e = load(&f)["errors"][0].as_str().unwrap().to_string();
    assert!(e.contains("`title`") && e.ends_with("[[0]]"), "{e}");
}

#[test]
fn a_number_changes_the_batch_and_its_revert_restores_it() {
    let f = files();
    let run = |f: &BTreeMap<String, String>| {
        assert_eq!(load(f)["errors"], serde_json::json!([]));
        web_sim::run_batch("neutral", 20, 0, None).unwrap().0
    };
    let before = run(&f);
    let mut edited = f.clone();
    // The weight of «Интриги при дворе».
    let from = "weight: 4,\n        once: false,\n        cooldown_years: 6,";
    edit(
        &mut edited,
        "events/reign.ron",
        from,
        &from.replace("4", "40"),
    );
    let after = run(&edited);
    assert_ne!(before, after);
    assert_eq!(run(&f), before);
}

/// A country and a province renamed with their cases in names.ron: the world has the new
/// names, and the chronicle declines them.
#[test]
fn a_renamed_country_and_province_reach_the_chronicle_in_their_cases() {
    let chronicles = |f: &BTreeMap<String, String>| {
        assert_eq!(load(f)["errors"], serde_json::json!([]));
        (0..40)
            .map(|s| web_sim::run_chronicle(s).unwrap())
            .collect::<String>()
    };
    let old = chronicles(&files());
    assert!(
        old.contains("под руку Нордмарка"),
        "no land lost to Nordmark in 40 games"
    );
    let mut f = files();
    edit(
        &mut f,
        "presets/default.ron",
        "name: \"Нордмарк\"",
        "name: \"Северия\"",
    );
    edit(
        &mut f,
        "names.ron",
        "\"Нордмарк||а|у||ом|е\"",
        "\"Севери|я|и|и|ю|ей|и\"",
    );
    edit(
        &mut f,
        "maps/default.ron",
        "name: \"Нордхейм\"",
        "name: \"Северград\"",
    );
    edit(
        &mut f,
        "names.ron",
        "\"Нордхейм||а|у||ом|е\"",
        "\"Северград||а|у||ом|е\"",
    );
    let new = chronicles(&f);
    assert!(new.contains("под руку Северии"), "{new}");
    assert!(
        !new.contains("Нордмарк") && !new.contains("Нордхейм"),
        "{new}"
    );
    assert_eq!(load(&f)["lint"], serde_json::json!([]));
    // Without its cases the new name stays in the nominative, and the lint says so.
    let mut f = files();
    edit(
        &mut f,
        "presets/default.ron",
        "name: \"Нордмарк\"",
        "name: \"Северия\"",
    );
    assert_eq!(
        load(&f)["lint"],
        serde_json::json!(["Северия: нет падежей (names.ron forms)"])
    );
}

#[test]
fn trace_and_errors_before_load() {
    assert!(load(&files())["errors"].as_array().unwrap().is_empty());
    let out: serde_json::Value = serde_json::from_str(&web_sim::trace(42, None)).unwrap();
    assert!(out["text"].as_str().unwrap().contains(" [-]\n"), "{out}");
    let out: serde_json::Value =
        serde_json::from_str(&web_sim::trace(42, Some("nothing".into()))).unwrap();
    assert_eq!(out["error"], "нет оси nothing");
    let out: serde_json::Value = serde_json::from_str(&web_sim::batch("nope", 1, 0, None)).unwrap();
    assert_eq!(out["error"], "нет стратегии nope");
}
