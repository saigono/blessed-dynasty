//! The influence graph (docs/design/hidden-state.html): every axis steps toward its target,
//! its anchor plus the `Target` edges into it; `Flow` edges add to stocks every year.

use crate::data::{AxisDef, Data, curve};
use crate::fx::Fx;
use crate::rules::add_axis;
use crate::state::{AxisId, World};
use serde::Deserialize;

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
    let v = edges.fold(a.anchor.unwrap_or(a.default), |s, (_, c)| s + c) + w.axes[&s.shocks];
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

/// The edges of `kind` into `to` with their index and contribution now, in file order.
pub fn parts<'a>(
    d: &'a Data,
    w: &'a World,
    to: &'a AxisId,
    kind: InfluenceKind,
) -> impl Iterator<Item = (usize, Fx)> + 'a {
    let edges = d.influences.iter().enumerate();
    let into = edges.filter(move |(_, e)| e.kind == kind && e.to == *to);
    into.map(move |(i, e)| (i, e.now(i, w)))
}

/// The target of `a`: its anchor (`default` without one) plus the `Target` edges into it,
/// clamped to its bounds.
pub fn target(d: &Data, w: &World, a: &AxisDef) -> Fx {
    let anchor = a.anchor.unwrap_or(a.default);
    let edges = parts(d, w, &a.id, InfluenceKind::Target);
    edges.fold(anchor, |s, (_, c)| s + c).clamp(a.min, a.max)
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
        .map(|(i, e)| (&e.to, e.now(i, w) / tpy))
        .collect();
    for (to, v) in flows {
        add_axis(w, d, to, v);
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
        d.influences = vec![edge(r#"(from: "legitimacy", to: "army", k: 0.5, rest: 40)"#)];
        // Legitimacy 45 (preset): 50 + 0.5 * (45 - 40).
        assert_eq!(army_target(&d, &w), Fx(52_500));
        w.axes.insert(ax("legitimacy"), Fx::from_int(20));
        assert_eq!(army_target(&d, &w), Fx::from_int(40));
        // A second edge adds up; an edge of k < 0 pulls down.
        d.influences.push(edge(r#"(from: "prestige", to: "army", k: -1, rest: 0)"#));
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
        let (mut d, w) = setup(include_str!("../tests/main/rules.ron"));
        let old = crate::war::income_parts(&w, &d);
        assert_eq!(d.economy.flows, [(ax("income"), Fx::from_int(1))]);
        d.economy.flows.clear();
        assert_ne!(crate::war::income_parts(&w, &d), old);
        d.influences = vec![edge(r#"(from: "income", to: "treasury", k: 1, kind: Flow)"#)];
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
        let factions = [("loyalty_nobles", 80), ("loyalty_church", 60), ("loyalty_people", 40)];
        set(&mut w, &d, &factions);
        set(&mut w, &d, &[("legitimacy", 20), ("shocks", -4)]);
        // 50 + 0.6 * (65 - 50) + 0.4 * (20 - 50) - 4.
        assert_eq!(w.axes[&ax("stability")], Fx::from_int(43));
        set(&mut w, &d, &[("shocks", -100)]);
        assert_eq!(w.axes[&ax("stability")], Fx(0)); // clamped
    }

    #[test]
    fn writes_to_stability_are_shocks_and_shocks_fade() {
        let (d, mut w) = setup(RULES);
        let still = [("loyalty_nobles", 50), ("loyalty_church", 50), ("shocks", 0)];
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
