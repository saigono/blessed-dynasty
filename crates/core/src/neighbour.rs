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
    let kind = data.events.iter().find(|e| e.id == *event).map(|e| &e.target);
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
