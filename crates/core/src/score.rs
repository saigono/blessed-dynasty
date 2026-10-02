//! The score of a dynasty, read from its chronicle (DESIGN 8). Weights in `data/score.ron`.

use crate::data::{Data, DataError, parse};
use crate::fx::Fx;
use crate::game::Decision;
use crate::rules::Sign;
use crate::sim::{Chronicle, ChronicleEntry, FallReason};
use crate::state::{AxisId, Holder, World};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

/// The parts of the score, the keys of `ScoreRules.weights` and `Score.parts`.
pub const PARTS: [&str; 5] = [
    "years",
    "territory_years",
    "prestige",
    "stability",
    "legacy",
];
const DECISIVE: usize = 3;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ScoreRules {
    /// Points per unit of each part in `PARTS`, all of them.
    pub weights: BTreeMap<String, Fx>,
    pub prestige_axis: AxisId,
    /// A crisis: an entry of a `Bad` event with importance above this.
    pub crisis_importance: u32,
    /// Institutions: these flags, alive at the fall, count for `legacy`.
    pub legacy_flags: Vec<String>,
    /// The events with `sign: Good`, filled by `load`.
    #[serde(skip)]
    pub good_events: BTreeSet<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Score {
    pub total: i64,
    /// Points per part of `PARTS`.
    pub parts: BTreeMap<String, i64>,
    /// The decisions behind the most of the chronicle, at most three, heaviest first.
    pub decisive: Vec<DecisiveDecision>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DecisiveDecision {
    pub decision_idx: usize,
    pub decision: Decision,
    /// The weights of its marks over all entries: plus under `Good` events and provinces
    /// gained, minus under other events and provinces lost, none under a new ruler.
    pub weight: Fx,
}

/// Reads `data/score.ron`; the event signs come from `data` (reign and simulation events).
pub fn load(text: &str, data: &Data) -> Result<ScoreRules, DataError> {
    let mut rules: ScoreRules = parse(text)?;
    let keys: BTreeSet<&str> = rules.weights.keys().map(String::as_str).collect();
    if keys != BTreeSet::from(PARTS) {
        let m = format!("score weights: need exactly {PARTS:?}");
        return Err(DataError::Invalid(m));
    }
    if !data.axes.iter().any(|a| a.id == rules.prestige_axis) {
        let m = format!("score: unknown axis {}", rules.prestige_axis.0);
        return Err(DataError::Invalid(m));
    }
    let events = data.events.iter().chain(&data.sim_events);
    let good = events.filter(|e| e.sign == Sign::Good);
    rules.good_events = good.map(|e| e.id.clone()).collect();
    Ok(rules)
}

/// Parts from the entry snapshots. A year without a snapshot of its own takes the last one
/// before it; the years before the first entry (the founder's reign) take the first one.
/// - `years`: `Chronicle.years`.
/// - `territory_years`: provinces of the crown and its vassals, summed over the years.
/// - `prestige`: `prestige_axis`, summed over the years.
/// - `stability`: crises the dynasty outlived (its fall came in a later year, or never)
///   with no fewer provinces than the entry before.
/// - `legacy`: `legacy_flags` set in the last snapshot.
pub fn compute(c: &Chronicle, decisions: &[Decision], rules: &ScoreRules) -> Score {
    let year = |e: &ChronicleEntry| e.tick.year(e.snapshot.time_unit);
    let (mut territory, mut prestige, mut i) = (0, Fx(0), 0);
    for y in 0..c.years {
        while c.entries.get(i + 1).is_some_and(|e| year(e) <= y) {
            i += 1;
        }
        if let Some(e) = c.entries.get(i) {
            territory += realm(&e.snapshot);
            prestige = prestige + e.snapshot.axes[&rules.prestige_axis];
        }
    }

    let good =
        |e: &ChronicleEntry| (e.event.as_ref()).is_some_and(|id| rules.good_events.contains(id));
    let mut crises = 0;
    let mut before = c.entries.first().map_or(0, |e| realm(&e.snapshot));
    for e in &c.entries {
        let now = realm(&e.snapshot);
        let crisis = e.event.is_some() && !good(e) && e.importance > rules.crisis_importance;
        let outlived = c.fall == FallReason::Alive || year(e) < c.years;
        crises += (crisis && outlived && now >= before) as i64;
        before = now;
    }

    let flags = c.entries.last().map(|e| &e.snapshot.flags);
    let legacy = flags.map_or(0, |f| {
        rules.legacy_flags.iter().filter(|l| f.contains(*l)).count()
    });

    let raw = [
        Fx::from_int(c.years as i64),
        Fx::from_int(territory),
        prestige,
        Fx::from_int(crises),
        Fx::from_int(legacy as i64),
    ];
    let parts: BTreeMap<String, i64> = (PARTS.iter().zip(raw))
        .map(|(p, v)| (p.to_string(), (v * rules.weights[*p]).0 / Fx::SCALE))
        .collect();

    // decision -> (all its weight, signed weight)
    let mut by_decision: BTreeMap<usize, (Fx, Fx)> = BTreeMap::new();
    // The realm before the tick of the entry, and (tick, realm) of the entry before.
    let (mut earlier, mut last) = (None, None);
    for e in &c.entries {
        let now = realm(&e.snapshot);
        if let Some((tick, r)) = last
            && tick < e.tick
        {
            earlier = Some(r);
        }
        last = Some((e.tick, now));
        // An entry without an event is a province gained or lost, or a new ruler (neutral).
        let sign = match (&e.event, earlier) {
            (Some(_), _) if good(e) => 1,
            (Some(_), _) => -1,
            (None, Some(r)) => (now - r).signum(),
            (None, None) => 0,
        };
        for t in &e.causes {
            let (all, signed) = by_decision.entry(t.decision_idx).or_default();
            *all = *all + t.weight;
            *signed = *signed + Fx(t.weight.0 * sign);
        }
    }
    let mut decisive: Vec<_> = by_decision.into_iter().collect();
    // Stable: the earlier decision first on a tie.
    decisive.sort_by_key(|(_, (all, _))| Reverse(*all));
    let decisive = (decisive.into_iter().take(DECISIVE))
        .map(|(idx, (_, weight))| DecisiveDecision {
            decision_idx: idx,
            decision: decisions[idx].clone(),
            weight,
        })
        .collect();

    Score {
        total: parts.values().sum(),
        parts,
        decisive,
    }
}

/// Provinces of the crown and its vassals.
fn realm(w: &World) -> i64 {
    let ours = w
        .provinces
        .values()
        .filter(|p| !matches!(p.holder, Holder::Foreign(_)));
    ours.count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{DecisionKind, Game};
    use crate::state::{CauseTag, Preset, ProvinceId};
    use crate::time::Tick;

    const RULES: &str = include_str!("../../../data/rules.ron");
    const PRESET: &str = include_str!("../../../data/presets/default.ron");
    const MAP: &str = include_str!("../../../data/maps/default.ron");
    const SCORE: &str = include_str!("../../../data/score.ron");

    fn data() -> Data {
        let mut data = crate::data::load(RULES).unwrap();
        let event = |id: &str, sign: &str| {
            format!(
                "(id: \"{id}\", title: \"\", text: \"\", when: All([]), weight: 0, once: false, \
                 cooldown_years: 0, importance: 5, sign: {sign}, target: None, \
                 choices: [(text: \"\", effects: [], cause_tag: \"x\")])"
            )
        };
        let events = format!("[{}, {}]", event("good", "Good"), event("bad", "Bad"));
        data.add_events(&events).unwrap();
        data
    }

    fn rules() -> ScoreRules {
        load(SCORE, &data()).unwrap()
    }

    /// The preset world with `provinces` crown provinces and this prestige.
    fn world(provinces: usize, prestige: i64) -> World {
        let data = data();
        let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
        let mut w = Game::new(data, &preset, 0).world;
        let template = w.provinces.values().next().unwrap().clone();
        w.provinces = (0..provinces)
            .map(|i| (ProvinceId(format!("p{i}")), template.clone()))
            .map(|(id, p)| {
                (
                    id.clone(),
                    crate::state::Province {
                        id,
                        holder: Holder::Crown,
                        ..p
                    },
                )
            })
            .collect();
        w.axes
            .insert(AxisId("prestige".into()), Fx::from_int(prestige));
        w
    }

    fn entry(year: u32, w: &World) -> ChronicleEntry {
        let tick = Tick(year * w.time_unit.ticks_per_year);
        ChronicleEntry {
            tick,
            event: None,
            title: String::new(),
            text: String::new(),
            hint: None,
            importance: 0,
            causes: Vec::new(),
            snapshot: World { tick, ..w.clone() },
        }
    }

    fn event(year: u32, w: &World, id: &str, importance: u32) -> ChronicleEntry {
        ChronicleEntry {
            event: Some(id.into()),
            importance,
            ..entry(year, w)
        }
    }

    fn chronicle(years: u32, fall: FallReason, entries: Vec<ChronicleEntry>) -> Chronicle {
        Chronicle {
            entries,
            fall,
            years,
            rulers: Vec::new(),
            kin: Vec::new(),
            axes: Default::default(),
            deserted: 0,
        }
    }

    #[test]
    fn a_long_small_dynasty_beats_a_short_great_one() {
        let r = rules();
        // 100 years at 10 provinces; the founder's 30 years take the first snapshot.
        let long = chronicle(100, FallReason::NoHeir, vec![entry(30, &world(10, 10))]);
        // 40 years: 4 provinces, a peak of 20 for 5 years, 4 again; 6 on average.
        let short = chronicle(
            40,
            FallReason::NoHeir,
            vec![
                entry(0, &world(4, 10)),
                entry(10, &world(20, 10)),
                entry(15, &world(4, 10)),
            ],
        );
        let (long, short) = (compute(&long, &[], &r), compute(&short, &[], &r));
        assert_eq!(long.parts["territory_years"], 2000);
        assert_eq!(short.parts["territory_years"], 480);
        assert_eq!(long.parts["years"], 4000);
        assert_eq!(long.parts["prestige"], 200);
        assert_eq!(short.parts["prestige"], 80);
        assert!(long.total > short.total, "{long:?} vs {short:?}");
        assert_eq!(long.total, long.parts.values().sum::<i64>());
    }

    #[test]
    fn the_same_chronicle_gives_the_same_score() {
        let r = rules();
        let w = world(10, 10);
        let c = chronicle(
            50,
            FallReason::Usurped,
            vec![entry(0, &w), event(20, &w, "bad", 5)],
        );
        let copy: Chronicle = crate::data::parse(&ron::to_string(&c).unwrap()).unwrap();
        assert_eq!(compute(&c, &[], &r), compute(&copy, &[], &r));
    }

    #[test]
    fn decisive_are_the_three_heaviest_decisions_signed_by_their_entries() {
        let w = world(10, 0);
        let tag = |idx: usize, weight: i64| CauseTag {
            decision_idx: idx,
            cause_tag: format!("d{idx}"),
            weight: Fx::from_milli(weight),
        };
        let marked = |id: &str, causes: Vec<CauseTag>| ChronicleEntry {
            causes,
            ..event(1, &w, id, 5)
        };
        let entries = vec![
            marked("good", vec![tag(1, 2000), tag(0, 500), tag(4, 100)]),
            marked("bad", vec![tag(0, 500), tag(2, 300), tag(3, 300)]),
            // A new ruler: no event and the same realm, neutral.
            ChronicleEntry {
                causes: vec![tag(2, 100)],
                ..entry(2, &w)
            },
        ];
        let decisions: Vec<Decision> = (0..5)
            .map(|i| Decision {
                tick: Tick(i),
                kind: DecisionKind::Abdicate,
                cause_tag: format!("d{i}"),
            })
            .collect();
        let s = compute(
            &chronicle(10, FallReason::NoHeir, entries),
            &decisions,
            &rules(),
        );
        let got: Vec<(usize, Fx)> = s
            .decisive
            .iter()
            .map(|d| (d.decision_idx, d.weight))
            .collect();
        // By all their weight: 1 (2.0), 0 (1.0, half good, half bad), 2 (0.4); 3 (0.3) is out.
        assert_eq!(got, [(1, Fx(2000)), (0, Fx(0)), (2, Fx(-300))]);
        assert_eq!(s.decisive[0].decision, decisions[1]);
    }

    #[test]
    fn entries_without_an_event_are_signed_by_the_realm() {
        let tagged = |year: u32, provinces: usize, idx: usize| ChronicleEntry {
            causes: vec![CauseTag {
                decision_idx: idx,
                cause_tag: String::new(),
                weight: Fx::from_int(1),
            }],
            ..entry(year, &world(provinces, 0))
        };
        let entries = vec![
            tagged(0, 10, 0), // the first entry: nothing earlier, neutral
            tagged(5, 11, 1), // a province gained
            tagged(6, 9, 2),  // two lost in one tick: both against the realm of year 5
            tagged(6, 9, 3),
            tagged(7, 9, 4), // a new ruler
        ];
        let decisions: Vec<Decision> = (0..5)
            .map(|i| Decision {
                tick: Tick(i),
                kind: DecisionKind::Abdicate,
                cause_tag: String::new(),
            })
            .collect();
        let c = chronicle(10, FallReason::NoHeir, entries);
        let s = compute(&c, &decisions, &rules());
        let got: Vec<(usize, Fx)> = s
            .decisive
            .iter()
            .map(|d| (d.decision_idx, d.weight))
            .collect();
        assert_eq!(got, [(0, Fx(0)), (1, Fx(1000)), (2, Fx(-1000))]);
    }

    #[test]
    fn stability_counts_bad_crises_outlived_without_losses() {
        let r = rules();
        let (ten, nine) = (world(10, 0), world(9, 0));
        let stability = |fall: FallReason, entries: Vec<ChronicleEntry>| {
            compute(&chronicle(50, fall, entries), &[], &r).parts["stability"]
        };
        let alive = FallReason::Alive;
        assert_eq!(stability(alive.clone(), vec![event(5, &ten, "bad", 4)]), 20);
        // Importance at the threshold is no crisis; a good event is none either.
        assert_eq!(stability(alive.clone(), vec![event(5, &ten, "bad", 3)]), 0);
        assert_eq!(stability(alive.clone(), vec![event(5, &ten, "good", 5)]), 0);
        // A new ruler or a province is no crisis, whatever its importance.
        let notable = ChronicleEntry {
            importance: 5,
            ..entry(5, &ten)
        };
        assert_eq!(stability(alive.clone(), vec![notable]), 0);
        // A province lost since the entry before.
        let lost = vec![entry(1, &ten), event(5, &nine, "bad", 5)];
        assert_eq!(stability(alive.clone(), lost), 0);
        // The dynasty fell the year of the crisis, or outlived it.
        assert_eq!(
            stability(FallReason::NoHeir, vec![event(50, &ten, "bad", 5)]),
            0
        );
        assert_eq!(
            stability(FallReason::NoHeir, vec![event(49, &ten, "bad", 5)]),
            20
        );
    }

    #[test]
    fn legacy_counts_institutions_alive_in_the_last_snapshot() {
        let r = rules();
        let mut early = world(10, 0);
        early.flags = ["law_elective", "cathedral"].map(String::from).into();
        let mut late = early.clone();
        late.flags = ["cathedral", "plague"].map(String::from).into();
        let c = chronicle(
            50,
            FallReason::NoHeir,
            vec![entry(0, &early), entry(9, &late)],
        );
        assert_eq!(compute(&c, &[], &r).parts["legacy"], 100);
        let c = chronicle(50, FallReason::NoHeir, Vec::new());
        assert_eq!(compute(&c, &[], &r).parts["legacy"], 0);
    }

    #[test]
    fn load_checks_weights_and_axis_and_reads_event_signs() {
        let r = rules();
        assert!(r.good_events.contains("good") && !r.good_events.contains("bad"));
        let bad = SCORE.replace("\"legacy\": 100,", "");
        assert!(load(&bad, &data()).is_err());
        let bad = SCORE.replace("\"legacy\": 100,", "\"legacy\": 60, \"luck\": 1,");
        assert!(load(&bad, &data()).is_err());
        let bad = SCORE.replace("prestige_axis: \"prestige\"", "prestige_axis: \"glory\"");
        assert!(load(&bad, &data()).is_err());
    }
}
