//! Neighbour AI: a stance from relation and border strength, a relation shift, maybe an event.

use crate::data::Data;
use crate::fx::Fx;
use crate::game::PendingEvent;
use crate::rng::Rng;
use crate::rules::{EventTarget, Target};
use crate::state::{Holder, NeighbourId, Stance, World};

/// One year of a neighbour, rules in `Data.neighbour_ai`. Returns the event it starts, if
/// any; the caller queues it. The event need not exist (`neighbour_war_declared` before
/// stage 4): unknown events are dropped when picked.
pub fn neighbour_tick(
    w: &mut World,
    data: &Data,
    rng: &mut Rng,
    id: NeighbourId,
) -> Option<PendingEvent> {
    let ai = &data.neighbour_ai;
    let theirs = Holder::Foreign(id.clone());
    // Own provinces on the common border; the weakest is where they press.
    let border = (w.provinces.values())
        .filter(|p| !matches!(p.holder, Holder::Foreign(_)))
        .filter(|p| {
            let theirs = |q: &_| w.provinces.get(q).is_some_and(|q| q.holder == theirs);
            p.neighbours.iter().any(theirs)
        })
        .min_by_key(|p| p.crown_power)
        .map(|p| (p.id.clone(), p.crown_power));
    let n = w.neighbours.get_mut(&id)?;
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
    n.relation = (n.relation + rules.relation).clamp(Fx(0) - limit, limit);

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
    let target = match kind {
        Some(EventTarget::None) => None,
        Some(EventTarget::RandomProvince(_)) => Some(Target::Province(border?.0)),
        _ => Some(Target::Neighbour(id)),
    };
    Some(PendingEvent {
        event_id: event.clone(),
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Preset, ProvinceId};

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
        let world = World::from_preset(&data, &preset);
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
        // Weakest own province on the Nordmark border: weir, crown power 20.
        assert_eq!(stance(-50, 20), Stance::Expand);
        assert_eq!(stance(-50, 19), Stance::Defend);
        assert_eq!(stance(0, 100), Stance::Wait);
        assert_eq!(stance(50, 0), Stance::Trade);
        // No common border: never Expand.
        let (data, mut w) = setup();
        w.provinces
            .retain(|_, p| p.holder != Holder::Foreign(nordmark()));
        let n = w.neighbours.get_mut(&nordmark()).unwrap();
        (n.relation, n.strength) = (Fx::from_int(-50), Fx::from_int(100));
        neighbour_tick(&mut w, &data, &mut Rng::from_seed(1), nordmark());
        assert_eq!(w.neighbours[&nordmark()].stance, Stance::Defend);
    }

    #[test]
    fn stance_moves_relation() {
        let relation = |r, s| run(r, s, 1).0.neighbours[&nordmark()].relation;
        assert_eq!(relation(-50, 100), Fx::from_int(-52)); // Expand
        assert_eq!(relation(-50, 0), Fx::from_int(-51)); // Defend
        assert_eq!(relation(50, 0), Fx::from_int(51)); // Trade
        assert_eq!(relation(0, 0), Fx::from_int(0)); // Wait
        assert_eq!(relation(-100, 100), Fx::from_int(-100)); // clamped
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
                "neighbour_raid" => Target::Province(ProvinceId("weir".into())),
                _ => Target::Neighbour(nordmark()),
            };
            assert_eq!(e.target, Some(target), "{}", e.event_id);
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
            Some(Target::Province(ProvinceId("weir".into())))
        );
        w.provinces
            .retain(|_, p| p.holder != Holder::Foreign(nordmark()));
        assert_eq!(
            neighbour_tick(&mut w, &data, &mut Rng::from_seed(1), nordmark()),
            None
        );
    }
}
