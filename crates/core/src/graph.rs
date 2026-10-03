//! The influence graph (docs/design/hidden-state.html): every axis steps toward its target,
//! its anchor plus the `Target` edges into it; `Flow` edges add to stocks every year.

use crate::data::{AxisDef, Data, ENACT, LawDef, curve};
use crate::fx::Fx;
use crate::rules::add_axis;
use crate::state::{AxisId, CauseTag, MarkKey, World};
use serde::Deserialize;
use std::cmp::Reverse;

/// An edge of `rules.ron` `influences`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Influence {
    /// For the laws that scale it and for the loops; may be empty.
    #[serde(default)]
    pub id: String,
    pub from: AxisId,
    pub to: AxisId,
    #[serde(default)]
    pub k: Fx,
    /// The source value at which the edge adds nothing.
    #[serde(default)]
    pub rest: Fx,
    /// `(source, contribution)` points (`data::curve`); when given, they replace
    /// `k * (source - rest)`.
    #[serde(default)]
    pub curve: Vec<(Fx, Fx)>,
    /// Years over which the source is smoothed (`World.lagged`); 0: read as it is.
    #[serde(default)]
    pub delay: u32,
    #[serde(default)]
    pub kind: InfluenceKind,
    /// Counts only while a law in force names it (`LawDef.edges`, `scale`).
    #[serde(default)]
    pub off: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
pub enum InfluenceKind {
    /// Adds to the target of `to`.
    #[default]
    Target,
    /// Adds to the stock `to` every year.
    Flow,
}

impl Influence {
    /// What the edge adds at source value `src`: by `curve` when given, else
    /// `k * (src - rest)`. To the target, or a year's flow.
    pub fn contribution(&self, src: Fx) -> Fx {
        match self.curve.is_empty() {
            true => self.k * (src - self.rest),
            false => curve(&self.curve, src),
        }
    }

    /// The source as edge `i` of `Data.influences` reads it now: smoothed over `delay`
    /// (`World.lagged`, the current value before the first tick) or as it is.
    pub fn source(&self, i: usize, w: &World) -> Fx {
        match (self.delay, w.lagged.get(i)) {
            (1.., Some(v)) => *v,
            _ => w.axes[&self.from],
        }
    }

    /// The edge's contribution now, as edge `i`.
    pub fn now(&self, i: usize, w: &World) -> Fx {
        self.contribution(self.source(i, w))
    }
}

/// Stability as a derived node (`rules.ron` `stability`): its anchor plus the `Target` edges
/// into it plus `shocks`, clamped, never a step. `Axis(axis, x)` in effects goes to
/// `shocks`, a plain axis that fades by its own step toward 0.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Stability {
    pub axis: AxisId,
    pub shocks: AxisId,
}

/// Recomputes the derived stability, if the data has one.
pub fn recompute_stability(d: &Data, w: &mut World) {
    let Some(s) = &d.stability else {
        return;
    };
    let a = d.axes.iter().find(|a| a.id == s.axis);
    let a = a.expect("checked on load");
    let edges = parts(d, w, &a.id, InfluenceKind::Target);
    let v = edges.fold(anchor(d, w, a), |s, (_, c)| s + c) + w.axes[&s.shocks];
    w.axes.insert(a.id.clone(), v.clamp(a.min, a.max));
}

/// Yearly step of `a` toward its target: `AxisDef.step`, else `drift.step` for a faction
/// axis, else 0. Derived axes never step.
pub fn step(d: &Data, a: &AxisDef) -> Fx {
    if d.is_derived(&a.id) || d.stability.as_ref().is_some_and(|s| s.axis == a.id) {
        return Fx(0);
    }
    let faction = d.factions.iter().any(|f| f.axis == a.id);
    a.step.unwrap_or(if faction { d.drift.step } else { Fx(0) })
}

/// The edges of `kind` into `to` with their index and contribution now (times `scale`), in
/// file order.
pub fn parts<'a>(
    d: &'a Data,
    w: &'a World,
    to: &'a AxisId,
    kind: InfluenceKind,
) -> impl Iterator<Item = (usize, Fx)> + 'a {
    let edges = d.influences.iter().enumerate();
    let into = edges.filter(move |(_, e)| e.kind == kind && e.to == *to);
    into.map(move |(i, e)| (i, e.now(i, w) * scale(d, w, i)))
}

/// The product of the multipliers the laws in force put on edge `i` (`LawDef.edges`); 0 for
/// an edge `off` that none of them names.
pub fn scale(d: &Data, w: &World, i: usize) -> Fx {
    let e = &d.influences[i];
    let (mut k, mut named) = (Fx::from_int(1), false);
    for l in &d.laws.list {
        for (_, m) in l.edges.iter().filter(|(id, _)| *id == e.id) {
            if w.flags.contains(&l.id) {
                (k, named) = (k * *m, true);
            }
        }
    }
    match e.off && !named {
        true => Fx(0),
        false => k,
    }
}

/// The anchor of `a` now: its own (`default` without one) plus the shifts of the laws in
/// force (`LawDef.anchors`), the resistance to those being brought in and the pressure of the
/// crown's land over its room (`CrownCapacity.pressure`).
pub fn anchor(d: &Data, w: &World, a: &AxisDef) -> Fx {
    let mut v = a.anchor.unwrap_or(a.default);
    let c = &d.crown_capacity;
    for (_, s) in c.pressure.iter().filter(|(x, _)| *x == a.id) {
        v = v + *s * Fx::from_int(c.excess(w));
    }
    for l in &d.laws.list {
        for (_, s) in l.anchors.iter().filter(|(x, _)| *x == a.id) {
            if w.flags.contains(&l.id) {
                v = v + *s;
            }
        }
    }
    for x in &w.active_actions {
        let law = x.id.strip_prefix(ENACT).and_then(|id| d.law(id));
        let against = law.into_iter().flat_map(|l| &l.resistance);
        for (_, s) in against.filter(|(x, _)| *x == a.id) {
            v = v + *s;
        }
    }
    v
}

/// The target of `a`: its `anchor` plus the `Target` edges into it, clamped to its bounds.
pub fn target(d: &Data, w: &World, a: &AxisDef) -> Fx {
    let edges = parts(d, w, &a.id, InfluenceKind::Target);
    edges
        .fold(anchor(d, w, a), |s, (_, c)| s + c)
        .clamp(a.min, a.max)
}

/// A push on the target of a node: a `Target` edge into it (its index in `Data.influences`)
/// or a law in force shifting its anchor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Push<'a> {
    Edge(usize),
    Law(&'a LawDef),
}

impl Push<'_> {
    /// Where the marks of what pushes lie: on the edge's source axis, on the law's flag.
    pub fn key(&self, d: &Data) -> MarkKey {
        match self {
            Push::Edge(i) => MarkKey::Axis(d.influences[*i].from.clone()),
            Push::Law(l) => MarkKey::Flag(l.id.clone()),
        }
    }
}

/// What pushes the target of `a` now and by how much: the `Target` edges into it, then the
/// laws in force shifting its anchor, in data order.
pub fn pushes<'a>(
    d: &'a Data,
    w: &'a World,
    a: &'a AxisId,
) -> impl Iterator<Item = (Push<'a>, Fx)> + 'a {
    let edges = parts(d, w, a, InfluenceKind::Target).map(|(i, c)| (Push::Edge(i), c));
    let laws = d.laws_in_force(w).flat_map(move |l| {
        let shifts = l.anchors.iter().filter(move |(x, _)| x == a);
        shifts.map(move |(_, s)| (Push::Law(l), *s))
    });
    edges.chain(laws)
}

/// The `n` largest pushes on the target of `a` either way, with their sign, the largest
/// first; on a tie in `pushes` order. Pushes of 0 are left out.
pub fn pressing<'a>(d: &'a Data, w: &'a World, a: &'a AxisId, n: usize) -> Vec<(Push<'a>, Fx)> {
    let mut all: Vec<_> = pushes(d, w, a).filter(|(_, c)| *c != Fx(0)).collect();
    all.sort_by_key(|(_, c)| Reverse(c.0.abs()));
    all.truncate(n);
    all
}

/// A push passes marks only above this either way (docs/design/hidden-state.html, section 8).
pub const MARK_FLOW: Fx = Fx::from_int(1);
/// A node keeps at most this many marks that came to it by `flow_marks`.
pub const MARKS_PER_KEY: usize = 4;

/// Yearly: the marks of what pushes a node (`pushes`, `Push::key`) pass to the node, each
/// times the share of its push in all the pushes on it, if the push is above `MARK_FLOW`
/// either way. A node keeps the heaviest mark of a decision, and at most `MARKS_PER_KEY`
/// marks, the heaviest, once one has come to it.
pub fn flow_marks(d: &Data, w: &mut World) {
    if w.marks.is_empty() {
        return;
    }
    let abs = |c: Fx| Fx(c.0.abs());
    let mut moved: Vec<(MarkKey, CauseTag)> = vec![];
    for a in &d.axes {
        let all: Vec<(Push, Fx)> = pushes(d, w, &a.id).collect();
        let total = all.iter().fold(Fx(0), |s, (_, c)| s + abs(*c));
        for (p, c) in all.iter().filter(|(_, c)| abs(*c) > MARK_FLOW) {
            let Some(tags) = w.marks.get(&p.key(d)) else {
                continue;
            };
            let share = abs(*c) / total;
            moved.extend(tags.iter().map(|t| {
                let t = CauseTag {
                    weight: t.weight * share,
                    ..t.clone()
                };
                (MarkKey::Axis(a.id.clone()), t)
            }));
        }
    }
    for (k, t) in moved.into_iter().filter(|(_, t)| t.weight > Fx(0)) {
        let tags = w.marks.entry(k).or_default();
        match tags.iter_mut().find(|x| x.decision_idx == t.decision_idx) {
            Some(x) => x.weight = x.weight.max(t.weight),
            None => tags.push(t),
        }
        if tags.len() > MARKS_PER_KEY {
            tags.sort_by_key(|x| Reverse(x.weight));
            tags.truncate(MARKS_PER_KEY);
        }
    }
}

/// The `Flow` edges into the treasury: they are part of the yearly income
/// (`war::income_parts`), paid with it in `Game::passive`.
pub fn treasury_flows<'a>(d: &'a Data, w: &'a World) -> impl Iterator<Item = Fx> + 'a {
    parts(d, w, &d.economy.treasury, InfluenceKind::Flow).map(|(_, c)| c)
}

/// Per tick, from `Game::passive`, yearly amounts spread over the ticks of a year: the
/// lagged sources move toward the sources, then every axis steps toward its target (all
/// targets taken before any step), then the flows other than the treasury's.
pub fn tick(d: &Data, w: &mut World) {
    let tpy = Fx::from_int(d.time_unit.ticks_per_year as i64);
    if w.lagged.len() != d.influences.len() {
        w.lagged = (d.influences.iter()).map(|e| w.axes[&e.from]).collect();
    }
    let (axes, lagged) = (&w.axes, &mut w.lagged);
    for (e, v) in d.influences.iter().zip(lagged.iter_mut()) {
        if e.delay > 0 {
            *v = *v + (axes[&e.from] - *v) / (Fx::from_int(e.delay as i64) * tpy);
        }
    }
    let steps: Vec<(&AxisDef, Fx, Fx)> = (d.axes.iter())
        .map(|a| (a, step(d, a) / tpy))
        .filter(|(_, s)| *s != Fx(0))
        .map(|(a, s)| (a, s, target(d, w, a)))
        .collect();
    for (a, s, to) in steps {
        let v = w.axes.get_mut(&a.id).expect("the world has every axis");
        *v = match *v < to {
            true => (*v + s).min(to),
            false => (*v - s).max(to),
        };
    }
    let flows: Vec<(&AxisId, Fx)> = (d.influences.iter().enumerate())
        .filter(|(_, e)| e.kind == InfluenceKind::Flow && e.to != d.economy.treasury)
        .map(|(i, e)| (&e.to, e.now(i, w) * scale(d, w, i) / tpy))
        .collect();
    for (to, v) in flows {
        add_axis(w, d, to, v);
    }
}

/// The world `years` on with no events, no actions ending and no rulers dying: only the
/// yearly income into the treasury and the graph (`tick`), as `Game::passive` pays and
/// steps them. A copy; `w` stays as it is.
pub fn forecast(d: &Data, w: &World, years: u32) -> World {
    let mut w = w.clone();
    let tpy = d.time_unit.ticks_per_year;
    for _ in 0..years * tpy {
        let income = crate::war::yearly_income(&w, d) / Fx::from_int(tpy as i64);
        add_axis(&mut w, d, &d.economy.treasury, income);
        tick(d, &mut w);
        w.recompute_loyalty(d);
    }
    w
}

/// The bureaucracy that shows the player the hidden nodes (`rules.ron` `reveal`): a hidden
/// axis opens in words at its `AxisDef.reveal`, every number and the far forecast at
/// `numbers`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Reveal {
    pub axis: AxisId,
    pub numbers: Fx,
}

/// How the player sees an axis now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sight {
    /// The value, its target and what pushes it in numbers.
    Numbers,
    /// A hidden node, open in words.
    Words,
    /// A hidden node still closed: this bureaucracy opens it; None, never.
    Closed(Option<Fx>),
}

/// How the player sees `a` at the bureaucracy of `w` (`Data.reveal`): a plain axis in
/// numbers; a hidden one closed below its `AxisDef.reveal`, in words from there, in numbers
/// from `Reveal.numbers`. Without `reveal` in the data hidden axes stay closed.
pub fn sight(d: &Data, w: &World, a: &AxisDef) -> Sight {
    if !a.hidden {
        return Sight::Numbers;
    }
    let b = d.reveal.as_ref().map(|r| (w.axes[&r.axis], r.numbers));
    match (a.reveal, b) {
        (Some(_), Some((b, numbers))) if b >= numbers => Sight::Numbers,
        (Some(at), Some((b, _))) if b >= at => Sight::Words,
        (at, _) => Sight::Closed(at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AxisId, Preset};

    pub(super) fn setup(rules: &str) -> (Data, World) {
        let data = crate::data::load(rules).unwrap();
        let preset = Preset::load_with_map(
            include_str!("../../../data/presets/default.ron"),
            include_str!("../../../data/maps/default.ron"),
            &data,
        )
        .unwrap();
        let w = World::from_preset(&data, &preset);
        (data, w)
    }

    pub(super) const RULES: &str = include_str!("../../../data/rules.ron");

    fn ax(s: &str) -> AxisId {
        AxisId(s.into())
    }

    fn def<'a>(d: &'a mut Data, id: &str) -> &'a mut AxisDef {
        d.axes.iter_mut().find(|a| a.id.0 == id).unwrap()
    }

    #[test]
    fn steps_default_by_kind() {
        let (mut d, _) = setup(RULES);
        let step = |d: &mut Data, id: &str| {
            let a = def(d, id).clone();
            step(d, &a)
        };
        assert_eq!(step(&mut d, "loyalty_nobles"), d.drift.step); // a faction
        assert_eq!(step(&mut d, "treasury"), Fx(0)); // a stock
        assert_eq!(step(&mut d, "loyalty"), Fx(0)); // derived
        def(&mut d, "army").step = Some(Fx(2_500));
        assert_eq!(step(&mut d, "army"), Fx(2_500));
    }

    #[test]
    fn a_step_toward_the_anchor_is_never_exceeded() {
        let (mut d, mut w) = setup(RULES);
        let a = def(&mut d, "army");
        (a.step, a.anchor) = (Some(Fx::from_int(4)), Some(Fx::from_int(60)));
        w.axes.insert(ax("army"), Fx::from_int(50));
        let mut seen = vec![];
        for _ in 0..4 {
            tick(&d, &mut w);
            seen.push(w.axes[&ax("army")].0 / Fx::SCALE);
        }
        assert_eq!(seen, [54, 58, 60, 60]);
        // From above, and spread over the ticks of a year.
        d.time_unit.ticks_per_year = 4;
        w.axes.insert(ax("army"), Fx::from_int(70));
        tick(&d, &mut w);
        assert_eq!(w.axes[&ax("army")], Fx::from_int(69));
        // The anchor is clamped to the bounds.
        def(&mut d, "army").anchor = Some(Fx::from_int(5000));
        let a = def(&mut d, "army").clone();
        assert_eq!(target(&d, &w, &a), Fx::from_int(1000));
    }

    fn edge(ron: &str) -> Influence {
        crate::data::parse(ron).unwrap()
    }

    fn army_target(d: &Data, w: &World) -> Fx {
        target(d, w, d.axes.iter().find(|a| a.id.0 == "army").unwrap())
    }

    #[test]
    fn a_target_edge_shifts_the_target_by_k_times_the_distance_from_rest() {
        let (mut d, mut w) = setup(RULES);
        d.influences = vec![edge(
            r#"(from: "legitimacy", to: "army", k: 0.5, rest: 40)"#,
        )];
        // Legitimacy 45 (preset): 50 + 0.5 * (45 - 40).
        assert_eq!(army_target(&d, &w), Fx(52_500));
        w.axes.insert(ax("legitimacy"), Fx::from_int(20));
        assert_eq!(army_target(&d, &w), Fx::from_int(40));
        // A second edge adds up; an edge of k < 0 pulls down.
        d.influences
            .push(edge(r#"(from: "prestige", to: "army", k: -1, rest: 0)"#));
        assert_eq!(army_target(&d, &w), Fx::from_int(20)); // prestige 20
        // The army steps toward it.
        def(&mut d, "army").step = Some(Fx::from_int(100));
        tick(&d, &mut w);
        assert_eq!(w.axes[&ax("army")], Fx::from_int(20));
    }

    #[test]
    fn an_edge_curve_replaces_k() {
        let (mut d, mut w) = setup(RULES);
        let e = r#"(from: "legitimacy", to: "army", k: 9, curve: [(50, 0), (100, 40)])"#;
        d.influences = vec![edge(e)];
        let at = |w: &mut World, v: i64| {
            w.axes.insert(ax("legitimacy"), Fx::from_int(v));
            army_target(&d, w) - Fx::from_int(50)
        };
        assert_eq!(at(&mut w, 45), Fx(0)); // flat below the first point
        assert_eq!(at(&mut w, 75), Fx::from_int(20));
        assert_eq!(at(&mut w, 100), Fx::from_int(40));
    }

    #[test]
    fn a_delay_smooths_the_source() {
        let (mut d, mut w) = setup(RULES);
        let e = r#"(from: "legitimacy", to: "army", k: 1, rest: 45, delay: 4)"#;
        d.influences = vec![edge(e)];
        tick(&d, &mut w); // the lag starts at the source
        assert_eq!(w.lagged, [Fx::from_int(45)]);
        w.axes.insert(ax("legitimacy"), Fx::from_int(85));
        assert_eq!(army_target(&d, &w), Fx::from_int(50)); // not yet
        tick(&d, &mut w);
        assert_eq!(army_target(&d, &w), Fx::from_int(60)); // a quarter of the way
        tick(&d, &mut w);
        assert_eq!(army_target(&d, &w), Fx::from_int(67) + Fx(500));
        // Without a delay the source counts at once.
        d.influences[0].delay = 0;
        assert_eq!(army_target(&d, &w), Fx::from_int(90));
    }

    #[test]
    fn a_flow_adds_to_its_stock() {
        let (mut d, mut w) = setup(RULES);
        let e = r#"(from: "legitimacy", to: "prestige", k: 0.1, rest: 5, kind: Flow)"#;
        d.influences = vec![edge(e)];
        tick(&d, &mut w);
        assert_eq!(w.axes[&ax("prestige")], Fx::from_int(24)); // 20 + 0.1 * (45 - 5)
        // Spread over the ticks of a year.
        d.time_unit.ticks_per_year = 4;
        tick(&d, &mut w);
        assert_eq!(w.axes[&ax("prestige")], Fx::from_int(25));
    }

    fn mark(idx: usize, weight: Fx) -> CauseTag {
        CauseTag {
            decision_idx: idx,
            cause_tag: format!("d{idx}"),
            weight,
        }
    }

    #[test]
    fn a_mark_passes_an_edge_above_the_threshold_and_not_below() {
        let (mut d, mut w) = setup(RULES);
        d.laws.list.clear();
        let e = r#"(from: "legitimacy", to: "army", k: 1, rest: 45)"#;
        d.influences = vec![edge(e)];
        let legitimacy = MarkKey::Axis(ax("legitimacy"));
        let army = MarkKey::Axis(ax("army"));
        let at = |d: &Data, w: &mut World, v: Fx| {
            w.marks = [(legitimacy.clone(), vec![mark(0, Fx::from_int(1))])].into();
            w.axes.insert(ax("legitimacy"), v);
            flow_marks(d, w);
            w.marks.get(&army).cloned()
        };
        let all = Some(vec![mark(0, Fx::from_int(1))]);
        assert_eq!(at(&d, &mut w, Fx(46_001)), all);
        assert_eq!(at(&d, &mut w, Fx(46_000)), None); // a push of 1 is not above 1
        assert_eq!(at(&d, &mut w, Fx(43_999)), all); // -1.001
        // Two pushes: each passes its share; the second, of 0.5, none.
        d.influences
            .push(edge(r#"(from: "prestige", to: "army", k: 0.025, rest: 0)"#));
        let got = at(&d, &mut w, Fx::from_int(46) + Fx(500)); // 1.5 of 2
        assert_eq!(got, Some(vec![mark(0, Fx(750))]));
        // A node keeps a decision's heaviest mark and at most four marks.
        let mut tags: Vec<CauseTag> = (1..=4).map(|i| mark(i, Fx(i as i64 * 100))).collect();
        tags.push(mark(0, Fx(900)));
        w.marks.insert(army.clone(), tags);
        w.marks
            .insert(legitimacy.clone(), vec![mark(0, Fx::from_int(1))]);
        flow_marks(&d, &mut w);
        let kept: Vec<_> = w.marks[&army].iter().map(|t| t.decision_idx).collect();
        assert_eq!(kept, [0, 4, 3, 2]);
        assert_eq!(w.marks[&army][0].weight, Fx(900));
    }

    #[test]
    fn a_forecast_is_deterministic_and_leaves_the_world_as_it_is() {
        let (d, mut w) = setup(RULES);
        // Something to move: nobles high pull serfdom up (e4), serfdom the nobles (e5).
        set(&mut w, &d, &[("loyalty_nobles", 90), ("grain", 20)]);
        let before = w.clone();
        let a = forecast(&d, &w, 30);
        assert_eq!(w, before);
        assert_eq!(forecast(&d, &w, 30), a);
        assert!(a.axes[&ax("serfdom")] > w.axes[&ax("serfdom")]);
        assert_eq!(a.axes[&ax("grain")], Fx::from_int(50)); // step 5: back in 6 years
        // 30 years at once or in two goes: the same; 0 years: the world itself.
        assert_eq!(forecast(&d, &forecast(&d, &w, 5), 25), a);
        assert_eq!(forecast(&d, &w, 0), w);
        // The treasury takes its income, as in a year of the reign.
        let income = crate::war::yearly_income(&w, &d);
        let next = forecast(&d, &w, 1).axes[&ax("treasury")];
        assert_eq!(next, w.axes[&ax("treasury")] + income);
        // Spread over the ticks of a year, the same years.
        let mut d4 = d.clone();
        d4.time_unit.ticks_per_year = 4;
        let a4 = forecast(&d4, &w, 30);
        let close = (a4.axes[&ax("serfdom")] - a.axes[&ax("serfdom")]).0.abs();
        assert!(close < 1_000, "{close}");
    }

    #[test]
    fn the_bureaucracy_opens_the_hidden_nodes_by_their_thresholds() {
        let (mut d, mut w) = setup(RULES);
        let seen = |d: &Data, w: &mut World, b: i64, id: &str| {
            w.axes.insert(ax("bureaucracy"), Fx::from_int(b));
            sight(d, w, d.axes.iter().find(|a| a.id.0 == id).unwrap())
        };
        let closed = |v: i64| Sight::Closed(Some(Fx::from_int(v)));
        // The table of section 7: 20 grain and trade, 40 serfdom and liberties, 55 faith
        // and literacy, 70 strata and mobility, 85 numbers.
        for (id, at) in [
            ("grain", 20),
            ("trade", 20),
            ("serfdom", 40),
            ("liberties", 40),
            ("faith", 55),
            ("literacy", 55),
            ("strata", 70),
            ("mobility", 70),
        ] {
            assert_eq!(seen(&d, &mut w, at - 1, id), closed(at), "{id}");
            assert_eq!(seen(&d, &mut w, at, id), Sight::Words, "{id}");
            assert_eq!(seen(&d, &mut w, 84, id), Sight::Words, "{id}");
            assert_eq!(seen(&d, &mut w, 85, id), Sight::Numbers, "{id}");
        }
        // A plain axis always in numbers; a hidden one without `reveal` never open.
        assert_eq!(seen(&d, &mut w, 0, "army"), Sight::Numbers);
        assert_eq!(seen(&d, &mut w, 100, "shocks"), Sight::Closed(None));
        // Without `reveal` in the data nothing hidden opens.
        d.reveal = None;
        assert_eq!(seen(&d, &mut w, 100, "grain"), closed(20));
        // The axis that reveals must be known.
        let block = r#"reveal: (axis: "bureaucracy""#;
        let rules = RULES.replacen(block, r#"reveal: (axis: "nothing""#, 1);
        assert!(crate::data::load(&rules).is_err());
    }

    #[test]
    fn pressing_is_the_two_largest_pushes_with_their_sign() {
        let (mut d, mut w) = setup(RULES);
        d.laws.list.clear();
        d.influences = vec![
            edge(r#"(id: "a", from: "legitimacy", to: "army", k: 1, rest: 45)"#),
            edge(r#"(id: "b", from: "prestige", to: "army", k: -1, rest: 0)"#),
            edge(r#"(id: "c", from: "income", to: "army", k: 1, rest: 0)"#),
        ];
        // Legitimacy 45, prestige 20, income 5 (preset): 0, -20, +5.
        let army = ax("army");
        let got = pressing(&d, &w, &army, 2);
        assert_eq!(
            got,
            [
                (Push::Edge(1), Fx::from_int(-20)),
                (Push::Edge(2), Fx::from_int(5))
            ]
        );
        w.axes.insert(ax("legitimacy"), Fx::from_int(75));
        let got = pressing(&d, &w, &army, 2);
        assert_eq!(
            got,
            [
                (Push::Edge(0), Fx::from_int(30)),
                (Push::Edge(1), Fx::from_int(-20))
            ]
        );
        // A law shifting the anchor pushes too.
        let (d, mut w) = setup(RULES);
        w.flags.insert("law_serfdom".into());
        let serfdom = ax("serfdom");
        let got = pressing(&d, &w, &serfdom, 2);
        let law = matches!(got[..], [(Push::Law(l), v)] if l.id == "law_serfdom" && v == Fx::from_int(30));
        assert!(law, "{got:?}");
    }

    #[test]
    fn edges_need_known_plain_axes() {
        let with = |e: &str| {
            let edges = format!("    influences: [{e}, ");
            crate::data::load(&RULES.replacen("    influences: [", &edges, 1))
        };
        assert!(with(r#"(from: "army", to: "prestige")"#).is_ok());
        assert!(with(r#"(from: "nothing", to: "army")"#).is_err());
        assert!(with(r#"(from: "army", to: "nothing")"#).is_err());
        assert!(with(r#"(from: "army", to: "loyalty")"#).is_err()); // derived
    }

    #[test]
    fn a_flow_into_the_treasury_is_income_as_the_old_flows() {
        let (mut d, w) = setup(include_str!("../tests/main/data/rules.ron"));
        let old = crate::war::income_parts(&w, &d);
        assert_eq!(d.economy.flows, [(ax("income"), Fx::from_int(1))]);
        d.economy.flows.clear();
        assert_ne!(crate::war::income_parts(&w, &d), old);
        d.influences = vec![edge(
            r#"(from: "income", to: "treasury", k: 1, kind: Flow)"#,
        )];
        assert_eq!(crate::war::income_parts(&w, &d), old);
    }

    fn set(w: &mut World, d: &Data, axes: &[(&str, i64)]) {
        for (a, v) in axes {
            w.axes.insert(ax(a), Fx::from_int(*v));
        }
        w.recompute_loyalty(d);
    }

    #[test]
    fn stability_is_derived_by_the_formula() {
        let (d, mut w) = setup(RULES);
        // Loyalty (40 * 2 + 60 + 50) / 4 = 47.5, legitimacy 45: 50 - 1.5 - 2 = 46.5; the
        // preset's 55 is where it starts, the gap of 8.5 a shock.
        assert_eq!(w.axes[&ax("shocks")], Fx(8_500));
        assert_eq!(w.axes[&ax("stability")], Fx::from_int(55));
        let factions = [
            ("loyalty_nobles", 80),
            ("loyalty_church", 60),
            ("loyalty_people", 40),
        ];
        set(&mut w, &d, &factions);
        set(&mut w, &d, &[("legitimacy", 20), ("shocks", -4)]);
        // 50 + 0.6 * (65 - 50) + 0.4 * (20 - 50) - 4, no heresy at faith 65.
        assert_eq!(w.axes[&ax("stability")], Fx::from_int(43));
        // Heresy by the curve over faith: 12 at 30; at 37, 12 - 7 * (7 / 15) = 8.738.
        set(&mut w, &d, &[("faith", 30)]);
        assert_eq!(w.axes[&ax("stability")], Fx::from_int(31));
        set(&mut w, &d, &[("faith", 37)]);
        assert_eq!(w.axes[&ax("stability")], Fx(43_000 - 8_738));
        set(&mut w, &d, &[("shocks", -100)]);
        assert_eq!(w.axes[&ax("stability")], Fx(0)); // clamped
    }

    #[test]
    fn writes_to_stability_are_shocks_and_shocks_fade() {
        let (d, mut w) = setup(RULES);
        let still = [
            ("loyalty_nobles", 50),
            ("loyalty_church", 50),
            ("shocks", 0),
        ];
        set(&mut w, &d, &still); // nothing else moves
        let base = w.axes[&ax("stability")];
        add_axis(&mut w, &d, &ax("stability"), Fx::from_int(-10));
        assert_eq!(w.axes[&ax("shocks")], Fx::from_int(-10));
        assert_eq!(w.axes[&ax("stability")], base - Fx::from_int(10));
        let mut seen = vec![];
        for _ in 0..4 {
            tick(&d, &mut w);
            w.recompute_loyalty(&d);
            seen.push((w.axes[&ax("stability")] - base).0 / Fx::SCALE);
        }
        assert_eq!(seen, [-7, -4, -1, 0]); // shock_decay: the shocks axis steps 3 a year
        // Never a step of its own: stability has none.
        let a = d.axes.iter().find(|a| a.id.0 == "stability").unwrap();
        assert_eq!(step(&d, a), Fx(0));
    }

    #[test]
    fn loops_are_cycles_of_known_edges() {
        let with = |l: &str| {
            let rules = RULES.replacen(r#"("П1 Гнёт", ["e2", "e3"])"#, l, 1);
            crate::data::load(&rules)
        };
        assert!(with(r#"("x", ["e3", "e2"])"#).is_ok());
        assert!(with(r#"("x", ["e2", "e1"])"#).is_err()); // not a cycle
        assert!(with(r#"("x", ["e2", "nothing"])"#).is_err());
        assert!(with(r#"("x", [])"#).is_err());
    }

    #[test]
    fn stability_needs_two_plain_axes() {
        let with = |s: &str| {
            let block = r#"stability: (axis: "stability", shocks: "shocks")"#;
            crate::data::load(&RULES.replacen(block, s, 1))
        };
        assert!(with(r#"stability: (axis: "stability", shocks: "army")"#).is_ok());
        assert!(with(r#"stability: (axis: "stability", shocks: "stability")"#).is_err());
        assert!(with(r#"stability: (axis: "loyalty", shocks: "shocks")"#).is_err());
        assert!(with(r#"stability: (axis: "stability", shocks: "nothing")"#).is_err());
    }
}
