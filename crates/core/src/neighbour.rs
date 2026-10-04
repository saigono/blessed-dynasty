//! Neighbour AI: a stance from relation and border strength, a relation shift, maybe an event.

use crate::data::Data;
use crate::fx::Fx;
use crate::game::PendingEvent;
use crate::rng::Rng;
use crate::rules::{EventTarget, Target};
use crate::state::{Holder, NeighbourId, Stance, World};

/// One year of a neighbour, rules in `Data.neighbour_ai`. Returns the event it starts, if
/// any; the caller offers it to the event pick, which drops unknown events.
pub fn neighbour_tick(
    w: &mut World,
    data: &Data,
    rng: &mut Rng,
    id: NeighbourId,
) -> Option<PendingEvent> {
    let ai = &data.neighbour_ai;
    let border = (w.weakest_border(&id)).map(|p| (p.id.clone(), p.crown_power));
    let holder = Holder::Foreign(id.clone());
    let held = w.provinces.values().filter(|p| p.holder == holder).count();
    let n = w.neighbours.get_mut(&id)?;
    if n.per_province > Fx(0) {
        let target = n.per_province * Fx::from_int(held as i64);
        n.strength = match n.strength < target {
            true => (n.strength + ai.recover).min(target),
            false => (n.strength - ai.recover).max(target),
        };
    }
    n.stance = if n.relation < ai.hostile_below {
        match &border {
            Some((_, power)) if n.strength >= *power + ai.expand_margin => Stance::Expand,
            _ => Stance::Defend,
        }
    } else if n.relation > ai.friendly_above {
        Stance::Trade
    } else {
        Stance::Wait
    };
    let rules = ai.stance(&n.stance);
    let limit = Fx::from_int(100);
    let r = n.relation + rules.relation;
    let r = match r < Fx(0) {
        true => (r + ai.drift).min(Fx(0)),
        false => (r - ai.drift).max(Fx(0)),
    };
    n.relation = r.clamp(Fx(0) - limit, limit);
    // Stage 28: a hostile state beyond other lands has no border to threaten.
    if n.stance == Stance::Defend && border.is_none() {
        return None;
    }

    let mut roll = rng.range(0, 100) as u32;
    let (event, _) = rules.events.iter().find(|(_, chance)| {
        let hit = roll < *chance;
        roll = roll.saturating_sub(*chance);
        hit
    })?;
    let kind = data
        .events
        .iter()
        .find(|e| e.id == *event)
        .map(|e| &e.target);
    let (target, neighbour) = match kind {
        Some(EventTarget::None) => (None, None),
        Some(EventTarget::RandomProvince(_)) => (Some(Target::Province(border?.0)), Some(id)),
        _ => (Some(Target::Neighbour(id)), None),
    };
    Some(PendingEvent {
        event_id: event.clone(),
        target,
        neighbour,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Holder, Preset, ProvinceId};

    fn setup() -> (Data, World) {
        let mut data = crate::data::load(include_str!("../../../data/rules.ron")).unwrap();
        data.add_events(include_str!("../../../data/events/neighbours.ron"))
            .unwrap();
        let preset = Preset::load_with_map(
            include_str!("../../../data/presets/default.ron"),
            include_str!("../../../data/maps/default.ron"),
            &data,
        )
        .unwrap();
        let mut world = World::from_preset(&data, &preset);
        // The preset makes Nordmark hostile; tests start it neutral (Wait) unless they say so.
        world.neighbours.get_mut(&nordmark()).unwrap().relation = Fx(0);
        // Strength stays as a test sets it unless the test turns recovery on.
        data.neighbour_ai.recover = Fx(0);
        (data, world)
    }

    fn nordmark() -> NeighbourId {
        NeighbourId("nordmark".into())
    }

    /// Sets Nordmark's relation and strength, runs `years` ticks, returns the events.
    fn run(relation: i64, strength: i64, years: u32) -> (World, Vec<PendingEvent>) {
        let (data, mut w) = setup();
        let n = w.neighbours.get_mut(&nordmark()).unwrap();
        (n.relation, n.strength) = (Fx::from_int(relation), Fx::from_int(strength));
        let mut rng = Rng::from_seed(7);
        let events = (0..years)
            .filter_map(|_| neighbour_tick(&mut w, &data, &mut rng, nordmark()))
            .collect();
        (w, events)
    }

    fn stance(relation: i64, strength: i64) -> Stance {
        run(relation, strength, 1).0.neighbours[&nordmark()]
            .stance
            .clone()
    }

    #[test]
    fn stance_follows_relation_and_border_strength() {
        // Weakest own province on the Nordmark border: arden, crown power 22.5.
        assert_eq!(stance(-50, 23), Stance::Expand);
        assert_eq!(stance(-50, 22), Stance::Defend);
        assert_eq!(stance(0, 100), Stance::Wait);
        assert_eq!(stance(50, 0), Stance::Trade);
        // No common border: never Expand.
        let (data, mut w) = setup();
        w.provinces
            .retain(|_, p| p.holder != Holder::Foreign(nordmark()));
        let n = w.neighbours.get_mut(&nordmark()).unwrap();
        (n.relation, n.strength) = (Fx::from_int(-50), Fx::from_int(100));
        // Stage 28: and no threats from beyond other lands, where Defend has its ultimatum.
        let mut rng = Rng::from_seed(1);
        let events = (0..50).filter_map(|_| neighbour_tick(&mut w, &data, &mut rng, nordmark()));
        assert_eq!(events.count(), 0);
        assert_eq!(w.neighbours[&nordmark()].stance, Stance::Defend);
    }

    #[test]
    fn strength_recovers_toward_its_lands() {
        let (mut data, w) = setup();
        // From the preset: Nordmark, strength 60 over 3 provinces.
        assert_eq!(w.neighbours[&nordmark()].per_province, Fx::from_int(20));
        data.neighbour_ai.recover = Fx::from_int(5);
        let after = |data: &Data, strength: i64, setup_w: &dyn Fn(&mut World)| {
            let (_, mut w) = setup();
            w.neighbours.get_mut(&nordmark()).unwrap().strength = Fx::from_int(strength);
            setup_w(&mut w);
            let e = neighbour_tick(&mut w, data, &mut Rng::from_seed(7), nordmark());
            (w, e)
        };
        let strength =
            |s, f: &dyn Fn(&mut World)| after(&data, s, f).0.neighbours[&nordmark()].strength;
        let same = |_: &mut World| {};
        let i = Fx::from_int;
        assert_eq!(strength(50, &same), i(55));
        assert_eq!(strength(58, &same), i(60)); // not above the target
        assert_eq!(strength(70, &same), i(65));
        assert_eq!(strength(62, &same), i(60)); // not below it
        // Land moves the target: 2 provinces give 40, 4 give 80.
        let lost = |w: &mut World| {
            w.provinces
                .get_mut(&ProvinceId("frostad".into()))
                .unwrap()
                .holder = Holder::Crown;
        };
        let gained = |w: &mut World| {
            w.provinces
                .get_mut(&ProvinceId("holm".into()))
                .unwrap()
                .holder = Holder::Foreign(nordmark());
        };
        assert_eq!(strength(60, &lost), i(55));
        assert_eq!(strength(42, &lost), i(40));
        assert_eq!(strength(60, &gained), i(65));
        // recover 0: the same world and event as a state that never recovers.
        data.neighbour_ai.recover = Fx(0);
        let never = |w: &mut World| w.neighbours.get_mut(&nordmark()).unwrap().per_province = Fx(0);
        for s in [20, 60, 100] {
            let (mut w, e) = after(&data, s, &same);
            w.neighbours.get_mut(&nordmark()).unwrap().per_province = Fx(0);
            assert_eq!((w, e), after(&data, s, &never), "{s}");
        }
    }

    #[test]
    fn stance_moves_relation() {
        let relation = |r, s| run(r, s, 1).0.neighbours[&nordmark()].relation;
        // The stance's change, then the drift toward 0.
        let d = setup().0.neighbour_ai.drift;
        assert!(d > Fx(0) && d < Fx::from_int(10), "{d}");
        let i = Fx::from_int;
        assert_eq!(relation(-50, 100), i(-52) + d); // Expand
        assert_eq!(relation(-50, 0), i(-51) + d); // Defend
        assert_eq!(relation(50, 0), i(51) - d); // Trade
        assert_eq!(relation(-10, 0), i(-10) + d); // Wait
        assert_eq!(relation(10, 0), i(10) - d);
        assert_eq!(relation(0, 0), i(0)); // the drift stops at 0
        assert_eq!(relation(-100, 100), (i(-102) + d).max(i(-100))); // clamped
    }

    #[test]
    fn hostile_neighbour_acts_friendly_one_does_not_raid() {
        let (_, hostile) = run(-50, 100, 20);
        assert!(!hostile.is_empty());
        let ids = |es: &[PendingEvent]| es.iter().map(|e| e.event_id.clone()).collect::<Vec<_>>();
        assert!(ids(&hostile).contains(&"neighbour_raid".to_string()));
        for e in &hostile {
            let target = match e.event_id.as_str() {
                // The weakest border province, the rest at the neighbour itself.
                "neighbour_raid" => Target::Province(ProvinceId("arden".into())),
                _ => Target::Neighbour(nordmark()),
            };
            assert_eq!(e.target, Some(target), "{}", e.event_id);
            // A province event also names its raider.
            let raider = (e.event_id == "neighbour_raid").then(nordmark);
            assert_eq!(e.neighbour, raider, "{}", e.event_id);
        }
        let (_, friendly) = run(80, 100, 20);
        assert!(!ids(&friendly).contains(&"neighbour_raid".to_string()));
        assert!(ids(&friendly).contains(&"neighbour_trade".to_string()));
    }

    #[test]
    fn event_target_follows_the_event_definition() {
        let (mut data, mut w) = setup();
        data.neighbour_ai.wait.events = vec![("neighbour_trade".into(), 100)];
        let mut tick =
            |data: &Data| neighbour_tick(&mut w, data, &mut Rng::from_seed(1), nordmark());
        assert_eq!(
            tick(&data).unwrap().target,
            Some(Target::Neighbour(nordmark()))
        );
        data.events[1].target = EventTarget::None; // neighbour_trade
        assert_eq!(tick(&data).unwrap().target, None);
        data.events.clear(); // unknown, e.g. neighbour_war_declared before stage 4
        assert_eq!(
            tick(&data).unwrap().target,
            Some(Target::Neighbour(nordmark()))
        );
        data.neighbour_ai.wait.events.clear();
        assert_eq!(tick(&data), None);
    }

    #[test]
    fn raid_needs_a_border() {
        let (mut data, mut w) = setup();
        data.neighbour_ai.wait.events = vec![("neighbour_raid".into(), 100)];
        let raid = neighbour_tick(&mut w, &data, &mut Rng::from_seed(1), nordmark());
        assert_eq!(
            raid.unwrap().target,
            Some(Target::Province(ProvinceId("arden".into())))
        );
        w.provinces
            .retain(|_, p| p.holder != Holder::Foreign(nordmark()));
        assert_eq!(
            neighbour_tick(&mut w, &data, &mut Rng::from_seed(1), nordmark()),
            None
        );
    }
}
