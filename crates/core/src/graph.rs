//! The influence graph (docs/design/hidden-state.html): every axis steps toward its target,
//! its anchor plus the `Target` edges into it.

use crate::data::{AxisDef, Data};
use crate::fx::Fx;
use crate::state::World;

/// Yearly step of `a` toward its target: `AxisDef.step`, else `drift.step` for a faction
/// axis, else 0. Derived axes never step.
pub fn step(d: &Data, a: &AxisDef) -> Fx {
    if d.is_derived(&a.id) {
        return Fx(0);
    }
    let faction = d.factions.iter().any(|f| f.axis == a.id);
    a.step.unwrap_or(if faction { d.drift.step } else { Fx(0) })
}

/// The target of `a`: its anchor (`default` without one), clamped to its bounds.
pub fn target(a: &AxisDef) -> Fx {
    a.anchor.unwrap_or(a.default).clamp(a.min, a.max)
}

/// Per tick, from `Game::passive`: every axis moves its step (spread over the ticks of a
/// year) toward its target.
pub fn tick(d: &Data, w: &mut World) {
    let tpy = Fx::from_int(d.time_unit.ticks_per_year as i64);
    for a in &d.axes {
        let s = step(d, a) / tpy;
        if s == Fx(0) {
            continue;
        }
        let to = target(a);
        let v = w.axes.get_mut(&a.id).expect("the world has every axis");
        *v = match *v < to {
            true => (*v + s).min(to),
            false => (*v - s).max(to),
        };
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
        assert_eq!(target(def(&mut d, "army")), Fx::from_int(1000));
    }
}
