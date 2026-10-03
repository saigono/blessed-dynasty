//! The core in the browser, for the content editor (editor/index.html): the data from a set
//! of texts, `batch` and `trace` as `cli` runs them (`bd_core::batch`). Every export takes and
//! gives strings, JSON where there is structure.

use bd_core::batch::{self, Files};
use bd_core::game::Game;
use bd_core::score::ScoreRules;
use std::cell::RefCell;
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;

/// The preset and the map `cli` plays by default.
pub const PRESET: &str = "presets/default.ron";
pub const MAP: &str = "maps/default.ron";

struct Loaded {
    files: Files,
    start: Game,
    rules: ScoreRules,
}

thread_local! {
    static LOADED: RefCell<Option<Loaded>> = const { RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    /// The page's progress callback: one more game of `batch` played.
    #[wasm_bindgen(js_namespace = globalThis, js_name = bdProgress)]
    fn progress(seed: u32);
}

#[cfg(not(target_arch = "wasm32"))]
fn progress(_: u32) {}

/// Loads `files` (path under `data/` -> RON text) for `batch`, `trace` and `chronicle`: the
/// remarks of `bd_core::lint`, or the errors of `batch::load` (the data loaded before stays).
pub fn load_files(files: Files) -> Result<Vec<String>, Vec<String>> {
    let start = batch::load(&files, PRESET, MAP, 0)?;
    let rules = batch::score_rules(&files, &start).map_err(|e| vec![e])?;
    let lint = bd_core::lint::lint(&start.data, &start.world);
    let loaded = Loaded {
        files,
        start,
        rules,
    };
    LOADED.with(|l| *l.borrow_mut() = Some(loaded));
    Ok(lint)
}

fn with<T>(f: impl FnOnce(&Loaded) -> Result<T, String>) -> Result<T, String> {
    LOADED.with(|l| f(l.borrow().as_ref().ok_or("данные не загружены")?))
}

/// `cli batch --strategy --runs --seed-start [--law]` on the loaded data: its output, and per
/// event id the number of games it came in (`Row.events`).
pub fn run_batch(
    strategy: &str,
    runs: u32,
    seed_start: u32,
    law: Option<&str>,
) -> Result<(String, BTreeMap<String, usize>), String> {
    with(|l| {
        let mut start = l.start.clone();
        if let Some(law) = law {
            batch::set_law(&mut start, law)?;
        }
        let auto = batch::chooser(&l.files, &start, strategy)?;
        let seeds = seed_start as u64..seed_start as u64 + runs as u64;
        let tell = |s: u64| progress(s as u32);
        let (text, rows) = batch::batch(&start, seeds, &[], auto.as_ref(), &l.rules, tell)?;
        let mut events = BTreeMap::new();
        for id in rows.iter().flat_map(|r| &r.events) {
            *events.entry(id.clone()).or_default() += 1;
        }
        Ok((text, events))
    })
}

/// `cli trace --seed` with an empty script, `--node` if given.
pub fn run_trace(seed: u32, node: Option<&str>) -> Result<String, String> {
    with(|l| {
        let g = l.start.reseeded(seed as u64);
        batch::trace_of(g, &l.rules, &[], node, None)
    })
}

/// The chronicle of a neutral game of `seed`, as `cli run --strategy neutral` tells it.
pub fn run_chronicle(seed: u32) -> Result<String, String> {
    with(|l| {
        let mut g = l.start.reseeded(seed as u64);
        batch::play(&mut g, None, &mut vec![])?;
        match batch::dynasty(&g, &l.rules).0 {
            Some(c) => Ok(batch::chronicle_text(&g, &c)),
            None => Err("правление не кончилось".into()),
        }
    })
}

fn json(r: Result<serde_json::Value, String>) -> String {
    match r {
        Ok(v) => v.to_string(),
        Err(e) => serde_json::json!({ "error": e }).to_string(),
    }
}

/// `files`: a JSON object path -> text. `{"errors": [...], "lint": [...]}`.
#[wasm_bindgen]
pub fn load(files: &str) -> String {
    let files: Files = match serde_json::from_str(files) {
        Ok(f) => f,
        Err(e) => return json(Err(format!("files: {e}"))),
    };
    let (errors, lint) = match load_files(files) {
        Ok(lint) => (vec![], lint),
        Err(errors) => (errors, vec![]),
    };
    serde_json::json!({ "errors": errors, "lint": lint }).to_string()
}

/// `{"text": <cli batch output>, "events": {id: games}}` or `{"error": ...}`.
#[wasm_bindgen]
pub fn batch(strategy: &str, runs: u32, seed_start: u32, law: Option<String>) -> String {
    let r = run_batch(strategy, runs, seed_start, law.as_deref());
    json(r.map(|(text, events)| serde_json::json!({ "text": text, "events": events })))
}

/// `{"text": <cli trace output>}` or `{"error": ...}`.
#[wasm_bindgen]
pub fn trace(seed: u32, node: Option<String>) -> String {
    let r = run_trace(seed, node.as_deref());
    json(r.map(|text| serde_json::json!({ "text": text })))
}

/// `{"text": <the chronicle>}` or `{"error": ...}`.
#[wasm_bindgen]
pub fn chronicle(seed: u32) -> String {
    json(run_chronicle(seed).map(|text| serde_json::json!({ "text": text })))
}
