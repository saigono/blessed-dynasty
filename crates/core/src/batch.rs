//! The runner of `cli` and `web-sim`: data from a set of texts, a reign played by a script or
//! a strategy, `batch` and its summary, `trace`. No I/O: the caller reads the files.

use crate::data::{AxisDef, Data, DataError};
use crate::fx::Fx;
use crate::game::{DecisionKind, Game, Step};
use crate::rng::Rng;
use crate::rules::Target;
use crate::score::{self, ScoreRules};
use crate::sim::{self, AutoChooser, FallReason};
use crate::state::Preset;
use crate::time::Years;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// The data files by their path under `data/` (`rules.ron`, `events/war.ron`), with their text.
pub type Files = BTreeMap<String, String>;

/// The middle choice of every event, no actions; every other strategy is a set of
/// `AutoChooser` weights in data/strategies.ron.
pub const NEUTRAL: &str = "neutral";

/// `AutoChooser` weights added to the founder's own (`AutoChooser::for_ruler`).
pub type Strategies = BTreeMap<String, BTreeMap<String, Fx>>;

#[derive(Deserialize, Debug, PartialEq)]
pub enum ScriptStep {
    /// Up to n ticks; stops early when an event fires.
    Wait(u32),
    Action(String, Option<Target>),
    Choose(usize),
    /// The first choice with this cause tag, else choice 0.
    ChooseByTag(String),
    Abdicate,
    /// Stage 24: `Game::write_testament`, e.g. `Testament((precept: Some("treasury")))`.
    Testament(crate::testament::Testament),
}

/// A safety cap for the neutral strategy; the ruler dies long before.
pub const MAX_YEARS: Years = Years(150);

/// One game of `batch`.
#[derive(Debug, Default)]
pub struct Row {
    seed: u64,
    reign: u32,
    /// Of the dynasty, the founder's reign included.
    years: u32,
    score: i64,
    fall: Option<FallReason>,
    /// The army and the treasury at the end of the dynasty.
    army: i64,
    treasury: i64,
    /// Years the army deserted for want of pay after the founder.
    deserted: u32,
    /// The treasury at the end of each year of the founder's reign.
    reign_treasury: Vec<i64>,
    /// Coronations after the founder, those contested (`abdication.contested_flag`), and
    /// changes of the succession law in the simulation.
    successions: u32,
    contested: u32,
    law_changes: u32,
    /// The laws of `Data.laws` in force at the last entry, and repeals in the simulation.
    laws: Vec<String>,
    repeals: u32,
    /// Coronations of an heir designated over the rightful one, and of bastards.
    designated: u32,
    bastards: u32,
    /// Per hidden node (`hidden_nodes`, in data order): its value at the dynasty's years
    /// `NODES_AT` (None: fallen before) and at the fall; its years at a bound and all its
    /// simulated years.
    nodes: Vec<[Option<i64>; 3]>,
    bounds: Vec<(usize, usize)>,
    /// Years the shocks of stability (`Data.stability`) stood at a bound, of all simulated.
    shocks: (usize, usize),
    /// The score of the dynasty's first `SCORE_AT` years, as if it lived on after them.
    score_at: i64,
    /// Every catastrophe of `Data.symptoms` in the chronicle (`symptom_gaps`).
    catastrophes: Vec<Gap>,
    /// The events of the game: those chosen in the reign and those of the chronicle.
    pub events: BTreeSet<String>,
    /// Wars begun after the founder (`war.start_event` in the chronicle), stage 24.
    wars: u32,
}

/// The dynasty years of the second score in `batch`: most dynasties of `neutral` live to
/// `sim.max_years`, so the full score hardly tells laws apart (stage 20).
const SCORE_AT: u32 = 150;

/// A catastrophe of `Data.symptoms`: its id, the years since the first entry of one of its
/// symptoms in the chronicle, and since the first after the last catastrophe of its kind;
/// None: no symptom came before it.
type Gap = (String, Option<u32>, Option<u32>);

/// Every catastrophe of `Data.symptoms` in the chronicle, in order (criterion 3 of
/// docs/design/hidden-state.html).
fn symptom_gaps(d: &Data, c: &sim::Chronicle) -> Vec<Gap> {
    let year = |e: &sim::ChronicleEntry| e.tick.year(e.snapshot.time_unit);
    let mut out = vec![];
    for (k, e) in c.entries.iter().enumerate() {
        let id = e.event.as_deref().unwrap_or_default();
        let Some((_, symptoms)) = d.symptoms.iter().find(|(x, _)| x == id) else {
            continue;
        };
        let to = c.entries[k..]
            .iter()
            .take_while(|x| x.tick == e.tick)
            .count()
            + k;
        let last = c.entries[..k]
            .iter()
            .rposition(|x| x.event.as_deref() == Some(id));
        let first = |from: usize| {
            let symptom =
                |x: &&sim::ChronicleEntry| (x.event.as_ref()).is_some_and(|i| symptoms.contains(i));
            c.entries[from..to]
                .iter()
                .find(symptom)
                .map(|f| year(e) - year(f))
        };
        out.push((id.to_string(), first(0), first(last.map_or(0, |i| i + 1))));
    }
    out
}

/// The hidden axes with an edge of the influence graph (not, say, the shocks of stability),
/// with their index in `Data.axes`.
fn hidden_nodes(d: &Data) -> impl Iterator<Item = (usize, &AxisDef)> + Clone {
    let edge = |a: &AxisDef| (d.influences.iter()).any(|e| e.from == a.id || e.to == a.id);
    (d.axes.iter().enumerate()).filter(move |(_, a)| a.hidden && edge(a))
}

/// A symptom this many years before its catastrophe or more counts as foretelling it
/// (criterion 3 of docs/design/hidden-state.html).
const SYMPTOM_YEARS: u32 = 20;

/// Dynasty years `batch` reports the hidden nodes at, besides the fall.
const NODES_AT: [u32; 2] = [100, 150];

/// Reign years `batch` reports the treasury at.
const TREASURY_AT: [usize; 3] = [10, 20, 30];

/// Years before which the founder's death counts as early (DESIGN 4.3).
const EARLY_YEARS: u32 = 10;

pub fn batch_row(
    start: &Game,
    seed: u64,
    script: &[ScriptStep],
    auto: Option<&AutoChooser>,
    rules: &ScoreRules,
) -> Result<Row, String> {
    let mut g = start.reseeded(seed);
    let mut log = vec![];
    play_script(&mut g, script, true, &mut log)?;
    play(&mut g, auto, &mut log)?;
    let reign = g.world.tick.year(g.world.time_unit);
    let (Some(mut c), Some(s)) = dynasty(&g, rules) else {
        return Err(format!(
            "seed {seed}: правитель жив через {} лет",
            MAX_YEARS.0
        ));
    };
    let axis = |a| c.axes.get(a).map_or(0, |v: &Fx| v.0 / Fx::SCALE);
    let t = &g.data.sim.texts;
    let crowned = c.entries.iter().filter(|e| e.title == t.crowned.0);
    let contested =
        |e: &&sim::ChronicleEntry| e.snapshot.flags.contains(&g.data.abdication.contested_flag);
    let laws = c.entries.iter().filter(|e| e.title == t.law_changed.0);
    let hidden = hidden_nodes(&g.data);
    let year = |y: u32, i: usize| {
        let n = c.nodes.iter().find(|n| n.year == y);
        n.map(|n| n.axes[i].0 / Fx::SCALE)
    };
    let nodes = (hidden.clone())
        .map(|(i, a)| {
            [
                year(NODES_AT[0], i),
                year(NODES_AT[1], i),
                Some(axis(&a.id)),
            ]
        })
        .collect();
    let at_bound = |(i, a): (usize, &AxisDef)| {
        let at = (c.nodes.iter()).filter(|n| n.axes[i] <= a.min || n.axes[i] >= a.max);
        (at.count(), c.nodes.len())
    };
    let bounds = hidden.map(at_bound).collect();
    let shocks = (g.data.axes.iter().enumerate())
        .find(|(_, a)| g.data.stability.as_ref().is_some_and(|s| s.shocks == a.id))
        .map_or((0, 0), at_bound);
    let catastrophes = symptom_gaps(&g.data, &c);
    let chosen = g.decisions.iter().filter_map(|d| match &d.kind {
        DecisionKind::EventChoice { event_id, .. } => Some(event_id.clone()),
        _ => None,
    });
    let events = chosen.chain(c.entries.iter().filter_map(|e| e.event.clone()));
    let events = events.collect();
    let laws_at_fall = c.entries.last().map_or(vec![], |e| {
        let laws = g.data.laws_in_force(&e.snapshot);
        laws.map(|l| l.id.clone()).collect()
    });
    let repeals = (c.entries.iter())
        .filter(|e| e.title == t.law_repealed.0)
        .count() as u32;
    let (successions, contested) = (crowned.clone().count(), crowned.filter(contested).count());
    let law_changes = laws.count() as u32;
    let (years, fall, army, treasury) = (
        c.years,
        c.fall.clone(),
        axis(&g.data.war.army),
        axis(&g.data.economy.treasury),
    );
    let designated = c.rulers.iter().filter(|r| r.designated).count() as u32;
    let start = Some(&g.data.war.start_event);
    let wars = c
        .entries
        .iter()
        .filter(|e| e.event.as_ref() == start)
        .count() as u32;
    let bastards = (c.kin.iter())
        .filter(|k| k.bastard && k.crowned.is_some())
        .count() as u32;
    // The first SCORE_AT years: a fall after them is no fall yet.
    if c.years > SCORE_AT {
        let year = |e: &sim::ChronicleEntry| e.tick.year(e.snapshot.time_unit);
        c.entries.retain(|e| year(e) < SCORE_AT);
        (c.years, c.fall) = (SCORE_AT, FallReason::Alive);
    }
    let score_at = score::compute(&c, &g.decisions, rules).total;
    Ok(Row {
        nodes,
        bounds,
        shocks,
        score_at,
        catastrophes,
        successions: successions as u32,
        contested: contested as u32,
        law_changes,
        laws: laws_at_fall,
        repeals,
        designated,
        bastards,
        seed,
        reign,
        years,
        score: s.total,
        army,
        treasury,
        deserted: c.deserted,
        fall: Some(fall),
        reign_treasury: log,
        events,
        wars,
    })
}

/// The CSV, then `#` lines: quartiles of the dynasty years, score, reign years, army and
/// treasury at the end, the reign's treasury at `TREASURY_AT`, the share of early deaths and
/// of dynasties whose army deserted, the fall reasons by frequency; the hidden nodes (`hidden`,
/// the order of `Row.nodes`) at `NODES_AT` and the fall, and their years at a bound; the score
/// of the first `SCORE_AT` years and the share of falls; per catastrophe (`catastrophes`) the
/// share of those with a symptom `SYMPTOM_YEARS` or more before (any, and one after the last
/// catastrophe of the kind). The CSV ends with the score
/// of `SCORE_AT` years and the years from the first symptom to the dynasty's first
/// catastrophe (empty: none, `-`: no symptom before it).
fn batch_report(rows: &[Row], hidden: &[&str], catastrophes: &[&str]) -> String {
    let mut out = String::from(
        "seed,reign_years,dynasty_years,score,fall_reason,early_death,army,treasury,deserted,\
         treasury_10,treasury_20,treasury_30",
    );
    for id in hidden {
        out += &format!(",{id}_{},{id}_{},{id}_fall", NODES_AT[0], NODES_AT[1]);
    }
    out += &format!(",score_{SCORE_AT},symptom_years\n");
    for r in rows {
        let early = r.reign < EARLY_YEARS;
        let (seed, reign, years, score, army) = (r.seed, r.reign, r.years, r.score, r.army);
        let fall = r.fall.as_ref().map_or(String::new(), |f| format!("{f:?}"));
        out += &format!("{seed},{reign},{years},{score},{fall},{early},{army},");
        out += &format!("{},{}", r.treasury, r.deserted);
        for y in TREASURY_AT {
            let t = r
                .reign_treasury
                .get(y - 1)
                .map_or(String::new(), |t| t.to_string());
            out += &format!(",{t}");
        }
        for v in r.nodes.iter().flatten() {
            out += &format!(",{}", v.map_or(String::new(), |v| v.to_string()));
        }
        let gap = match r.catastrophes.first() {
            None => String::new(),
            Some((_, None, _)) => "-".into(),
            Some((_, Some(y), _)) => y.to_string(),
        };
        out += &format!(",{},{gap}\n", r.score_at);
    }
    let quartiles = |mut v: Vec<i64>| {
        v.sort();
        let at = |q: usize| v.get(v.len() * q / 4).copied().unwrap_or(0);
        format!("{} / {} / {}", at(1), at(2), at(3))
    };
    out += &format!("# runs {}\n# квартили (25 / 50 / 75%):\n", rows.len());
    let of = |f: fn(&Row) -> i64| quartiles(rows.iter().map(f).collect());
    out += &format!("#   лет династии {}\n", of(|r| r.years as i64));
    out += &format!("#   счёт {}\n", of(|r| r.score));
    out += &format!("#   счёт за {SCORE_AT} лет {}\n", of(|r| r.score_at));
    out += &format!("#   лет правления {}\n", of(|r| r.reign as i64));
    out += &format!("#   армия в конце {}\n", of(|r| r.army));
    out += &format!("#   казна в конце {}\n", of(|r| r.treasury));
    for y in TREASURY_AT {
        let at = rows
            .iter()
            .filter_map(|r| r.reign_treasury.get(y - 1).copied());
        out += &format!(
            "#   казна к {y}-му году правления {}\n",
            quartiles(at.collect())
        );
    }
    let early = rows.iter().filter(|r| r.reign < EARLY_YEARS).count();
    let deserted = rows.iter().filter(|r| r.deserted > 0).count();
    out += &format!("# ранняя смерть {}%\n", percent(early, rows.len()));
    out += &format!(
        "# дезертирство в {}% династий\n",
        percent(deserted, rows.len())
    );
    let sum = |f: fn(&Row) -> u32| rows.iter().map(f).sum::<u32>() as usize;
    let disputed = rows.iter().filter(|r| r.contested > 0).count();
    out += &format!(
        "# спор о престоле: {}% воцарений, в {}% династий\n",
        percent(sum(|r| r.contested), sum(|r| r.successions)),
        percent(disputed, rows.len())
    );
    out += &format!(
        "# воцарения назначенных в обход закона: {}% воцарений\n",
        percent(sum(|r| r.designated), sum(|r| r.successions))
    );
    let bastards = rows.iter().filter(|r| r.bastards > 0).count();
    out += &format!(
        "# воцарения бастардов: {}% воцарений, в {}% династий\n",
        percent(sum(|r| r.bastards), sum(|r| r.successions)),
        percent(bastards, rows.len())
    );
    let changed = rows.iter().filter(|r| r.law_changes > 0).count();
    out += &format!(
        "# закон сменён после основателя в {}% династий\n",
        percent(changed, rows.len())
    );
    let repealed = rows.iter().filter(|r| r.repeals > 0).count();
    out += &format!(
        "# отмена закона в {}% династий; законы при падении:",
        percent(repealed, rows.len())
    );
    let mut laws: BTreeMap<&str, usize> = BTreeMap::new();
    for l in rows.iter().flat_map(|r| &r.laws) {
        *laws.entry(l).or_default() += 1;
    }
    for (l, n) in laws {
        out += &format!(" {l} {}%", percent(n, rows.len()));
    }
    out += "\n";
    if !hidden.is_empty() {
        let at = NODES_AT.map(|y| format!("{y}-м году"));
        out += &format!(
            "# скрытые узлы, квартили на {} / {} / при падении; на краях:\n",
            at[0], at[1]
        );
    }
    let permille = |(n, of): (usize, usize)| {
        format!(
            "{}.{}%",
            n * 1000 / of.max(1) / 10,
            n * 1000 / of.max(1) % 10
        )
    };
    for (k, id) in hidden.iter().enumerate() {
        let at = |j: usize| quartiles(rows.iter().filter_map(|r| r.nodes[k][j]).collect());
        let bounds = rows.iter().map(|r| r.bounds[k]);
        let bounds = bounds.fold((0, 0), |(a, b), (n, of)| (a + n, b + of));
        out += &format!(
            "#   {id} {} | {} | {} | {}\n",
            at(0),
            at(1),
            at(2),
            permille(bounds)
        );
    }
    let all = rows.iter().flat_map(|r| &r.bounds);
    let all = all.fold((0, 0), |(a, b), (n, of)| (a + n, b + of));
    if !hidden.is_empty() {
        out += &format!("# узло-лет на краях {}\n", permille(all));
    }
    let shocks = rows.iter().map(|r| r.shocks);
    let shocks = shocks.fold((0, 0), |(a, b), (n, of)| (a + n, b + of));
    if shocks.1 > 0 {
        out += &format!("# потрясения на краях {} лет\n", permille(shocks));
    }
    let fell = rows
        .iter()
        .filter(|r| r.fall != Some(FallReason::Alive))
        .count();
    out += &format!("# доля падений {}%\n", percent(fell, rows.len()));
    let years = rows.iter().map(|r| r.years as usize).sum::<usize>();
    let wars = (sum(|r| r.wars) * 1000).checked_div(years).unwrap_or(0);
    out += &format!("# войн после основателя на 1000 лет династии: {wars}\n");
    if !catastrophes.is_empty() {
        out += &format!("# симптом за {SYMPTOM_YEARS}+ лет до катастрофы:");
    }
    let early = |y: &Option<u32>| y.is_some_and(|y| y >= SYMPTOM_YEARS);
    for k in catastrophes {
        let all = rows
            .iter()
            .flat_map(|r| &r.catastrophes)
            .filter(|(c, ..)| c == k);
        let first = all.clone().filter(|(_, y, _)| early(y)).count();
        let since = all.clone().filter(|(.., y)| early(y)).count();
        let all = all.count();
        out += &format!(
            " {k} {}% (после прошлой {}%) из {all};",
            percent(first, all),
            percent(since, all)
        );
    }
    if !catastrophes.is_empty() {
        out += "\n";
    }
    out += "# причины падения:\n";
    let mut falls: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *falls
            .entry(format!("{:?}", r.fall.as_ref().expect("played")))
            .or_default() += 1;
    }
    let mut falls: Vec<_> = falls.into_iter().collect();
    falls.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (fall, n) in falls {
        out += &format!("#   {fall} {}%\n", percent(n, rows.len()));
    }
    out
}

fn percent(n: usize, of: usize) -> usize {
    (n * 100).checked_div(of).unwrap_or(0)
}

/// Every chronicle entry, its chain (nodes from the start, law, decision: text) and under it
/// each decision behind it: when, its tag and target, the weight of its marks left at the
/// entry, the event; at last how the dynasty ended.
pub fn trace(g: &Game, c: &sim::Chronicle) -> String {
    let w = &g.world;
    let mut out = String::new();
    for e in &c.entries {
        let date = e.tick.date(w.time_unit, w.start_year);
        let event = e.event.as_deref().unwrap_or("-");
        out += &format!("{date} {} [{event}]\n", e.title);
        if let Some(ch) = &e.chain {
            let nodes: Vec<&str> = ch.nodes.iter().map(|a| a.0.as_str()).collect();
            let law = ch.law.as_deref().unwrap_or("-");
            let decision = ch.decision.map_or("-".into(), |i| format!("#{i}"));
            out += &format!(
                "  цепочка {decision} {law} → {}: {}\n",
                nodes.join(" → "),
                ch.text
            );
        }
        if e.causes.is_empty() {
            out += "  без решений основателя\n";
        }
        for t in &e.causes {
            let d = &g.decisions[t.decision_idx];
            let target = match &d.kind {
                DecisionKind::EventChoice { target, .. }
                | DecisionKind::ActionStarted { target, .. } => target_name(target),
                DecisionKind::Abdicate | DecisionKind::Testament(_) => String::new(),
            };
            out += &format!(
                "  решение #{} (тик {}, {}{target}) → метка ({}) → {event}\n",
                t.decision_idx, d.tick.0, t.cause_tag, t.weight
            );
        }
    }
    out + &format!("конец {:?} на {}-м году\n", c.fall, c.years)
}

/// Every simulated year of `node`: its value, its target and the `Target` edges into it,
/// each with what it adds (in file order).
pub fn node_trace(g: &Game, c: &sim::Chronicle, node: &str) -> Result<String, String> {
    let d = &g.data;
    let pos = |id: &str| d.axes.iter().position(|a| a.id.0 == id);
    let i = pos(node).ok_or(format!("нет оси {node}"))?;
    let a = &d.axes[i];
    let edges = (d.influences.iter().enumerate())
        .filter(|(_, e)| e.to.0 == node && e.kind == crate::graph::InfluenceKind::Target);
    // The laws of a year: as at the last entry by then (the founder's end before the first).
    let year = |e: &sim::ChronicleEntry| e.tick.year(e.snapshot.time_unit);
    let laws =
        |y: u32| (c.entries.iter().rev().find(|e| year(e) <= y)).map_or(&g.world, |e| &e.snapshot);
    let mut out = String::new();
    for n in &c.nodes {
        let date = g.world.start_year + n.year;
        let w = laws(n.year);
        let parts: Vec<(&str, Fx)> = (edges.clone())
            .map(|(j, e)| {
                let src = match e.delay {
                    0 => n.axes[pos(&e.from.0).expect("checked on load")],
                    _ => n.lagged[j],
                };
                (
                    e.id.as_str(),
                    e.contribution(src) * crate::graph::scale(d, w, j),
                )
            })
            .collect();
        let anchor = crate::graph::anchor(d, w, a);
        let target = (parts.iter())
            .fold(anchor, |s, (_, v)| s + *v)
            .clamp(a.min, a.max);
        out += &format!("{date} {node} {} → {target}:", n.axes[i]);
        for (id, v) in parts {
            out += &format!(" {id} {}{v}", if v < Fx(0) { "" } else { "+" });
        }
        out += "\n";
    }
    Ok(out)
}

pub fn target_name(t: &Option<Target>) -> String {
    match t {
        Some(Target::Province(id)) => format!(", {}", id.0),
        Some(Target::Neighbour(id)) => format!(", {}", id.0),
        Some(Target::Heir(i)) => format!(", наследник {i}"),
        None => String::new(),
    }
}

/// RON as in the data files: `Province("capital")` for `Some(Province("capital"))`.
pub fn parse(text: &str) -> Result<Vec<ScriptStep>, String> {
    let options =
        ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    options.from_str(text).map_err(|e| format!("скрипт: {e}"))
}

/// The dynasty after an ended reign (the simulation goes on with the game's rng) and its
/// score; None while the reign goes on.
pub fn dynasty(g: &Game, rules: &ScoreRules) -> (Option<sim::Chronicle>, Option<score::Score>) {
    let Some(cause) = g.ended.clone() else {
        return (None, None);
    };
    let c = sim::run(g.reign_end(cause), &g.data, g.rng.clone());
    let s = score::compute(&c, &g.decisions, rules);
    (Some(c), Some(s))
}

/// `soft`: a Wait, an Action or a Testament while an event waits takes its middle choice instead of
/// failing, and a step
/// that does not fit the game (a script of another seed) is skipped. `log` gets
/// the treasury at the end of every year (see `note`).
pub fn play_script(
    g: &mut Game,
    script: &[ScriptStep],
    soft: bool,
    log: &mut Vec<i64>,
) -> Result<(), String> {
    for (i, step) in script.iter().enumerate() {
        // The rest of the script has no reign to act in.
        if g.ended.is_some() {
            break;
        }
        // Soft: an action waits for no event, it takes the middle choice first.
        if soft
            && let ScriptStep::Action(..) | ScriptStep::Testament(_) = step
            && let Some(choices) = pending_choices(g)
        {
            g.choose((choices.len() - 1) / 2).map_err(err)?;
        }
        let res = match step {
            ScriptStep::Wait(_) if g.pending_event.is_some() && !soft => {
                Err("событие ждёт выбора".into())
            }
            ScriptStep::Wait(n) => wait(g, *n, soft, log),
            ScriptStep::Action(id, target) => g.start_action(id, target.clone()).map_err(err),
            ScriptStep::Choose(idx) => g.choose(*idx).map_err(err),
            ScriptStep::ChooseByTag(tag) => match pending_choices(g) {
                Some(choices) => {
                    let idx = choices.iter().position(|c| c.cause_tag == *tag);
                    g.choose(idx.unwrap_or(0)).map_err(err)
                }
                None => Err("нет события".into()),
            },
            ScriptStep::Abdicate => g.abdicate().map_err(err),
            ScriptStep::Testament(t) => g.write_testament(t.clone()).map_err(err),
        };
        if !soft {
            res.map_err(|e| format!("шаг {} {step:?}: {e}", i + 1))?;
        }
    }
    Ok(())
}

/// Up to n ticks, stopping at an event; `soft` takes the middle choice of a waiting event
/// first and stops at none.
fn wait(g: &mut Game, n: u32, soft: bool, log: &mut Vec<i64>) -> Result<(), String> {
    for _ in 0..n {
        if soft && let Some(choices) = pending_choices(g) {
            g.choose((choices.len() - 1) / 2).map_err(err)?;
        }
        note(g, log);
        match g.wait().map_err(err)? {
            Step::Idle => {}
            Step::Event(_) if soft => {}
            Step::Event(_) | Step::ReignEnded(_) => break,
        }
    }
    Ok(())
}

/// Until the reign ends (at most `MAX_YEARS`): `auto` starts its best action before every tick
/// and makes every choice; without it (`neutral`) no actions and the middle choice. The
/// automaton's noise comes from a stream of its own (stage 26b): the game's rng goes only to
/// the game, so a link of the decisions plays it again.
pub fn play(g: &mut Game, auto: Option<&AutoChooser>, log: &mut Vec<i64>) -> Result<(), String> {
    let end = MAX_YEARS.ticks(g.world.time_unit);
    let mut noise = Rng::from_seed(g.rng.clone().next_u64() ^ NOISE);
    while g.world.tick < end && g.ended.is_none() {
        note(g, log);
        if let Some(a) = auto
            && g.pending_event.is_none()
            && let Some((id, target)) = lent(g, &mut noise, |g| a.action(g))
        {
            g.start_action(&id, target).map_err(err)?;
        }
        match g.wait().map_err(err)? {
            Step::Idle => {}
            Step::Event(v) => {
                let idx = match auto {
                    Some(a) => lent(g, &mut noise, |g| a.choose(g, &v.choices)),
                    None => (v.choices.len() - 1) / 2,
                };
                g.choose(idx).map_err(err)?
            }
            Step::ReignEnded(_) => break,
        }
    }
    Ok(())
}

/// Salt of the automaton's noise stream in `play`.
const NOISE: u64 = 0x6e6f_6973_65;

/// `f` with `rng` lent to the game in place of its own.
fn lent<T>(g: &mut Game, rng: &mut Rng, f: impl FnOnce(&mut Game) -> T) -> T {
    std::mem::swap(&mut g.rng, rng);
    let r = f(g);
    std::mem::swap(&mut g.rng, rng);
    r
}

/// The treasury of every year the reign has finished and `log` misses, as it stands now:
/// called before a tick, it sees the year closed with its choices made.
fn note(g: &Game, log: &mut Vec<i64>) {
    let w = &g.world;
    let treasury = w.axes[&g.data.economy.treasury].0 / Fx::SCALE;
    while log.len() < w.tick.year(w.time_unit) as usize {
        log.push(treasury);
    }
}

fn pending_choices(g: &Game) -> Option<&[crate::rules::Choice]> {
    let p = g.pending_event.as_ref()?;
    let e = g.data.events.iter().find(|e| e.id == p.event_id)?;
    Some(&e.choices)
}

fn err(e: crate::game::GameError) -> String {
    format!("{e:?}")
}

/// FNV-1a over the Debug dump: maps are BTreeMaps and `Fx` prints as an integer, so the
/// text, and the hash, depend only on the World.
pub fn world_hash(g: &Game) -> String {
    let text = format!("{:?}", g.world);
    let hash = (text.bytes()).fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

/// The game of `preset` on `map` (keys of `files` too) from the data of `files`: `rules.ron`,
/// `events/*.ron`, `events/sim/*.ron` (each in key order), `hints.ron`, `actions.ron`,
/// `names.ron`. Every file is checked even when one before it fails; the errors by
/// `error_text`.
pub fn load(files: &Files, preset: &str, map: &str, seed: u64) -> Result<Game, Vec<String>> {
    let text = |name: &str| -> Result<&str, Vec<String>> {
        let t = files
            .get(name)
            .ok_or_else(|| vec![format!("{name}: нет файла")]);
        t.map(String::as_str)
    };
    let rules = text("rules.ron")?;
    let mut data =
        crate::data::load(rules).map_err(|e| vec![error_text("rules.ron", rules, &e)])?;
    let in_dir = |dir: &'static str| {
        files.iter().filter(move |(k, _)| {
            let rest = k.strip_prefix(dir).and_then(|r| r.strip_prefix('/'));
            rest.is_some_and(|r| !r.contains('/') && r.ends_with(".ron"))
        })
    };
    type Add = fn(&mut Data, &str) -> Result<(), DataError>;
    let dirs = [
        ("events", Data::add_events as Add),
        ("events/sim", Data::add_sim_events),
    ];
    let dirs = dirs
        .into_iter()
        .flat_map(|(d, add)| in_dir(d).map(move |(k, t)| (k.as_str(), add, Some(t))));
    let tops = [
        ("hints.ron", Data::add_hints as Add),
        ("actions.ron", Data::add_actions),
        ("names.ron", Data::add_names),
    ];
    let tops = tops.into_iter().map(|(k, add)| (k, add, files.get(k)));
    let mut errors = vec![];
    for (name, add, t) in dirs.chain(tops) {
        match t {
            Some(t) => errors.extend(add(&mut data, t).err().map(|e| error_text(name, t, &e))),
            None => errors.push(format!("{name}: нет файла")),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let (p, m) = (text(preset)?, text(map)?);
    let loaded = Preset::load_with_map(p, m, &data).map_err(|e| {
        // A parse error is of the map unless the preset fails alone.
        let own = matches!(Preset::load(p, &data), Err(DataError::Parse(_)));
        let (file, t) = if own { (preset, p) } else { (map, m) };
        vec![error_text(file, t, &e)]
    })?;
    Ok(Game::new(data, &loaded, seed))
}

/// `file: message`; for a parse error `file:line:col: message [path]`, the path by
/// `field_path`.
pub fn error_text(file: &str, text: &str, e: &DataError) -> String {
    match e {
        DataError::Parse(e) => {
            let at = &e.span.start;
            let path = field_path(text, at.line, at.col);
            format!("{file}:{at}: {} [{path}]", e.code)
        }
        DataError::Invalid(m) => format!("{file}: {m}"),
    }
}

/// Where the RON `text` stands at `line`:`col` (1-based, in chars, as ron counts): the
/// struct fields, map keys and list indices around it, `[3].choices[1].effects`.
pub fn field_path(text: &str, line: usize, col: usize) -> String {
    enum Frame {
        List(usize),
        Struct(String),
        Map(String),
    }
    let mut stack: Vec<Frame> = vec![];
    let (mut word, mut string) = (String::new(), String::new());
    let mut chars = text.chars().peekable();
    let (mut l, mut c) = (1, 1);
    while (l, c) < (line, col) {
        let Some(ch) = chars.next() else { break };
        (l, c) = if ch == '\n' { (l + 1, 1) } else { (l, c + 1) };
        match ch {
            '"' => {
                string.clear();
                while let Some(s) = chars.next() {
                    (l, c) = if s == '\n' { (l + 1, 1) } else { (l, c + 1) };
                    match s {
                        '"' => break,
                        '\\' => {
                            chars.next();
                            c += 1;
                        }
                        _ => string.push(s),
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for s in chars.by_ref() {
                    if s == '\n' {
                        (l, c) = (l + 1, 1);
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                let mut prev = ' ';
                for s in chars.by_ref() {
                    (l, c) = if s == '\n' { (l + 1, 1) } else { (l, c + 1) };
                    if prev == '*' && s == '/' {
                        break;
                    }
                    prev = s;
                }
            }
            '[' => stack.push(Frame::List(0)),
            '(' => stack.push(Frame::Struct(String::new())),
            '{' => stack.push(Frame::Map(String::new())),
            ']' | ')' | '}' => {
                stack.pop();
            }
            ',' => match stack.last_mut() {
                Some(Frame::List(i)) => *i += 1,
                Some(Frame::Struct(f) | Frame::Map(f)) => f.clear(),
                None => {}
            },
            ':' => match stack.last_mut() {
                Some(Frame::Struct(f)) => *f = word.clone(),
                Some(Frame::Map(k)) => *k = string.clone(),
                _ => {}
            },
            _ if ch.is_alphanumeric() || ch == '_' => {
                word.push(ch);
                continue;
            }
            _ => {}
        }
        if !ch.is_whitespace() {
            word.clear();
        }
    }
    let mut out = String::new();
    for f in stack {
        match f {
            Frame::List(i) => out += &format!("[{i}]"),
            Frame::Struct(f) if !f.is_empty() => out += &format!(".{f}"),
            Frame::Map(k) if !k.is_empty() => out += &format!("[\"{k}\"]"),
            _ => {}
        }
    }
    out
}

/// The score rules of `score.ron`.
pub fn score_rules(files: &Files, g: &Game) -> Result<ScoreRules, String> {
    let text = files.get("score.ron").ok_or("score.ron: нет файла")?;
    score::load(text, &g.data).map_err(|e| error_text("score.ron", text, &e))
}

/// None for `neutral`; otherwise the founder's `AutoChooser` plus the weights of `name` in
/// `strategies.ron`.
pub fn chooser(files: &Files, g: &Game, name: &str) -> Result<Option<AutoChooser>, String> {
    if name == NEUTRAL {
        return Ok(None);
    }
    let text = files
        .get("strategies.ron")
        .ok_or("strategies.ron: нет файла")?;
    let all: Strategies = ron::from_str(text).map_err(|e| format!("strategies.ron: {e}"))?;
    let extra = all.get(name).ok_or(format!("нет стратегии {name}"))?;
    let mut auto = AutoChooser::for_ruler(&g.data, &g.world.ruler);
    for (k, v) in extra {
        let w = auto.weights.entry(k.clone()).or_default();
        *w = *w + *v;
    }
    Ok(Some(auto))
}

/// `law` of rules.ron `laws` in force from the start in place of its group's, as if brought
/// in then; its one-off effects aside.
pub fn set_law(g: &mut Game, law: &str) -> Result<(), String> {
    let d = &g.data;
    let l = d.law(law).ok_or(format!("нет закона {law}"))?;
    let w = &mut g.world;
    for o in (d.laws.list.iter()).filter(|o| !l.group.is_empty() && o.group == l.group) {
        w.flags.remove(&o.id);
        w.laws.remove(&o.id);
    }
    w.flags.insert(law.to_string());
    w.laws.insert(law.to_string(), w.tick);
    Ok(())
}

/// `cli batch`: the games of `seeds` from `start` (`progress` after each one), the CSV and
/// summary of `batch_report`, and the rows.
pub fn batch(
    start: &Game,
    seeds: std::ops::Range<u64>,
    script: &[ScriptStep],
    auto: Option<&AutoChooser>,
    rules: &ScoreRules,
    mut progress: impl FnMut(u64),
) -> Result<(String, Vec<Row>), String> {
    let mut rows = vec![];
    for seed in seeds {
        rows.push(batch_row(start, seed, script, auto, rules)?);
        progress(seed);
    }
    let hidden = hidden_nodes(&start.data).map(|(_, a)| a.id.0.as_str());
    let hidden: Vec<&str> = hidden.collect();
    let catastrophes = start.data.symptoms.iter().map(|(c, _)| c.as_str());
    let catastrophes: Vec<&str> = catastrophes.collect();
    Ok((batch_report(&rows, &hidden, &catastrophes), rows))
}

/// The chronicle as `cli run` tells it: every entry with its hint, the epilogue, the rulers
/// with their lives.
pub fn chronicle_text(g: &Game, c: &sim::Chronicle) -> String {
    let w = &g.world;
    let date = |t: crate::time::Tick| t.date(w.time_unit, w.start_year);
    let mut out = String::from("Хроника:\n");
    for e in &c.entries {
        let hint = e.hint.as_deref().map_or(String::new(), |h| format!(" {h}"));
        out += &format!("  {} {}. {}{hint}\n", date(e.tick), e.title, e.text);
    }
    if !c.epilogue.is_empty() {
        out += &format!("  {}\n", c.epilogue);
    }
    out += "Правители:\n";
    for r in &c.rulers {
        out += &format!("  {}, {}–{}\n", r.full_name(), date(r.start), date(r.end));
        if !r.biography.is_empty() {
            out += &format!("    {}\n", r.biography);
        }
    }
    out
}

/// `cli trace`: `script` (soft), then the neutral strategy until the reign ends; every
/// chronicle entry with its chain (`trace`), or with `node` that axis year by year
/// (`node_trace`); with `realm`, of that foreign kingdom's chronicle (stage 26).
pub fn trace_of(
    mut g: Game,
    rules: &ScoreRules,
    script: &[ScriptStep],
    node: Option<&str>,
    realm: Option<&str>,
) -> Result<String, String> {
    play_script(&mut g, script, true, &mut vec![])?;
    play(&mut g, None, &mut vec![])?;
    let (Some(ours), _) = dynasty(&g, rules) else {
        return Err("правление не кончилось".into());
    };
    let c = match realm {
        Some(id) => (ours.realms.get(&crate::state::NeighbourId(id.into())))
            .ok_or(format!("нет королевства {id}"))?,
        None => &ours,
    };
    match node {
        Some(node) => node_trace(&g, c, node),
        None => Ok(trace(&g, c)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::replay;
    use crate::time::Tick;

    const RULES: &str = include_str!("../../../data/rules.ron");
    const PRESET: &str = include_str!("../../../data/presets/default.ron");
    const MAP: &str = include_str!("../../../data/maps/default.ron");

    /// An event every tick, with tags "a", "b", "c"; one free action "act".
    fn game() -> Game {
        let mut data = crate::data::load(RULES).unwrap();
        data.quiet_weight = 0;
        let choice = |t: &str| format!("(text: \"{t}\", effects: [], cause_tag: \"{t}\")");
        let choices = ["a", "b", "c"].map(choice).join(", ");
        data.add_events(&format!(
            "[(id: \"e\", title: \"\", text: \"\", when: All([]), weight: 1, once: false, \
             cooldown_years: 0, importance: 1, target: None, choices: [{choices}])]"
        ))
        .unwrap();
        data.add_actions(
            "[(id: \"act\", name: \"\", duration_years: 1, cost: 0, requires: All([]), \
             min_crown_power: 0, target: None, on_complete: [], cause_tag: \"act\")]",
        )
        .unwrap();
        let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
        Game::new(data, &preset, 7)
    }

    fn tags(g: &Game) -> Vec<&str> {
        g.decisions.iter().map(|d| d.cause_tag.as_str()).collect()
    }

    fn script(text: &str) -> Vec<ScriptStep> {
        parse(text).unwrap()
    }

    #[test]
    fn script_steps() {
        let mut g = game();
        let s = script(
            "[Action(\"act\", None), Wait(5), ChooseByTag(\"c\"), Wait(1), ChooseByTag(\"x\"), \
             Wait(1), Choose(1)]",
        );
        play_script(&mut g, &s, false, &mut vec![]).unwrap();
        // Each Wait stopped at the event of its first tick.
        assert_eq!(g.world.tick, Tick(3));
        assert_eq!(tags(&g), ["act", "c", "a", "b"]);
        assert_eq!(
            script("[Action(\"x\", Province(\"capital\"))]"),
            [ScriptStep::Action(
                "x".into(),
                Some(Target::Province(crate::state::ProvinceId("capital".into())))
            )]
        );
    }

    #[test]
    fn script_errors() {
        let fails =
            |text: &str| play_script(&mut game(), &script(text), false, &mut vec![]).unwrap_err();
        assert!(fails("[Choose(0)]").contains("NoEvent"));
        assert!(fails("[ChooseByTag(\"a\")]").contains("нет события"));
        assert!(fails("[Wait(1), Wait(1)]").contains("событие ждёт выбора"));
        assert!(fails("[Wait(1), Choose(3)]").contains("BadChoice"));
        assert!(fails("[Action(\"nope\", None)]").contains("Unknown"));
        assert!(fails("[Wait(1), Abdicate]").contains("EventPending"));
        assert!(parse("[Jump]").is_err());
    }

    #[test]
    fn soft_wait_takes_the_middle_and_goes_on() {
        let mut g = game();
        let steps = "[Wait(5), Wait(1), Action(\"nope\", None)]";
        play_script(&mut g, &script(steps), true, &mut vec![]).unwrap();
        // An event every tick: each one waiting when the next tick or an action comes takes
        // the middle; the unknown action is skipped.
        assert_eq!(g.world.tick, Tick(6));
        assert_eq!(tags(&g), ["b"; 6]);
        assert!(g.pending_event.is_none());
    }

    #[test]
    fn batch_summary() {
        let row = |seed, reign: u32, score, fall, deserted| Row {
            seed,
            reign,
            years: 100,
            score,
            fall: Some(fall),
            army: reign as i64,
            treasury: score * 10,
            deserted,
            reign_treasury: (1..=reign as i64).collect(),
            successions: 4,
            contested: seed as u32,
            law_changes: (seed == 3) as u32,
            laws: (seed < 2)
                .then(|| "law_x".to_string())
                .into_iter()
                .collect(),
            repeals: (seed == 0) as u32,
            designated: (seed == 1) as u32,
            bastards: 2 * (seed == 2) as u32,
            nodes: vec![],
            bounds: vec![],
            shocks: (0, 0),
            score_at: score / 2,
            wars: seed as u32,
            catastrophes: match seed {
                1 => vec![
                    ("war".into(), Some(25), Some(25)),
                    ("war".into(), Some(40), Some(10)),
                ],
                2 => vec![("war".into(), None, None)],
                3 => vec![("plague".into(), Some(3), Some(3))],
                _ => vec![],
            },
            events: BTreeSet::new(),
        };
        let rows = [
            row(0, 5, 10, FallReason::NoHeir, 0),
            row(1, 30, 30, FallReason::Usurped, 2),
            row(2, 40, 20, FallReason::Usurped, 0),
            row(3, 20, 40, FallReason::Alive, 0),
        ];
        let out = batch_report(&rows, &[], &[]);
        let mut lines = out.lines();
        assert_eq!(
            lines.next(),
            Some(
                "seed,reign_years,dynasty_years,score,fall_reason,early_death,army,treasury,\
                 deserted,treasury_10,treasury_20,treasury_30,score_150,symptom_years"
            )
        );
        // The last two: the score of 150 years, the years from the first symptom to the first
        // catastrophe (empty: none; `-`: no symptom before it).
        assert_eq!(lines.next(), Some("0,5,100,10,NoHeir,true,5,100,0,,,,5,"));
        assert_eq!(
            lines.next(),
            Some("1,30,100,30,Usurped,false,30,300,2,10,20,30,15,25")
        );
        assert_eq!(
            lines.next(),
            Some("2,40,100,20,Usurped,false,40,200,0,10,20,30,10,-")
        );
        let summary: Vec<_> = lines.skip(1).collect();
        assert_eq!(
            summary,
            [
                "# runs 4",
                "# квартили (25 / 50 / 75%):",
                "#   лет династии 100 / 100 / 100",
                "#   счёт 20 / 30 / 40",
                "#   счёт за 150 лет 10 / 15 / 20",
                "#   лет правления 20 / 30 / 40",
                "#   армия в конце 20 / 30 / 40",
                "#   казна в конце 200 / 300 / 400",
                "#   казна к 10-му году правления 10 / 10 / 10",
                "#   казна к 20-му году правления 20 / 20 / 20",
                "#   казна к 30-му году правления 30 / 30 / 30",
                "# ранняя смерть 25%",
                "# дезертирство в 25% династий",
                "# спор о престоле: 37% воцарений, в 75% династий",
                "# воцарения назначенных в обход закона: 6% воцарений",
                "# воцарения бастардов: 12% воцарений, в 25% династий",
                "# закон сменён после основателя в 25% династий",
                "# отмена закона в 25% династий; законы при падении: law_x 50%",
                "# доля падений 75%",
                // Wars 0 + 1 + 2 + 3 in 400 years of dynasties (stage 24).
                "# войн после основателя на 1000 лет династии: 15",
                "# причины падения:",
                "#   Usurped 50%",
                "#   Alive 25%",
                "#   NoHeir 25%",
            ]
        );
        // A hidden node x: three columns, quartiles at 100 (none lived to 150) and at the
        // fall, its years at a bound (0 + 1 + 2 + 3 of 400).
        let rows = rows.map(|r| Row {
            nodes: vec![[Some(r.seed as i64 * 10), None, Some(5)]],
            bounds: vec![(r.seed as usize, 100)],
            shocks: (r.seed as usize, 100),
            ..r
        });
        let out = batch_report(&rows, &["x"], &["war", "plague"]);
        let mut lines = out.lines();
        assert!(
            lines
                .next()
                .unwrap()
                .ends_with(",treasury_30,x_100,x_150,x_fall,score_150,symptom_years")
        );
        assert_eq!(
            lines.next(),
            Some("0,5,100,10,NoHeir,true,5,100,0,,,,0,,5,5,")
        );
        // Of three wars two had a symptom 20 years before or more, one since the last war.
        let symptoms = "\n# симптом за 20+ лет до катастрофы: war 66% (после прошлой 33%) из 3; \
                        plague 0% (после прошлой 0%) из 1;\n";
        assert!(out.contains(symptoms), "{out}");
        assert!(
            out.contains("\n#   x 10 / 20 / 30 | 0 / 0 / 0 | 5 / 5 / 5 | 1.5%\n"),
            "{out}"
        );
        assert!(out.contains("\n# узло-лет на краях 1.5%\n"), "{out}");
        assert!(out.contains("\n# потрясения на краях 1.5% лет\n"), "{out}");
    }

    #[test]
    fn symptom_gaps_count_from_the_first_symptom_and_from_the_last_catastrophe() {
        let mut g = game();
        g.data.symptoms = vec![("war".into(), vec!["sign".into()])];
        let entry = |year: u32, id: &str| sim::ChronicleEntry {
            tick: Tick(year),
            event: Some(id.into()),
            title: String::new(),
            text: String::new(),
            hint: None,
            importance: 0,
            causes: vec![],
            snapshot: g.world.clone(),
            chain: None,
            news: false,
        };
        let entries = [
            (5, "sign"),
            (10, "war"),
            (30, "other"),
            (40, "war"),
            (45, "sign"),
        ];
        let c = sim::Chronicle {
            entries: entries.iter().map(|(y, id)| entry(*y, id)).collect(),
            fall: FallReason::Alive,
            years: 50,
            rulers: vec![],
            kin: vec![],
            axes: Default::default(),
            deserted: 0,
            nodes: vec![],
            epilogue: String::new(),
            realms: Default::default(),
        };
        let war = |a, b| ("war".to_string(), a, b);
        let want = [war(Some(5), Some(5)), war(Some(35), None)];
        assert_eq!(symptom_gaps(&g.data, &c), want);
    }

    #[test]
    fn neutral_takes_the_middle_and_no_actions() {
        let mut g = game();
        play(&mut g, None, &mut vec![]).unwrap();
        assert_eq!(g.world.tick, MAX_YEARS.ticks(g.world.time_unit));
        assert_eq!(g.decisions.len(), MAX_YEARS.0 as usize);
        assert!(tags(&g).iter().all(|t| *t == "b"));
    }

    #[test]
    fn replay_matches_the_game() {
        let mut g = game();
        let s = script("[Action(\"act\", None), Wait(1), Choose(2), Wait(1), Choose(0), Wait(1)]");
        play_script(&mut g, &s, false, &mut vec![]).unwrap();
        let mut r = game();
        replay(&mut r, &g.decisions, g.world.tick).unwrap();
        assert_eq!(r.world, g.world);
        assert_eq!(r.decisions, g.decisions);
        assert_eq!(world_hash(&r), world_hash(&g));
        assert_ne!(world_hash(&r), world_hash(&game()));

        // A choice missing from the journal, or one for another event, is a mismatch.
        let e = replay(&mut game(), &g.decisions[2..], g.world.tick).unwrap_err();
        assert!(e.contains("без выбора"), "{e}");
        let mut wrong = g.decisions.clone();
        if let DecisionKind::EventChoice { event_id, .. } = &mut wrong[1].kind {
            *event_id = "other".into();
        }
        let e = replay(&mut game(), &wrong, g.world.tick).unwrap_err();
        assert!(e.contains("ждали событие other"), "{e}");
    }

    #[test]
    fn the_dynasty_follows_an_ended_reign() {
        let mut g = game();
        let rules = score::load(include_str!("../../../data/score.ron"), &g.data).unwrap();
        assert_eq!(dynasty(&g, &rules), (None, None));
        g.ended = Some("illness".into());
        let (c, s) = dynasty(&g, &rules);
        let c = c.unwrap();
        assert!(!c.entries.is_empty());
        assert_eq!(s.unwrap(), score::compute(&c, &g.decisions, &rules));
    }

    #[test]
    fn field_path_names_fields_keys_and_indices() {
        let text = "// a \"quoted\" (comment)\n[\n  (id: \"a\", choices: []),\n  (id: \"b:c\", w: {\"k\": [1, 2, X]}, /* ( */ t: (1, 2)),\n]";
        // Line 4: the X of the map's list, then the tuple t.
        let x = text.lines().nth(3).unwrap().find('X').unwrap() + 1;
        assert_eq!(field_path(text, 4, x), "[1].w[\"k\"][2]");
        let t = text.lines().nth(3).unwrap().find("2))").unwrap() + 1;
        assert_eq!(field_path(text, 4, t), "[1].t");
        assert_eq!(field_path(text, 3, 9), "[0].id");
        assert_eq!(field_path(text, 3, 12), "[0]");
        assert_eq!(field_path(text, 1, 5), "");
    }

    fn files() -> Files {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/");
        let mut files = Files::new();
        for dir in ["", "events/", "events/sim/", "presets/", "maps/"] {
            for e in std::fs::read_dir(format!("{root}{dir}")).unwrap() {
                let p = e.unwrap().path();
                if p.extension().is_some_and(|x| x == "ron") {
                    let name = p.file_name().unwrap().to_string_lossy();
                    files.insert(format!("{dir}{name}"), std::fs::read_to_string(&p).unwrap());
                }
            }
        }
        files
    }

    #[test]
    fn load_checks_every_file_and_points_at_the_field() {
        let files = files();
        let (p, m) = ("presets/default.ron", "maps/default.ron");
        assert!(load(&files, p, m, 1).is_ok());
        let mut broken = files.clone();
        let war = broken["events/war.ron"].replacen("weight: ", "weight: \"x\", w: ", 1);
        broken.insert("events/war.ron".into(), war);
        let names = broken["names.ron"].replacen("rulers: [", "rulers: [1, ", 1);
        broken.insert("names.ron".into(), names);
        broken.remove("actions.ron");
        let e = load(&broken, p, m, 1).err().unwrap();
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(
            e[0].starts_with("events/war.ron:") && e[0].ends_with("[[0].weight]"),
            "{e:?}"
        );
        assert_eq!(e[1], "actions.ron: нет файла");
        assert!(
            e[2].starts_with("names.ron:") && e[2].ends_with("[.rulers[0]]"),
            "{e:?}"
        );
        // A broken map is the map's, a broken preset the preset's.
        let mut broken = files.clone();
        broken.insert(m.into(), broken[m].replacen("income: 7", "income: x", 1));
        let e = load(&broken, p, m, 1).err().unwrap();
        assert!(
            e[0].starts_with("maps/default.ron:") && e[0].contains("[.provinces[0].income]"),
            "{e:?}"
        );
        let mut broken = files.clone();
        broken.insert(p.into(), broken[p].replacen("age: 32", "age: x", 1));
        let e = load(&broken, p, m, 1).err().unwrap();
        assert!(
            e[0].starts_with("presets/default.ron:") && e[0].contains("[.ruler.age]"),
            "{e:?}"
        );
        // Rules that do not hold.
        let mut broken = files.clone();
        broken.insert(
            "rules.ron".into(),
            broken["rules.ron"].replace("loyalty_axis: \"loyalty\"", "loyalty_axis: \"x\""),
        );
        assert_eq!(
            load(&broken, p, m, 1).err().unwrap(),
            ["rules.ron: loyalty_axis is not an axis"]
        );
    }
}
