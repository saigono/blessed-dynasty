//! Stage 26: every neighbour a kingdom of its own, a `World` from its point of view, played
//! year by year by the same automaton as our dynasty after the founder (`sim::Dynasty`).
//! Only the map is shared: after each year every kingdom's holders follow ours. Nothing of
//! a kingdom reaches our world but `Neighbour.realm`, so our outcome stays as it was.

use crate::data::Data;
use crate::game::Game;
use crate::rng::Rng;
use crate::sim::Dynasty;
use crate::state::{Holder, NeighbourId, Preset, RealmView, World};
use std::collections::BTreeMap;

/// The foreign kingdoms of a game.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Realms {
    /// Our kingdom in their worlds.
    pub us: NeighbourId,
    pub list: BTreeMap<NeighbourId, Dynasty>,
}

/// The kingdoms of `preset` (`Preset.realms`), each with its own stream (`stream`) and the
/// simulation events in its pool; its house goes to our `Neighbour.realm`.
pub fn start(data: &Data, preset: &Preset, seed: u64, w: &mut World) -> Realms {
    let Some(start) = &preset.realms else {
        return Realms::default();
    };
    let mut data = data.clone();
    // Their pool is all of it at once: no reign of theirs is a player's.
    let sim = std::mem::take(&mut data.sim_events);
    data.events.extend(sim);
    let mut list = BTreeMap::new();
    for r in &start.kingdoms {
        let own = preset.realm(r);
        let g = Game {
            world: World::from_preset(&data, &own),
            rng: stream(seed, &r.id),
            data: data.clone(),
            decisions: Vec::new(),
            pending_event: None,
            queue: Vec::new(),
            ended: None,
            reported: false,
            realms: Realms::default(),
        };
        let d = Dynasty::new(g);
        if let Some(n) = w.neighbours.get_mut(&r.id) {
            n.realm = Some(view(&d, r.house.clone()));
        }
        list.insert(r.id.clone(), d);
    }
    Realms {
        us: start.us.clone(),
        list,
    }
}

/// The stream of kingdom `id` in the game of `seed`: apart from ours and every other one.
pub fn stream(seed: u64, id: &NeighbourId) -> Rng {
    let hash = (id.0.bytes()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    });
    Rng::from_seed(seed ^ hash)
}

impl Realms {
    /// The streams of a game started anew from `seed`, before its first year.
    pub fn reseed(&mut self, seed: u64) {
        for (id, d) in &mut self.list {
            d.g.rng = stream(seed, id);
            d.salt = d.g.rng.clone().next_u64();
        }
    }
}

/// A year of every kingdom up to our tick, then its holders as ours (`share`), then what we
/// see of it (`Neighbour.realm`). Called by `Game::wait` at the end of each year.
pub(crate) fn year(g: &mut Game) {
    let (w, realms) = (&mut g.world, &mut g.realms);
    for (id, d) in &mut realms.list {
        if d.fall.is_some() {
            continue;
        }
        d.until(w.tick);
        share(w, &realms.us, id, &mut d.g);
        if let Some(v) = w.neighbours.get_mut(id).and_then(|n| n.realm.as_mut()) {
            *v = view(d, std::mem::take(&mut v.house));
        }
    }
}

/// The holders of kingdom `id` (its world in `g`) as ours, the one map: our land is `us`, the
/// others' theirs, its own stays its own. A province it took from a kingdom in its own story
/// goes back to its crown, one taken from it here is lost (stage 27 lets kingdoms take
/// land); a vassal that broke away from it stays apart, unseen here.
fn share(ours: &World, us: &NeighbourId, id: &NeighbourId, g: &mut Game) {
    let kingdom = |n: &NeighbourId| n == us || ours.neighbours.contains_key(n);
    let mut moved = false;
    for (pid, p) in &ours.provinces {
        let Some(q) = g.world.provinces.get_mut(pid) else {
            continue;
        };
        let want = match &p.holder {
            Holder::Foreign(n) if n == id => match &q.holder {
                Holder::Foreign(x) if kingdom(x) => Holder::Crown,
                h => h.clone(),
            },
            Holder::Foreign(n) => Holder::Foreign(n.clone()),
            _ => Holder::Foreign(us.clone()),
        };
        if q.holder != want {
            q.holder = want;
            moved = true;
        }
    }
    if moved {
        g.world.recompute_crown_power(&g.data);
    }
}

fn view(d: &Dynasty, house: String) -> RealmView {
    let (w, data) = (&d.g.world, &d.g.data);
    let stability = data.stability.as_ref().map(|s| w.axes[&s.axis]);
    RealmView {
        house,
        ruler: w.ruler.name.clone(),
        law: data.heirs.law(w).map_or(String::new(), |l| l.flag.clone()),
        stability: stability.unwrap_or_default(),
        fallen: d.fall.is_some(),
    }
}
