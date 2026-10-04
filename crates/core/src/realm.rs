//! Stage 26: every neighbour a kingdom of its own, a `World` from its point of view, played
//! year by year by the same automaton as our dynasty after the founder (`sim::Dynasty`).
//!
//! Stage 27: the kingdoms meet. Each world sees every other kingdom as a `Neighbour` whose
//! strength comes from that kingdom's own world (`RealmStrength`), so they raid, threaten and
//! go to war on each other by the events and the war chain we know; a war lives in the world
//! of the side that took it up. We are no neighbour in their worlds: our wars and dealings
//! with them are played in ours alone, as before. At the end of each of our years (`year`)
//! every kingdom plays its year, then the one map is drawn anew from what each world changed,
//! ours first, then the kingdoms by id: the first change of a province holds. A vassal that
//! broke away founds a kingdom, a fallen dynasty leaves its throne to a new house, a kingdom
//! with no crown land is no more; what the crown hears of it is `News`.

use crate::data::{Data, Dissolution, NewsKind, RealmStrength};
use crate::fx::Fx;
use crate::game::Game;
use crate::rng::Rng;
use crate::rules::add_axis;
use crate::sim::{Dynasty, FallReason};
use crate::state::{
    Axes, Capital, Founded, Heir, Holder, Map, Neighbour, NeighbourId, Preset, Province,
    ProvinceId, RealmView, Ruler, Sex, Stance, World,
};
use crate::time::Tick;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The foreign kingdoms of a game.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Realms {
    /// Our kingdom in their worlds.
    pub us: NeighbourId,
    pub list: BTreeMap<NeighbourId, Dynasty>,
    /// Stage 27: the seed of the game, for the streams of kingdoms founded later.
    pub seed: u64,
    /// The one map as the last year left it: who holds each province, `us` for ours.
    pub owners: BTreeMap<ProvinceId, NeighbourId>,
    /// Each kingdom's strength as the worlds last saw it.
    pub strength: BTreeMap<NeighbourId, Fx>,
    /// How a kingdom founded later starts; None: none is, a fallen one stands still. An
    /// `Arc` since stage 27b: `batch` shares the starting game between threads.
    pub founding: Option<Arc<Founding>>,
    /// What the crown heard of the world, oldest first.
    pub news: Vec<News>,
    /// Every dynasty of a kingdom that fell, when and how: for `cli` and calibration.
    pub falls: Vec<(NeighbourId, Tick, FallReason)>,
}

/// What a kingdom founded later starts from (`RealmsStart.founded`).
#[derive(Clone, Debug, PartialEq)]
pub struct Founding {
    /// The data of a kingdom: the events of the simulation in its pool.
    pub data: Data,
    pub profile: Founded,
    /// The preset's axes under the profile's.
    pub axes: Axes,
    pub crown_bonus: Fx,
}

/// News from afar (`NewsRules`): a short entry of the year's summary and of the chronicle.
#[derive(Clone, Debug, PartialEq)]
pub struct News {
    pub tick: Tick,
    pub title: String,
    pub text: String,
    pub importance: u32,
}

/// The kingdoms of `preset` (`Preset.realms`, with `Data.realm`), each with its own stream
/// (`stream`) and the simulation events in its pool, met (`meet`).
pub fn start(data: &Data, preset: &Preset, seed: u64, w: &mut World) -> Realms {
    let (Some(start), Some(rules)) = (&preset.realms, &data.realm) else {
        return Realms::default();
    };
    let mut kd = data.clone();
    // Their pool is all of it at once: no reign of theirs is a player's.
    let sim = std::mem::take(&mut kd.sim_events);
    kd.events.extend(sim);
    let mut list = BTreeMap::new();
    for r in &start.kingdoms {
        let own = World::from_preset(&kd, &preset.realm(r));
        let mut d = Dynasty::new(kingdom(own, stream(seed, &r.id), &kd));
        d.house = r.house.clone();
        d.name = w
            .neighbours
            .get(&r.id)
            .map_or(r.id.0.clone(), |n| n.name.clone());
        list.insert(r.id.clone(), d);
    }
    let founding = start.founded.as_ref().map(|f| {
        let mut axes = preset.axes.clone();
        axes.extend(f.axes.clone());
        Arc::new(Founding {
            data: kd,
            profile: f.clone(),
            axes,
            crown_bonus: preset.capital.crown_bonus,
        })
    });
    let us = start.us.clone();
    let owners = (w.provinces.values())
        .map(|p| (p.id.clone(), owner(&p.holder, &us)))
        .collect();
    let mut realms = Realms {
        us,
        list,
        seed,
        owners,
        strength: BTreeMap::new(),
        founding,
        news: Vec::new(),
        falls: Vec::new(),
    };
    meet(&mut realms, w, &rules.strength);
    for r in &start.kingdoms {
        let w = &mut realms.list.get_mut(&r.id).expect("listed").g.world;
        for (n, v) in &r.relations {
            if let Some(n) = w.neighbours.get_mut(n) {
                n.relation = *v;
            }
        }
    }
    realms
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
        self.seed = seed;
        for (id, d) in &mut self.list {
            d.g.rng = stream(seed, id);
            d.salt = d.g.rng.clone().next_u64();
        }
    }
}

/// A kingdom's own game.
fn kingdom(world: World, rng: Rng, data: &Data) -> Game {
    Game {
        world,
        rng,
        data: data.clone(),
        decisions: Vec::new(),
        pending_event: None,
        queue: Vec::new(),
        ended: None,
        reported: false,
        realms: Realms::default(),
    }
}

/// Who holds a province of world `me`: `me` for its crown and vassals.
fn owner(h: &Holder, me: &NeighbourId) -> NeighbourId {
    match h {
        Holder::Foreign(n) => n.clone(),
        _ => me.clone(),
    }
}

/// The name our world knows a state by.
fn name(w: &World, id: &NeighbourId) -> String {
    w.neighbours
        .get(id)
        .map_or(id.0.clone(), |n| n.name.clone())
}

/// A year of every kingdom up to our tick, then the world drawn anew (see the module). Called
/// by `Game::wait` at the end of each year.
pub(crate) fn year(g: &mut Game) {
    if g.realms.owners.is_empty() {
        return;
    }
    let Some(rules) = &g.data.realm else {
        return;
    };
    let (ours, realms, data) = (&mut g.world, &mut g.realms, &g.data);
    let (tick, tpy) = (ours.tick, ours.time_unit.ticks_per_year);
    for d in realms.list.values_mut() {
        if d.fall.is_none() {
            d.until(tick);
        }
    }
    let mut news = vec![];
    let mut tell = |k: &NewsKind, named: &[(&str, &str)]| {
        if k.importance >= rules.news.threshold {
            news.push(told(k, named, data, tick));
        }
    };

    // What each world changed of the map: the first change of a province holds; land comes
    // to us or leaves us in our world alone.
    let mut owners = realms.owners.clone();
    let mut by: BTreeMap<ProvinceId, NeighbourId> = BTreeMap::new();
    let worlds = std::iter::once((&realms.us, &*ours));
    for (me, w) in worlds.chain(realms.list.iter().map(|(id, d)| (id, &d.g.world))) {
        for p in w.provinces.values() {
            let (o, was) = (owner(&p.holder, me), realms.owners.get(&p.id));
            let ours_moved = *me != realms.us && (o == realms.us || was == Some(&realms.us));
            if was != Some(&o) && !ours_moved && !by.contains_key(&p.id) {
                by.insert(p.id.clone(), me.clone());
                owners.insert(p.id.clone(), o);
            }
        }
    }
    pay_back(realms, ours, &rules.strength);
    for (id, d) in &realms.list {
        let begun = |x: &&crate::war::War| x.started.0 + tpy > tick.0;
        let war = (d.g.world.war.as_ref()).filter(begun);
        if let Some(x) = war.filter(|x| realms.list.contains_key(&x.enemy)) {
            let (a, b) = (name(ours, id), name(ours, &x.enemy));
            tell(&rules.news.war, &[("realm", &a), ("enemy", &b)]);
        }
    }
    for p in by.keys() {
        let (from, to) = (&realms.owners[p], &owners[p]);
        if realms.list.contains_key(from) && realms.list.contains_key(to) {
            let (a, b) = (name(ours, to), name(ours, from));
            let named = [
                ("realm", a.as_str()),
                ("enemy", &b),
                ("province", &ours.provinces[p].name),
            ];
            tell(&rules.news.capture, &named);
        }
    }
    // The new states, each in the world it broke away from.
    let new: BTreeSet<NeighbourId> = (owners.values())
        .filter(|o| **o != realms.us && !realms.list.contains_key(*o))
        .cloned()
        .collect();
    for id in new {
        let Some(from) = (by.iter())
            .find(|(p, _)| owners[*p] == id)
            .map(|(_, me)| me.clone())
        else {
            continue;
        };
        let src = match realms.list.get(&from) {
            Some(d) => d.g.world.clone(),
            None => ours.clone(),
        };
        let called = name(&src, &id);
        if found(realms, ours, &owners, (&id, &called), &src, &from) && from != realms.us {
            let (lord, province) = (name(ours, &from), capital_name(realms, &id));
            let named = [
                ("realm", called.as_str()),
                ("enemy", &lord),
                ("province", &province),
            ];
            tell(&rules.news.breakaway, &named);
        }
    }
    // Fallen dynasties: a new house where the crown keeps land, else the kingdom is no more.
    let fallen: Vec<NeighbourId> = (realms.list.iter())
        .filter(|(_, d)| d.fall.as_ref().is_some_and(|f| *f != FallReason::Alive))
        .map(|(id, _)| id.clone())
        .filter(|_| realms.founding.is_some())
        .collect();
    for id in fallen {
        let d = &realms.list[&id];
        let fall = d.fall.clone().expect("fallen");
        realms.falls.push((id.clone(), d.g.world.tick, fall));
        let w = &d.g.world;
        let vassal = |p: &ProvinceId| matches!(w.provinces[p].holder, Holder::Vassal(_));
        let crown = (owners.iter()).any(|(p, o)| *o == id && !vassal(p));
        let realm = d.name.clone();
        if crown {
            let (old, house) = rehouse(realms, &owners, &id, data, &rules.usurper);
            ours.unions.remove(&id);
            tell(
                &rules.news.house,
                &[("realm", &realm), ("house", &house), ("old", &old)],
            );
            continue;
        }
        tell(&rules.news.fallen, &[("realm", &realm)]);
        let d = realms.list.remove(&id).expect("listed");
        let w = d.g.world;
        let capital = &owners[&w.capital.province];
        let conqueror = (rules.dissolution == Dissolution::Conqueror && *capital != id)
            .then(|| capital.clone());
        // To the conqueror, or every vassal a state on his land.
        let mut states = BTreeMap::new();
        for (p, o) in owners.iter_mut().filter(|(_, o)| **o == id) {
            *o = match (&conqueror, &w.provinces[p].holder) {
                (Some(c), _) => c.clone(),
                (None, Holder::Vassal(v)) => {
                    let taken = |n: &NeighbourId| {
                        *n == realms.us
                            || realms.list.contains_key(n)
                            || ours.neighbours.contains_key(n)
                    };
                    (states.entry(v.clone()).or_insert_with(|| {
                        let mut n = NeighbourId(v.0.clone());
                        while taken(&n) {
                            n.0.push('+');
                        }
                        n
                    }))
                    .clone()
                }
                (None, _) => unreachable!("no crown land is left"),
            };
        }
        for (v, n) in states {
            let called = w.vassals.get(&v).map_or(v.0.clone(), |v| v.name.clone());
            if found(realms, ours, &owners, (&n, &called), &w, &id) {
                let province = capital_name(realms, &n);
                let named = [
                    ("realm", called.as_str()),
                    ("enemy", &realm),
                    ("province", &province),
                ];
                tell(&rules.news.breakaway, &named);
            }
        }
    }
    draw(ours, &realms.us, &owners, data);
    for (id, d) in &mut realms.list {
        draw(&mut d.g.world, id, &owners, &d.g.data);
    }
    realms.owners = owners;
    meet(realms, ours, &rules.strength);
    realms.news.extend(news);
}

/// The capital's name of kingdom `id`.
fn capital_name(realms: &Realms, id: &NeighbourId) -> String {
    let w = &realms.list[id].g.world;
    w.provinces[&w.capital.province].name.clone()
}

/// News of kind `k`, `named` filled in, a variant by the tick apart from every stream.
fn told(k: &NewsKind, named: &[(&str, &str)], data: &Data, tick: Tick) -> News {
    let named: Vec<crate::text::Named> = named.iter().map(|(k, v)| (*k, *v, None)).collect();
    let fill = |s: &str| crate::text::fill(s, &data.names, &named);
    let text = crate::text::pick(&k.texts, NEWS ^ tick.0 as u64, named.len() as u64);
    News {
        tick,
        title: fill(&k.title),
        text: fill(text),
        importance: k.importance,
    }
}

/// The salt of the variants of `News`.
const NEWS: u64 = 0x4e45_5753 << 32;

/// What the worlds took from each kingdom's strength this year (a war it lost, tribute it
/// paid) or gave it comes off its army, or onto it, at the rate of `RealmStrength.army`.
fn pay_back(realms: &mut Realms, ours: &World, r: &RealmStrength) {
    if r.army <= Fx(0) {
        return;
    }
    let mut moved: BTreeMap<NeighbourId, Fx> = BTreeMap::new();
    let worlds = std::iter::once(ours).chain(realms.list.values().map(|d| &d.g.world));
    for w in worlds {
        for (id, n) in &w.neighbours {
            if let Some(was) = realms.strength.get(id) {
                let m = moved.entry(id.clone()).or_default();
                *m = *m + n.strength - *was;
            }
        }
    }
    for (id, v) in moved.into_iter().filter(|(_, v)| *v != Fx(0)) {
        if let Some(d) = realms.list.get_mut(&id) {
            let army = d.g.data.war.army.clone();
            add_axis(&mut d.g.world, &d.g.data, &army, v / r.army);
        }
    }
}

/// Kingdom `id` called `name` founded on its land of `owners`, taken from world `src` of
/// `from` (`Founding`): a house of its name, a ruler and heirs of the profile, the grudge of
/// `Effect::Secede` against `from`. Our world gets an entry for it with a colour of its own.
/// False without a profile.
fn found(
    realms: &mut Realms,
    ours: &mut World,
    owners: &BTreeMap<ProvinceId, NeighbourId>,
    (id, name): (&NeighbourId, &str),
    src: &World,
    from: &NeighbourId,
) -> bool {
    let Some(f) = realms.founding.clone() else {
        return false;
    };
    let mine = |p: &Province| owners[&p.id] == *id;
    let capital = (src.provinces.values().filter(|p| mine(p)))
        .max_by_key(|p| (p.income, Reverse(&p.id)))
        .map(|p| p.id.clone())
        .expect("a new state has land");
    let provinces = (src.provinces.values())
        .map(|p| Province {
            holder: match mine(p) {
                true => Holder::Crown,
                false => Holder::Foreign(owners[&p.id].clone()),
            },
            ..p.clone()
        })
        .collect();
    let mut rng = stream(realms.seed ^ ours.tick.0 as u64, id);
    let (ruler, heirs) = people(&f.profile, &f.data, &mut rng);
    let preset = Preset {
        start_year: ours.year(),
        axes: f.axes.clone(),
        map: Map {
            provinces,
            polygons: BTreeMap::new(),
        },
        capital: Capital {
            province: capital,
            crown_bonus: f.crown_bonus,
        },
        vassals: vec![],
        ruler,
        heirs,
        neighbours: vec![],
        flags: f.profile.flags.clone(),
        intro: String::new(),
        realms: None,
    };
    let mut w = World::from_preset(&f.data, &preset);
    (w.start_year, w.tick, w.ruler.reign_start) = (ours.start_year, ours.tick, ours.tick);
    if let Some(lord) = ours
        .neighbours
        .get(from)
        .filter(|_| realms.list.contains_key(from))
    {
        let grudge = f.data.sim.secession_relation;
        w.neighbours.insert(from.clone(), stranger(lord, grudge));
    }
    let mut d = Dynasty::new(kingdom(w, rng, &f.data));
    (d.house, d.name) = (name.to_string(), name.to_string());
    realms.list.insert(id.clone(), d);
    let ordinal = (ours.neighbours.values().map(|n| n.ordinal + 1).max()).unwrap_or(0);
    // Our own vassal that broke away has his entry already (`Effect::Secede`).
    ours.neighbours
        .entry(id.clone())
        .or_insert_with(|| Neighbour {
            id: id.clone(),
            name: name.to_string(),
            relation: Fx(0),
            strength: Fx(0),
            stance: Stance::Wait,
            per_province: Fx(0),
            ordinal,
            marks: Default::default(),
            realm: None,
        });
    true
}

/// A ruler and heirs by profile `f`, the names its own or of `names.ron` by sex.
fn people(f: &Founded, data: &Data, rng: &mut Rng) -> (Ruler, Vec<Heir>) {
    let n = &data.names;
    let mut named = |name: &str, sex: Sex, ruler: bool| {
        let pool = match sex {
            Sex::Female => &n.daughters,
            Sex::Male if ruler => &n.rulers,
            Sex::Male => &n.heirs,
        };
        match name.is_empty() && !pool.is_empty() {
            true => pool[rng.range(0, pool.len() as i64) as usize].clone(),
            false => name.to_string(),
        }
    };
    let r = &f.ruler;
    let ruler = Ruler {
        name: named(&r.name, r.sex, true),
        ..r.clone()
    };
    let heirs = (f.heirs.iter())
        .map(|h| Heir {
            name: named(&h.name, h.sex, false),
            ..h.clone()
        })
        .collect();
    (ruler, heirs)
}

/// A new house on the fallen throne of kingdom `id`, as a usurper on ours: its strongest
/// vassal (his land the crown's), else a house of `names.houses` no kingdom has; the ruler and
/// heirs of the profile, the axes of `usurper`; the court moves to the nearest crown land if
/// the capital is lost; the last house's war and plans end with it. Returns (the old house,
/// the new one).
fn rehouse(
    realms: &mut Realms,
    owners: &BTreeMap<ProvinceId, NeighbourId>,
    id: &NeighbourId,
    data: &Data,
    usurper: &[(crate::state::AxisId, Fx)],
) -> (String, String) {
    let f = realms.founding.clone().expect("checked by the caller");
    let houses: BTreeSet<String> = realms.list.values().map(|d| d.house.clone()).collect();
    let mut d = realms.list.remove(id).expect("listed");
    let (g, old, called) = (&mut d.g, d.house, d.name);
    let w = &mut g.world;
    let ours = |p: &Province| owners[&p.id] == *id;
    let landed = |v: &crate::state::VassalId| {
        (w.provinces.values()).any(|p| ours(p) && p.holder == Holder::Vassal(v.clone()))
    };
    let strongest = (w.vassals.values().filter(|v| landed(&v.id)))
        .max_by_key(|v| (v.strength, Reverse(v.id.clone())))
        .map(|v| v.id.clone());
    let house = match strongest {
        Some(v) => {
            let v = w.vassals.remove(&v).expect("found above");
            let his = Holder::Vassal(v.id);
            for p in w.provinces.values_mut().filter(|p| p.holder == his) {
                p.holder = Holder::Crown;
            }
            v.name
        }
        None => {
            let free: Vec<&String> = (data.names.houses.iter())
                .filter(|h| !houses.contains(*h))
                .collect();
            match free.is_empty() {
                true => old.clone(),
                false => free[g.rng.range(0, free.len() as i64) as usize].clone(),
            }
        }
    };
    let crown = |p: &Province| ours(p) && !matches!(p.holder, Holder::Vassal(_));
    if !w.provinces.get(&w.capital.province).is_some_and(crown) {
        let near = (w.provinces.values().filter(|p| crown(p)))
            .min_by_key(|p| (p.distance_to_capital, &p.id))
            .map(|p| p.id.clone())
            .expect("the crown keeps land");
        w.capital.province = near;
        let hops = w.hops(&w.capital.province);
        for p in w.provinces.values_mut() {
            p.distance_to_capital = hops.get(&p.id).copied().unwrap_or(u32::MAX);
        }
    }
    let (mut ruler, heirs) = people(&f.profile, &g.data, &mut g.rng);
    ruler.reign_start = w.tick;
    w.found_house(ruler, heirs);
    let d_ = &g.data;
    for flag in [
        &d_.sim.usurped_flag,
        &d_.abdication.contested_flag,
        &d_.sim.regency_flag,
    ] {
        w.flags.remove(flag);
    }
    for flag in &d_.sim.reign_flags {
        w.flags.remove(flag);
    }
    let married = &d_.heirs.married_flag;
    match f.profile.flags.contains(married) {
        true => w.flags.insert(married.clone()),
        false => w.flags.remove(married),
    };
    for (a, v) in usurper {
        let by = *v - w.axes[a];
        add_axis(w, d_, a, by);
    }
    w.war = None;
    w.recompute_loyalty(d_);
    w.recompute_crown_power(d_);
    (g.queue, g.pending_event, g.ended, g.reported) = (Vec::new(), None, None, false);
    let mut d = Dynasty::under(d.c, d.g);
    (d.house, d.name) = (house.clone(), called);
    realms.list.insert(id.clone(), d);
    (old, house)
}

/// Draws the map of `owners` into world `w` of `me`: its own land keeps its crown and
/// vassals, land it gained is its crown's, the rest is foreign.
fn draw(w: &mut World, me: &NeighbourId, owners: &BTreeMap<ProvinceId, NeighbourId>, data: &Data) {
    let mut moved = false;
    for p in w.provinces.values_mut() {
        let o = &owners[&p.id];
        let want = match &p.holder {
            Holder::Foreign(_) if o == me => Holder::Crown,
            _ if o == me => continue,
            _ => Holder::Foreign(o.clone()),
        };
        if p.holder != want {
            p.holder = want;
            moved = true;
        }
    }
    if moved {
        w.recompute_crown_power(data);
    }
}

/// Every world sees every kingdom but itself, theirs not us: an entry made where missing
/// (neutral, a copy of ours), its strength from the kingdom's own world, no recovery of its
/// own (`Neighbour.per_province` 0); ours shows its house and ruler (`RealmView`).
fn meet(realms: &mut Realms, ours: &mut World, r: &RealmStrength) {
    let mut held: BTreeMap<&NeighbourId, i64> = BTreeMap::new();
    for o in realms.owners.values() {
        *held.entry(o).or_default() += 1;
    }
    let strength: BTreeMap<NeighbourId, Fx> = (realms.list.iter())
        .map(|(id, d)| {
            let (w, data) = (&d.g.world, &d.g.data);
            let axes =
                (r.axes.iter()).fold(Fx(0), |s, (a, c)| s + crate::data::curve(c, w.axes[a]));
            let land = Fx::from_int(held.get(id).copied().unwrap_or(0)) * r.province;
            (
                id.clone(),
                (w.axes[&data.war.army] * r.army + land + axes).max(Fx(0)),
            )
        })
        .collect();
    for (id, d) in &realms.list {
        if let Some(n) = ours.neighbours.get_mut(id) {
            (n.strength, n.per_province) = (strength[id], Fx(0));
            n.realm = Some(view(d));
        }
    }
    let known: Vec<Neighbour> = (realms.list.keys())
        .filter_map(|id| ours.neighbours.get(id))
        .map(|n| stranger(n, Fx(0)))
        .collect();
    for (id, d) in &mut realms.list {
        let w = &mut d.g.world;
        for n in known.iter().filter(|n| n.id != *id) {
            let e = w
                .neighbours
                .entry(n.id.clone())
                .or_insert_with(|| n.clone());
            (e.name, e.strength, e.ordinal) = (n.name.clone(), n.strength, n.ordinal);
            e.per_province = Fx(0);
        }
    }
    realms.strength = strength;
}

/// `n` as a new acquaintance sees it: of this relation, waiting, unmarked, no kingdom shown.
fn stranger(n: &Neighbour, relation: Fx) -> Neighbour {
    Neighbour {
        relation,
        stance: Stance::Wait,
        per_province: Fx(0),
        marks: Default::default(),
        realm: None,
        ..n.clone()
    }
}

fn view(d: &Dynasty) -> RealmView {
    let (w, data) = (&d.g.world, &d.g.data);
    let stability = data.stability.as_ref().map(|s| w.axes[&s.axis]);
    RealmView {
        house: d.house.clone(),
        ruler: w.ruler.name.clone(),
        law: data.heirs.law(w).map_or(String::new(), |l| l.flag.clone()),
        stability: stability.unwrap_or_default(),
        fallen: d.fall.is_some(),
    }
}
