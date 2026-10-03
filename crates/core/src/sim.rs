//! The dynasty after the founder: the same `Game` year by year, choices by `AutoChooser`,
//! until the dynasty falls or `sim.max_years` pass. The result is a `Chronicle`.

use crate::data::{Data, Epithet, LawDef, SuccessionRule, TraitRule};
use crate::fx::Fx;
use crate::game::{ActionId, Game, PendingEvent, ReignEnd, Step};
use crate::rng::Rng;
use crate::rules::add_axis;
use crate::rules::{Choice, Effect, Event, HeirOp, NewHolder, Predicate, ProvinceField, Target};
use crate::state::{
    Axes, AxisId, CauseTag, HeirStatus, Holder, Kin, MarkKey, NeighbourId, ProvinceId, Ruler, Sex,
    Vassal, VassalId, World,
};
use crate::testament;
use crate::text;
use crate::time::Tick;
use crate::war::WarStage;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chronicle {
    pub entries: Vec<ChronicleEntry>,
    pub fall: FallReason,
    /// From tick 0 (the founder's accession) to the fall, at most `sim.max_years`.
    pub years: u32,
    /// The founder first.
    pub rulers: Vec<RulerRecord>,
    /// The family tree at the end, `World.kin`.
    #[serde(default)]
    pub kin: Vec<Kin>,
    /// The axes at the end, e.g. the army a dynasty fell or lived on with.
    #[serde(default)]
    pub axes: Axes,
    /// Years the army deserted for want of pay after the founder (`World.deserted`).
    #[serde(default)]
    pub deserted: u32,
    /// The graph at the end of every simulated year; only with `Data.influences`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeYear>,
    /// How the dynasty ended, told (`sim.texts.fall_told`); empty without a text.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub epilogue: String,
    /// The chronicles of the foreign kingdoms (`realm.rs`), hidden from the player.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub realms: BTreeMap<NeighbourId, Chronicle>,
}

/// The axes and the lagged sources of the influence graph after a year's tick.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NodeYear {
    /// Years from tick 0.
    pub year: u32,
    /// In `Data.axes` order.
    pub axes: Vec<Fx>,
    /// `World.lagged`.
    pub lagged: Vec<Fx>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChronicleEntry {
    pub tick: Tick,
    /// The event behind the entry; None for a new ruler and a province lost or gained.
    pub event: Option<String>,
    pub title: String,
    pub text: String,
    /// The hint (`data/hints.ron`) of the main cause as a sentence, if that cause weighs at
    /// least `sim.hint_weight`.
    pub hint: Option<String>,
    pub importance: u32,
    /// The player's decisions behind the entry, one per decision, heaviest first.
    pub causes: Vec<CauseTag>,
    pub snapshot: World,
    /// What led to the event through the influence graph (`chain`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<Chain>,
    /// Told in the entry before it (stage 26c, `fuse`): kept for the score and the counts,
    /// shown no more.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub joined: bool,
}

/// A chain of the influence graph behind an event, told in the chronicle.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chain {
    /// From the start of the chain to the node of the event's condition.
    pub nodes: Vec<AxisId>,
    /// The law at the start of the chain, if any.
    pub law: Option<String>,
    /// The player's decision at the start (an index into `Game.decisions`), if any: the one
    /// that brought in `law`, else the heaviest mark on the nodes.
    pub decision: Option<usize>,
    /// The sentence (`data/hints.ron`, keys `chain:`).
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum FallReason {
    NoHeir,
    CapitalLost,
    /// `sim.usurped_flag` is set.
    Usurped,
    /// No province is left to the crown: the realm fell apart into appanages.
    NoCrownLand,
    /// Not a fall: the dynasty reached `sim.max_years`.
    Alive,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RulerRecord {
    pub name: String,
    pub traits: BTreeSet<String>,
    pub start: Tick,
    pub end: Tick,
    /// The cause of the reign end; None when the dynasty fell under him or lives on.
    pub cause: Option<String>,
    /// Crowned as the designated heir over the rightful one (`World.designated`).
    #[serde(default)]
    pub designated: bool,
    /// What he is called after, by the deeds of the reign (`sim.texts.epithets`): «Строитель».
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub epithet: String,
    /// His life in a paragraph (`sim.texts.life`): how he came to the throne, the epithet,
    /// the main entries of the reign, how it ended.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub biography: String,
}

impl RulerRecord {
    /// «Ульрих Строитель», or the name alone.
    pub fn full_name(&self) -> String {
        match self.epithet.is_empty() {
            true => self.name.clone(),
            false => format!("{} {}", self.name, self.epithet),
        }
    }
}

/// What a reign leaves for its ruler's life (`finish`): the sex, how he came to the throne,
/// the deeds counted for the epithet (`data::Epithet`), the first entry of the reign.
#[derive(Clone, Debug, PartialEq)]
struct Reign {
    sex: Sex,
    accession: String,
    deeds: BTreeMap<String, u32>,
    from: usize,
}

impl Reign {
    fn count(&mut self, deed: &str) {
        *self.deeds.entry(deed.to_string()).or_default() += 1;
    }
}

/// Plays the dynasty from the end of the founder's reign. Simulation events
/// (`Data.sim_events`) join the pool; nothing the automaton does is a player decision, so
/// it marks nothing. An unfinished war starts over from its declaration under the new ruler,
/// since the deferred queue ends with the reign. The variants of the texts come from an rng
/// of their own, seeded by a copy of `rng` (`text::pick`): the main stream stays as it was.
/// The foreign kingdoms (`ReignEnd.realms`) go on beside it and end with it, in
/// `Chronicle.realms`.
pub fn run(reign_end: ReignEnd, data: &Data, rng: Rng) -> Chronicle {
    let mut data = data.clone();
    data.events.extend(data.sim_events.clone());
    let salt = rng.clone().next_u64();
    let founder = record(&reign_end.world.ruler);
    let mut c = chronicle(RulerRecord {
        end: reign_end.tick,
        cause: Some(reign_end.cause),
        ..founder
    });
    let mut world = reign_end.world;
    let (reign, told) = founder_reign(&world, &data, salt);
    finish(&mut c, reign, told, &world, &data, salt);
    let deserted = world.deserted;
    died(&mut world, &data, c.rulers[0].cause.as_deref());
    // The founder's testament binds from his death, with the legend he leaves.
    let legend = testament::legend(&data, &world);
    if let Some(t) = &mut world.testament {
        (t.legend, t.since) = (legend, Some(reign_end.tick));
    }
    let mut g = Game {
        world,
        rng,
        data,
        decisions: Vec::new(),
        pending_event: None,
        queue: Vec::new(),
        ended: None,
        reported: false,
        realms: reign_end.realms,
    };
    if let Some(war) = &mut g.world.war {
        war.stage = WarStage::Declared;
        let p = PendingEvent {
            event_id: g.data.war.start_event.clone(),
            target: Some(Target::Neighbour(war.enemy.clone())),
            neighbour: None,
        };
        g.queue.push((g.world.tick, p));
    }
    if let Some(told) = testament::read(&g.data, &g.world, salt, TESTAMENT) {
        c.entries.push(entry(&g, told, g.data.sim.notable, vec![]));
    }
    let mut d = Dynasty {
        g,
        c,
        reign: None,
        last_heir: None,
        told: None,
        salt,
        deserted,
        fall: None,
    };
    while d.fall.is_none() {
        d.tick();
    }
    d.close()
}

/// A chronicle with its first ruler.
fn chronicle(first: RulerRecord) -> Chronicle {
    Chronicle {
        entries: Vec::new(),
        fall: FallReason::Alive,
        years: 0,
        rulers: vec![first],
        kin: Vec::new(),
        axes: Axes::new(),
        deserted: 0,
        nodes: Vec::new(),
        epilogue: String::new(),
        realms: BTreeMap::new(),
    }
}

/// A dynasty the automaton plays tick by tick: ours after the founder (`run`), every foreign
/// kingdom from the start (`realm.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct Dynasty {
    pub g: Game,
    pub c: Chronicle,
    /// The reign going on and its automaton; None: the throne waits for the next heir.
    reign: Option<(Reign, AutoChooser)>,
    /// The death of a young first heir who was the last one, with its place in the entries:
    /// told only if no heir comes after and the dynasty ends for want of one.
    last_heir: Option<(usize, ChronicleEntry)>,
    /// The last entry, if an event's not fused yet (`fuse`), and the event's target.
    told: Option<(usize, Option<Target>)>,
    /// Seeds the variants of the texts (`text::pick`).
    pub(crate) salt: u64,
    /// `World.deserted` when the dynasty began, for `Chronicle.deserted`.
    deserted: u32,
    /// How it ended; None while it goes on.
    pub fall: Option<FallReason>,
}

impl Dynasty {
    /// A kingdom whose ruler reigns from the start of `g`.
    pub fn new(g: Game) -> Dynasty {
        let r = &g.world.ruler;
        let reign = Reign {
            sex: r.sex,
            accession: String::new(),
            deeds: BTreeMap::new(),
            from: 0,
        };
        Dynasty {
            c: chronicle(record(r)),
            reign: Some((reign, AutoChooser::for_ruler(&g.data, r))),
            last_heir: None,
            told: None,
            salt: g.rng.clone().next_u64(),
            deserted: g.world.deserted,
            fall: None,
            g,
        }
    }

    /// Plays up to `tick`, the next heir crowned if the reign ended on it.
    pub fn until(&mut self, tick: Tick) {
        while self.fall.is_none() && self.g.world.tick < tick {
            self.tick();
        }
        if self.fall.is_none() && self.reign.is_none() {
            self.crown();
        }
    }

    /// Crowns the next heir; false and the fall `NoHeir` without one.
    fn crown(&mut self) -> bool {
        let Some(reign) = crown(&mut self.g, &mut self.c, self.salt) else {
            if let Some((i, e)) = self.last_heir.take() {
                self.c.entries.insert(i, e);
            }
            self.fall = Some(FallReason::NoHeir);
            return false;
        };
        let base = AutoChooser::for_ruler(&self.g.data, &self.g.world.ruler);
        self.reign = Some((reign, base));
        true
    }

    /// The dynasty ends under a living ruler.
    fn end(&mut self, reign: Reign, fall: FallReason) {
        let (g, c) = (&self.g, &mut self.c);
        c.rulers.last_mut().expect("a ruler reigned").end = g.world.tick;
        c.fall = fall.clone();
        let told = reign_deeds(c, reign.from, g, self.salt);
        finish(c, reign, told, &g.world, &g.data, self.salt);
        self.fall = Some(fall);
    }

    /// One tick: the next heir crowned first if the throne is empty, the automaton's action at
    /// the start of a year, its choice of the event, the entries; `fall` set once it ends.
    fn tick(&mut self) {
        if self.reign.is_none() && !self.crown() {
            return;
        }
        let (mut reign, base) = self.reign.take().expect("crowned");
        let salt = self.salt;
        let (g, c) = (&mut self.g, &mut self.c);
        let tpy = g.world.time_unit.ticks_per_year;
        // A ruler mindful of the testament, at its strength this year.
        let mindful;
        let auto = match g.world.testament.as_ref().and_then(|t| t.since) {
            Some(_) => {
                mindful = testament::chooser(&base, &g.data, &g.world);
                &mindful
            }
            None => &base,
        };
        let fall = match fallen(g) {
            None if g.world.tick.year(g.world.time_unit) >= g.data.sim.max_years => {
                Some(FallReason::Alive)
            }
            fall => fall,
        };
        if let Some(fall) = fall {
            self.end(reign, fall);
            return;
        }
        if g.world.ruler.age >= g.data.sim.regency_age {
            g.world.flags.remove(&g.data.sim.regency_flag);
        }
        if g.world.tick.0.is_multiple_of(tpy)
            && let Some((id, target)) = auto.action(g)
            // Listed by available_actions; only the slot may be gone.
            && g.start(&id, target, false).is_ok()
            && let Some(a) = g.data.actions.iter().find(|a| a.id == id)
        {
            reign.count(&a.cause_tag);
            if testament::faithful_action(g, a, &base) {
                keep(g, &mut reign);
            }
        }
        let holders: Vec<Holder> = g
            .world
            .provinces
            .values()
            .map(|p| p.holder.clone())
            .collect();
        let mut war = g.world.war.as_ref().map(|x| x.enemy.clone());
        let first = successor(&g.world, &g.data).map(|i| g.world.heirs[i].clone());
        let law = g.data.heirs.law(&g.world).map(|l| l.flag.clone());
        let laws: Vec<String> = (g.data.laws_in_force(&g.world))
            .map(|l| l.id.clone())
            .collect();
        let step = g.wait().expect("the reign goes on");
        let w = &g.world;
        if !g.data.influences.is_empty() && w.tick.0.is_multiple_of(tpy) {
            c.nodes.push(NodeYear {
                year: w.tick.year(w.time_unit),
                axes: g.data.axes.iter().map(|a| w.axes[&a.id]).collect(),
                lagged: w.lagged.clone(),
            });
        }
        // Within a tick only the yearly age risk takes an heir; events do on resolve.
        if let Some(h) = first.filter(|h| g.world.heir_index(h.id).is_none()) {
            let s = &g.data.sim;
            let (title, text) = &s.texts.heir_died;
            let text = variant(&s.texts, "heir_died", text, salt, c.entries.len());
            let named = [("heir", h.name.as_str(), Some(h.sex))];
            let fill = |t: &str| text::fill(t, &g.data.names, &named);
            let causes = causes(&g.world, [MarkKey::Heir(h.id)].into());
            let e = entry(g, (fill(title), fill(&text)), s.notable, causes);
            // `h` is from before the tick; heirs age a year before the death roll.
            if h.age + 1 >= s.heir_death_age {
                c.entries.push(e);
                reign.count("heir_died");
            } else if g.world.heirs.is_empty() {
                self.last_heir = Some((c.entries.len(), e));
            }
        }
        if !g.world.heirs.is_empty() {
            self.last_heir = None;
        }
        match step {
            Step::Idle => {}
            Step::Event(v) => {
                // Causes and chain as the world stood before the choice; only entries
                // need them. An omen is told whatever its importance.
                let p = g.pending_event.as_ref().expect("an event waits");
                let e = g.data.events.iter().find(|e| e.id == p.event_id);
                let e = e.expect("pending events exist");
                let told = (v.importance >= g.data.sim.threshold || e.omen).then(|| {
                    let keys = event_keys(&g.world, e, p);
                    (causes(&g.world, keys), chain(&g.data, &g.world, e))
                });
                // An event a neighbour's stance offers is his move: a war it starts
                // is his attack, not the ruler's breach of a Peace order.
                let ai = &g.data.neighbour_ai;
                let attack = [&ai.expand, &ai.defend, &ai.trade, &ai.wait]
                    .iter()
                    .any(|s| s.events.iter().any(|(id, _)| *id == v.event_id));
                let idx = auto.choose(g, &v.choices);
                let faithful = testament::faithful(g, &v.choices, idx, &base);
                let mut past = g.told(idx).unwrap_or(v.text);
                reign.count(&v.choices[idx].cause_tag);
                g.resolve(idx, false).expect("a listed choice");
                if attack && war.is_none() {
                    war = g.world.war.as_ref().map(|x| x.enemy.clone());
                }
                if faithful {
                    keep(g, &mut reign);
                    let n = TESTAMENT + c.entries.len() as u64;
                    past += &format!(" {}", testament::faithful_text(&g.data, &g.world, salt, n));
                }
                if let Some((causes, chain)) = told {
                    let e = entry(g, (v.title, past), v.importance, causes);
                    let e = ChronicleEntry {
                        event: Some(v.event_id),
                        chain,
                        ..e
                    };
                    let last = (self.told.take()).filter(|(i, _)| i + 1 == c.entries.len());
                    let n = FUSE + c.entries.len() as u64;
                    let fused = last.and_then(|(i, at)| {
                        let a = c.entries[i].told(at.as_ref());
                        let (title, text) =
                            fuse(&g.data, &g.world, &a, &e.told(v.target.as_ref()), salt, n)?;
                        Some((i, title, text))
                    });
                    let joined = fused.is_some();
                    if let Some((i, title, text)) = fused {
                        let a = &mut c.entries[i];
                        a.text = text;
                        if !title.is_empty() {
                            a.title = title;
                        }
                    } else {
                        self.told = Some((c.entries.len(), v.target));
                    }
                    c.entries.push(ChronicleEntry { joined, ..e });
                }
            }
            Step::ReignEnded(end) => {
                died(&mut g.world, &g.data, Some(&end.cause));
                let last = c.rulers.last_mut().expect("a ruler reigned");
                (last.end, last.cause) = (end.tick, Some(end.cause));
                let told = reign_deeds(c, reign.from, g, salt);
                finish(c, reign, told, &g.world, &g.data, salt);
                return;
            }
        }
        province_entries(g, &holders, c, &mut reign, salt);
        law_entry(g, law, c, &mut reign, salt);
        laws_entry(g, &laws, c, &mut reign, salt);
        if testament::broken(&g.world, &laws, &holders, war.as_ref()) {
            breach(g, c, &mut reign, salt);
        }
        self.reign = Some((reign, base));
    }

    /// The chronicle as it ends: the years, the family tree, the axes, the epilogue; a
    /// dynasty still going on ends `Alive` under its ruler (one fallen unseen as it is), and
    /// so do the foreign kingdoms beside it.
    pub fn close(mut self) -> Chronicle {
        if self.fall.is_none() {
            let fall = fallen(&self.g).unwrap_or(FallReason::Alive);
            match self.reign.take() {
                Some((reign, _)) => self.end(reign, fall),
                None => self.fall = Some(fall),
            }
        }
        let (g, mut c, salt) = (self.g, self.c, self.salt);
        c.fall = self.fall.expect("ended above");
        let w = &g.world;
        c.years = w.tick.year(w.time_unit);
        c.kin = w.kin.clone();
        c.axes = w.axes.clone();
        c.deserted = w.deserted - self.deserted;
        let last = c.rulers.last().expect("a ruler reigned");
        let s = &g.data.sim;
        let told = s.texts.fall_told.iter().find(|(f, _)| *f == c.fall);
        let told = text::pick(told.map_or(&[][..], |(_, v)| v), salt, EPILOGUE);
        let named = [("ruler", last.name.as_str(), Some(g.world.ruler.sex))];
        c.epilogue =
            text::fill(told, &g.data.names, &named).replace("{year}", &w.year().to_string());
        c.realms = (g.realms.list.into_iter())
            .map(|(id, r)| (id, r.close()))
            .collect();
        c
    }
}

/// Namespaces of `text::pick` beside the entries (numbered by their index).
const LIFE: u64 = 1 << 40;
const TESTAMENT: u64 = 1 << 44;
const EPILOGUE: u64 = 1 << 48;
const FUSE: u64 = 1 << 52;

/// An event told, as `fuse` joins it.
pub struct Told<'a> {
    pub event: &'a str,
    pub target: Option<&'a Target>,
    pub tick: Tick,
    /// The start of its chain (`Chain.nodes`).
    pub root: Option<&'a AxisId>,
    pub text: &'a str,
}

impl ChronicleEntry {
    /// The entry of an event about `target`, as `fuse` takes it.
    pub fn told<'a>(&'a self, target: Option<&'a Target>) -> Told<'a> {
        Told {
            event: self.event.as_deref().unwrap_or_default(),
            target,
            tick: self.tick,
            root: self.chain.as_ref().and_then(|c| c.nodes.first()),
            text: &self.text,
        }
    }
}

/// `b` told after `a` in one text, and the title of the pair (empty: keep the first's), when
/// they are linked a year apart at most (`SimTexts.fuse`); None: two entries. The join is
/// picked by `salt` and `n`, never the main rng.
pub fn fuse(
    d: &Data,
    w: &World,
    a: &Told,
    b: &Told,
    salt: u64,
    n: u64,
) -> Option<(String, String)> {
    let f = &d.sim.texts.fuse;
    let omen = |t: &Told| (d.events.iter().chain(&d.sim_events)).any(|e| e.id == t.event && e.omen);
    let years = (b.tick.0 - a.tick.0) / w.time_unit.ticks_per_year;
    if a.event.is_empty() || b.event.is_empty() || omen(a) || omen(b) || years > 1 {
        return None;
    }
    let pair = (f.pairs.iter())
        .find(|p| p.first.iter().any(|e| e == a.event) && p.then.iter().any(|e| e == b.event));
    let target = a.target.is_some() && a.target == b.target;
    let root = a.root.is_some() && a.root == b.root;
    let joins = match pair {
        Some(p) => &p.joins,
        None if !target && !root => return None,
        None if years == 0 => &f.same_year,
        None => &f.next_year,
    };
    let join = Some(text::pick(joins, salt, n)).filter(|j| !j.is_empty())?;
    let first = a.text.trim_end().trim_end_matches('.');
    let text = join
        .replace("{a}", first)
        .replace("{b}", &lower(d, w, b.text));
    Some((pair.map_or(String::new(), |p| p.title.clone()), text))
}

/// A decision true to the testament: `testament.faithful` shifts times its strength.
fn keep(g: &mut Game, reign: &mut Reign) {
    let k = testament::strength(&g.data, &g.world);
    let shifts = g
        .data
        .testament
        .as_ref()
        .map_or(&[][..], |r| &r.faithful.axes);
    for (a, v) in shifts {
        add_axis(&mut g.world, &g.data, a, *v * k);
    }
    g.world.recompute_loyalty(&g.data);
    reign.count("testament_kept");
}

/// The order of the testament broken: `testament.breach` shifts times its strength, an
/// entry, and the order binds no more.
fn breach(g: &mut Game, c: &mut Chronicle, reign: &mut Reign, salt: u64) {
    let k = testament::strength(&g.data, &g.world);
    let shifts = g.data.testament.as_ref().map_or(&[][..], |r| &r.breach);
    for (a, v) in shifts {
        add_axis(&mut g.world, &g.data, a, *v * k);
    }
    g.world.recompute_loyalty(&g.data);
    if let Some(t) = &mut g.world.testament {
        t.broken = Some(g.world.tick);
    }
    let n = TESTAMENT + c.entries.len() as u64;
    if let Some(told) = testament::breach_text(&g.data, &g.world, salt, n) {
        c.entries.push(entry(g, told, g.data.sim.notable, vec![]));
    }
    reign.count("testament_broken");
}

/// The text of entry kind `key`: `own` or one of its `SimTexts.variants`, by the entry's
/// index `n`.
fn variant(t: &crate::data::SimTexts, key: &str, own: &str, salt: u64, n: usize) -> String {
    let all: Vec<String> = std::iter::once(own.to_string())
        .chain(t.variants.get(key).into_iter().flatten().cloned())
        .collect();
    text::pick(&all, salt, n as u64).to_string()
}

/// The founder's reign as `finish` takes it, from the world at its end: his deeds by the
/// cause tags of his decisions (the marks they left), his traits and the laws he left in
/// force; the hints of his heaviest decisions as sentences.
fn founder_reign(w: &World, d: &Data, salt: u64) -> (Reign, Vec<String>) {
    let by = founder_decisions(w);
    let life = &d.sim.texts.life;
    let named = [("ruler", w.ruler.name.as_str(), Some(w.ruler.sex))];
    let year = (w.start_year + w.ruler.reign_start.year(w.time_unit)).to_string();
    let accession = text::pick(&life.founder, salt, LIFE);
    let reign = Reign {
        sex: w.ruler.sex,
        accession: text::fill(accession, &d.names, &named).replace("{year}", &year),
        deeds: founder_deeds(w),
        from: 0,
    };
    let mut heaviest: Vec<_> = by.into_values().collect();
    heaviest.sort_by_key(|(weight, _)| Reverse(*weight));
    let mut told: Vec<String> = vec![];
    for (_, tag) in heaviest {
        let hint = d.hints.get(tag).map(|h| format!("{}.", text::capital(h)));
        if let Some(h) = hint.filter(|h| !told.contains(h)) {
            told.push(h);
        }
    }
    told.truncate(life.deeds);
    (reign, told)
}

/// The founder's decisions by the marks they left: their weight and cause tag.
fn founder_decisions(w: &World) -> BTreeMap<usize, (Fx, &str)> {
    let mut by: BTreeMap<usize, (Fx, &str)> = BTreeMap::new();
    for t in w.marks.values().flatten() {
        let m = by.entry(t.decision_idx).or_insert((Fx(0), &t.cause_tag));
        m.0 = m.0 + t.weight;
    }
    by
}

/// The founder's deeds as `Epithet` counts them: the cause tag of every decision that left a
/// mark, `law` for every law he brought in.
pub(crate) fn founder_deeds(w: &World) -> BTreeMap<String, u32> {
    let mut deeds: BTreeMap<String, u32> = BTreeMap::new();
    let tags = founder_decisions(w).into_values().map(|(_, tag)| tag);
    for tag in tags.chain(w.laws.keys().map(|_| "law")) {
        *deeds.entry(tag.to_string()).or_default() += 1;
    }
    deeds
}

/// The main entries of the reign from entry `from` on, as sentences of `life.deed` (of
/// `life.same_year` for one of the year before it): the most important first (the earliest
/// on a tie), then in order of time.
fn reign_deeds<'a>(c: &Chronicle, from: usize, g: &'a Game, salt: u64) -> Vec<String> {
    let (d, w) = (&g.data, &g.world);
    let life = &d.sim.texts.life;
    let mut main: Vec<usize> = (from..c.entries.len())
        .filter(|i| !c.entries[*i].joined)
        .collect();
    main.sort_by_key(|i| (Reverse(c.entries[*i].importance), *i));
    main.truncate(life.deeds);
    main.sort();
    let mut last: Option<u32> = None;
    (main.into_iter())
        .map(|i| {
            let e = &c.entries[i];
            let deed = lower(d, w, text::first_sentence(&e.text));
            let deed = deed.trim_end_matches('.');
            let year = e.tick.date(w.time_unit, w.start_year);
            let at = e.tick.year(w.time_unit);
            let gap = last.replace(at).map(|y| at - y);
            let some = |v: &'a Vec<String>| (!v.is_empty()).then_some(v);
            let phrases = match gap {
                None => None,
                Some(0) => some(&life.same_year),
                Some(g) if g <= life.soon_years => some(&life.soon),
                Some(_) => some(&life.later),
            };
            let phrases = phrases.unwrap_or(&life.deed);
            let phrase = text::pick(phrases, salt, LIFE + i as u64);
            phrase.replace("{deed}", deed).replace("{year}", &year)
        })
        .collect()
}

/// `s` from a small letter, unless its first word is a name.
fn lower(d: &Data, w: &World, s: &str) -> String {
    let word = s.split([' ', ',', '.']).next().unwrap_or_default();
    let named = d.names.cases.contains_key(word) || w.provinces.values().any(|p| p.name == word);
    match named {
        true => s.to_string(),
        false => (s.chars().next().into_iter())
            .flat_map(char::to_lowercase)
            .chain(s.chars().skip(1))
            .collect(),
    }
}

/// The last ruler's life, his reign over (`end` and `cause` set; a fall under him is
/// `c.fall`): the epithet by his deeds and the paragraph of `life`; `{epithet}` names it.
fn finish(c: &mut Chronicle, reign: Reign, told: Vec<String>, w: &World, d: &Data, salt: u64) {
    let (t, n) = (&d.sim.texts, c.rulers.len() as u64 * 16);
    let r = c.rulers.last_mut().expect("a ruler reigned");
    let unit = w.time_unit;
    let years = (r.end.0 - r.start.0) / unit.ticks_per_year;
    let mut deeds = reign.deeds;
    for t in &r.traits {
        *deeds.entry(format!("trait:{t}")).or_default() += 1;
    }
    let score = |e: &Epithet| {
        let count: u32 = e
            .deeds
            .iter()
            .map(|k| deeds.get(k).copied().unwrap_or(0))
            .sum();
        let fits = count >= e.min && e.years.is_none_or(|y| years <= y);
        fits.then(|| count * 1000 / e.min.max(1))
    };
    let mut best: Option<(u32, &Epithet)> = None;
    for e in &t.epithets {
        if let Some(s) = score(e).filter(|s| best.is_none_or(|(b, _)| *s > b)) {
            best = Some((s, e));
        }
    }
    let female = reign.sex == Sex::Female;
    let (name, why) = best.map_or(("", ""), |(_, e)| {
        // One of its names, by seed apart from the main rng, as `text::pick`.
        let all: Vec<_> = std::iter::once(&e.name).chain(&e.also).collect();
        let k = Rng::from_seed(salt ^ (LIFE + n + 4)).range(0, all.len() as i64) as usize;
        let name = if female { &all[k].1 } else { &all[k].0 };
        (name.as_str(), text::pick(&e.told, salt, LIFE + n + 2))
    });
    r.epithet = name.to_string();
    let life = &t.life;
    let end = match &r.cause {
        Some(cause) => life.ends.get(cause),
        None => life
            .falls
            .iter()
            .find(|(f, _)| *f == c.fall)
            .map(|(_, v)| v),
    };
    let end = text::pick(end.map_or(&[][..], |v| v), salt, LIFE + n + 1);
    let will = testament::life(d, w, &deeds, salt, LIFE + n + 3);
    let phrases = std::iter::once(reign.accession.as_str())
        .chain([why])
        .chain(told.iter().map(String::as_str))
        .chain([will.as_str(), end]);
    let named = [
        ("ruler", r.name.as_str(), Some(reign.sex)),
        ("epithet", name, Some(reign.sex)),
    ];
    let year = (w.start_year + r.end.year(unit)).to_string();
    let fill = |p: &str| {
        text::fill(p, &d.names, &named)
            .replace("{year}", &year)
            .replace("{years}", &text::plural(years, &life.years))
    };
    let all: Vec<String> = phrases.filter(|p| !p.is_empty()).map(fill).collect();
    r.biography = all.join(" ");
}

/// The next ruler: the heir `successor` names. He keeps his name unless it is the newborn placeholder, then one from
/// `names.rulers`; traits roll by `sim.traits`; health `sim.ruler_health`. None: no heir.
pub fn succession(w: &World, data: &Data, rng: &mut Rng) -> Option<Ruler> {
    let heir = &w.heirs[successor(w, data)?];
    let pool = &data.names.rulers;
    let name = match heir.name == data.new_heir.name && !pool.is_empty() {
        true => pool[rng.range(0, pool.len() as i64) as usize].clone(),
        false => heir.name.clone(),
    };
    let traits = (data.sim.traits.iter())
        .filter(|t| {
            let status = match heir.status {
                HeirStatus::Home => Fx(0),
                HeirStatus::Studying(_) => t.studying,
                HeirStatus::Hostage(_) => t.hostage,
            };
            let chance = t.percent + heir.ability * t.ability_k + status;
            rng.range(0, Fx::from_int(100).0) < chance.0
        })
        .map(|t| t.id.clone())
        .collect();
    Some(Ruler {
        name,
        age: heir.age,
        health: data.sim.ruler_health,
        traits,
        reign_start: w.tick,
        sex: heir.sex,
    })
}

/// Who succeeds now: the designated heir (`World.designated`) while he lives and is no
/// hostage, else `rightful`. None: nobody may.
pub fn successor(w: &World, data: &Data) -> Option<usize> {
    let named = w.designated.and_then(|id| w.heir_index(id));
    let home = named.filter(|i| !matches!(w.heirs[*i].status, HeirStatus::Hostage(_)));
    home.or_else(|| rightful(w, data))
}

/// Who the rule of the law in force puts first (`SuccessionRule`) among the lawful heirs, among
/// recognized bastards only without one; `next_heir` without a law. None: nobody may.
pub fn rightful(w: &World, data: &Data) -> Option<usize> {
    let Some(law) = data.heirs.law(w) else {
        return next_heir(w);
    };
    let h = &w.heirs;
    let son = |i: &usize| h[*i].sex == Sex::Male;
    let child = |i: &usize| h[*i].id >= w.line_from;
    let lawful = h.iter().any(|x| !x.bastard);
    let mut all = (0..h.len()).filter(|i| !lawful || !h[*i].bastard);
    match law.rule {
        SuccessionRule::Absolute => all.next(),
        SuccessionRule::Male | SuccessionRule::Partition => {
            all.min_by_key(|i| (!child(i), !son(i), *i))
        }
        SuccessionRule::Salic => all.find(son),
        SuccessionRule::Seniority => {
            all.min_by_key(|i| (!son(i), child(i), Reverse(h[*i].age), *i))
        }
        SuccessionRule::Elective => all.min_by_key(|i| (Reverse(h[*i].ability), *i)),
    }
}

/// Who succeeds now: the index in `heirs` of the highest claim, the eldest on a tie.
pub fn next_heir(w: &World) -> Option<usize> {
    // max_by_key keeps the last of equals; going backwards, that is the eldest.
    (0..w.heirs.len()).rev().max_by_key(|&i| w.heirs[i].claim)
}

/// Crowns the next heir, the rightful one with at least `Law.rightful_claim`: a claim below
/// the law's `crisis_claim` contests the succession (`abdication.contested_flag`), and so
/// may the rivals (`Law.dispute_per_heir`), a woman under `Law.female_heir`, an heir named
/// over the law (`heirs.designate_dispute`), a child (`heirs.dispute_minor`) and a weak
/// heir (`heirs.dispute_weak`); a child
/// reigns under the regency flag, the other heirs become the collateral line, the flags of
/// the last reign (`sim.reign_flags`) go. None: no heir; else the new reign, told how it
/// began (`sim.texts.life`).
fn crown(g: &mut Game, c: &mut Chronicle, salt: u64) -> Option<Reign> {
    // The heir of a will goes first, as one designated, for this coronation only.
    let sealed = g.world.testament.as_mut().and_then(|t| t.heir.take());
    let w = &g.world;
    let home =
        |id: &u32| (w.heir_index(*id)).is_some_and(|i| w.heirs[i].status == HeirStatus::Home);
    let sealed = sealed.filter(home);
    if sealed.is_some() {
        g.world.designated = sealed;
    }
    let ruler = succession(&g.world, &g.data, &mut g.rng)?;
    let (d, w, rng) = (&g.data, &mut g.world, &mut g.rng);
    let i = successor(w, d).expect("succession found one");
    let lawful = rightful(w, d) == Some(i);
    let mut heir = w.heirs.remove(i);
    let willed = sealed == Some(heir.id);
    w.designated = None;
    if let Some(l) = d.heirs.law(w).filter(|_| lawful && !heir.bastard) {
        heir.claim = heir.claim.max(l.rightful_claim);
    }
    // The late ruler's unions end with him; the new one's come with him to the throne.
    w.unions.retain(|_, u| u.spouse.is_some());
    for u in w.unions.values_mut().filter(|u| u.spouse == Some(heir.id)) {
        u.spouse = None;
    }
    // The late ruler's other sons, eldest first, for a partition.
    let sons: Vec<_> = (w.heirs.iter())
        .filter(|h| h.id >= w.line_from && h.sex == Sex::Male)
        .map(|h| (h.id, h.name.clone()))
        .collect();
    let year = w.year();
    if let Some(k) = w.kin.iter_mut().find(|k| k.heir == Some(heir.id)) {
        (k.name, k.crowned) = (ruler.name.clone(), Some(year));
    }
    // His brothers and sisters become the collateral line, behind his children.
    w.line_from = w.next_heir_id;
    // Children before the coronation: lawful from his wedding on, bastards before it.
    let wed = heir.married.then(|| {
        let years = heir.married_in.map(|y| year.saturating_sub(y));
        years.map_or(d.heirs.adult_age, |y| ruler.age.saturating_sub(y))
    });
    born_before(d, w, rng, ruler.age, wed);
    let married = d.heirs.married_flag.clone();
    match heir.married {
        true => w.flags.insert(married),
        false => w.flags.remove(&married),
    };
    if let Some(l) = d.heirs.law(w) {
        // Chances in percent, rolled only where they apply and are above 0.
        let mut roll = |p: Fx| p > Fx(0) && rng.range(0, Fx::from_int(100).0) < p.0;
        // Each heir left is a rival: a chance of dispute per head.
        let quarrel = roll(l.dispute_per_heir * Fx::from_int(w.heirs.len() as i64));
        let queen = heir.sex == Sex::Female && l.female_heir.is_some();
        // Named over the rightful heir, who keeps his claim: a rival.
        let dispute = match d.testament.as_ref().filter(|_| willed) {
            Some(t) => t.heir_dispute,
            None => d.heirs.designate_dispute,
        };
        let named = !lawful && roll(dispute);
        let h = &d.heirs;
        let minor = ruler.age < d.sim.regency_age && roll(h.dispute_minor);
        let weak = heir.ability < h.dispute_weak.0 && roll(h.dispute_weak.1);
        if heir.claim < l.crisis_claim || quarrel || queen || named || minor || weak {
            w.flags.insert(d.abdication.contested_flag.clone());
        }
    }
    for f in &d.sim.reign_flags {
        w.flags.remove(f);
    }
    let law = d.heirs.law(w);
    let lands = match law.filter(|l| l.rule == SuccessionRule::Partition) {
        Some(l) => partition(w, l.house, &sons),
        None => vec![],
    };
    if !lands.is_empty() {
        w.recompute_crown_power(d);
    }
    if ruler.age < d.sim.regency_age {
        w.flags.insert(d.sim.regency_flag.clone());
    }
    let cheer = coronation(w, d, heir.claim, &ruler, salt ^ c.entries.len() as u64);
    let t = &d.sim.texts;
    let prev = c.rulers.last().expect("the founder reigned").full_name();
    let named = [
        ("ruler", ruler.name.as_str(), Some(ruler.sex)),
        ("prev", &prev, Some(w.ruler.sex)),
    ];
    let fill = |s: &str| text::fill(s, &d.names, &named);
    let (title, text) = &t.crowned;
    let mut told = (
        fill(title),
        fill(&variant(t, "crowned", text, salt, c.entries.len())),
    );
    let tt = d.testament.as_ref().map(|t| &t.texts);
    if let Some(tt) = tt.filter(|_| willed && !lawful) {
        let n = TESTAMENT + c.entries.len() as u64;
        told.1 = format!("{} {}", told.1, fill(text::pick(&tt.crowned, salt, n)));
    }
    if let Some(cheer) = cheer {
        told.1 = format!("{} {cheer}", told.1);
    }
    let life = &t.life;
    let contested = w.flags.contains(&d.abdication.contested_flag);
    let accession: &[String] = match () {
        _ if ruler.age < d.sim.regency_age => &life.regency,
        _ if !lawful && willed && tt.is_some() => &tt.expect("checked").willed,
        _ if !lawful => &life.designated,
        _ if contested => &life.contested,
        _ => &life.lawful,
    };
    let n = LIFE + c.rulers.len() as u64 * 16;
    let law = d.heirs.law(w).map_or("", |l| &l.name);
    let accession = (fill(text::pick(accession, salt, n)))
        .replace("{law}", law)
        .replace("{year}", &year.to_string());
    let sex = ruler.sex;
    c.rulers.push(RulerRecord {
        designated: !lawful,
        ..record(&ruler)
    });
    w.ruler = ruler;
    if let Some(t) = d.testament.as_ref().filter(|_| willed) {
        let k = testament::strength(d, w);
        for (a, v) in &t.heir {
            add_axis(w, d, a, *v * k);
        }
        w.recompute_loyalty(d);
    }
    (g.ended, g.reported) = (None, false);
    let causes = causes(&g.world, [MarkKey::Heir(heir.id)].into());
    c.entries.push(entry(g, told, g.data.sim.notable, causes));
    if !lands.is_empty() {
        let (title, text) = &g.data.sim.texts.partition;
        let text = variant(&g.data.sim.texts, "partition", text, salt, c.entries.len());
        let told = (
            title.clone(),
            ruled(g, &text).replace("{lands}", &lands.join(", ")),
        );
        c.entries.push(entry(g, told, g.data.sim.notable, vec![]));
    }
    Some(Reign {
        sex,
        accession,
        deeds: BTreeMap::new(),
        from: c.entries.len(),
    })
}

/// The axes at a coronation (`Data.coronation`, see `CoronationRules`, then a queen's
/// `Law.female_heir`) for a new ruler of `claim`. Returns how the chronicle tells the trait that moved an axis most, if it is told.
fn coronation(w: &mut World, d: &Data, claim: Fx, ruler: &Ruler, salt: u64) -> Option<String> {
    let c = &d.coronation;
    for f in &d.factions {
        let def = d
            .axes
            .iter()
            .find(|a| a.id == f.axis)
            .expect("checked on load");
        let back = (crate::graph::anchor(d, w, def) - w.axes[&f.axis]) * c.reset;
        add_axis(w, d, &f.axis, back);
    }
    if let Some((a, k)) = &c.legitimacy_from_claim {
        let toward = (claim - w.axes[a]) * *k;
        add_axis(w, d, a, toward);
    }
    let contested = w.flags.contains(&d.abdication.contested_flag);
    let law = d.heirs.law(w);
    let queen = law
        .and_then(|l| l.female_heir.as_ref())
        .filter(|_| ruler.sex == Sex::Female);
    let law = law.map_or(&[][..], |l| &l.coronation);
    let shifts = c.contested.iter().filter(|_| contested).chain(law);
    for (a, v) in shifts.chain(queen.into_iter().flatten()) {
        add_axis(w, d, a, *v);
    }
    let traits = d.sim.traits.iter().filter(|t| ruler.traits.contains(&t.id));
    let mut most: Option<(Fx, &TraitRule)> = None;
    for t in traits {
        for (a, v) in &t.axes {
            add_axis(w, d, a, *v);
        }
        let shift = t.axes.iter().map(|(_, v)| Fx(v.0.abs())).max();
        if let Some(shift) = shift.filter(|s| most.is_none_or(|(m, _)| *s > m)) {
            most = Some((shift, t));
        }
    }
    w.recompute_loyalty(d);
    let t = most?.1;
    let all: Vec<String> = (std::iter::once(&t.told).chain(&t.retold))
        .map(|(king, queen)| if ruler.sex == Sex::Male { king } else { queen }.clone())
        .collect();
    let told = text::pick(&all, salt, LIFE);
    (!told.is_empty()).then(|| told.to_string())
}

/// `SuccessionRule::Partition`: each son, eldest first, gets the crown province farthest from
/// the capital (the smallest id on a tie) as a house of his name with `(loyalty, strength)`;
/// the capital stays with the crown. «Сын — земля» for every grant.
fn partition(w: &mut World, (loyalty, strength): (Fx, Fx), sons: &[(u32, String)]) -> Vec<String> {
    let mut told = vec![];
    for (id, name) in sons {
        let far = (w.provinces.values())
            .filter(|p| p.holder == Holder::Crown && p.id != w.capital.province)
            .max_by_key(|p| (p.distance_to_capital, Reverse(&p.id)));
        let Some(p) = far.map(|p| p.id.clone()) else {
            break;
        };
        let house = VassalId(format!("{name}_{id}"));
        let v = Vassal {
            id: house.clone(),
            name: name.clone(),
            loyalty,
            strength,
        };
        w.vassals.insert(house.clone(), v);
        let p = w.provinces.get_mut(&p).expect("found above");
        p.holder = Holder::Vassal(house);
        told.push(format!("{name} — {}", p.name));
    }
    told
}

/// The children a new ruler of `age`, wed at age `wed`, had before the coronation: a roll of
/// `heirs.birth` for every adult year (times `unmarried` before the wedding: a bastard), and
/// each child born then a roll of `heirs.death` for every year of its own; ability grown at
/// home until adulthood.
fn born_before(d: &Data, w: &mut World, rng: &mut Rng, age: u32, wed: Option<u32>) {
    let r = &d.heirs;
    for at in r.adult_age..age {
        let bastard = wed.is_none_or(|m| at < m);
        let k = if bastard {
            r.unmarried
        } else {
            Fx::from_int(1)
        };
        if rng.range(0, Fx::from_int(100).0) >= (crate::data::by_age(&r.birth, at) * k).0 {
            continue;
        }
        let years = age - at;
        let risk = |y| crate::data::by_age(&r.death, y).0;
        if (0..years).any(|y| rng.range(0, 1000 * Fx::SCALE) < risk(y)) {
            continue;
        }
        let mut h = d.newborn_of(w.next_heir_id, r.sex(rng));
        h.bastard = bastard;
        let grown = r.growth_home * Fx::from_int(years.min(r.adult_age).into());
        (h.age, h.ability) = (years, (h.ability + grown).min(Fx::from_int(100)));
        w.add_heir(h);
    }
}

/// The reigning ruler's death in the family tree, unless the reign ended by abdication.
fn died(w: &mut World, d: &Data, cause: Option<&str>) {
    let year = w.year();
    let ruler = w.kin.iter_mut().rfind(|k| k.crowned.is_some());
    if let Some(k) = ruler.filter(|_| cause.is_some_and(|c| c != d.abdication.event)) {
        k.died = Some(year);
    }
}

fn record(r: &Ruler) -> RulerRecord {
    RulerRecord {
        name: r.name.clone(),
        traits: r.traits.clone(),
        start: r.reign_start,
        end: r.reign_start,
        cause: None,
        designated: false,
        epithet: String::new(),
        biography: String::new(),
    }
}

fn fallen(g: &Game) -> Option<FallReason> {
    let w = &g.world;
    let crown = |id: &ProvinceId| {
        w.provinces
            .get(id)
            .is_some_and(|p| p.holder == Holder::Crown)
    };
    if w.flags.contains(&g.data.sim.usurped_flag) {
        Some(FallReason::Usurped)
    } else if !w.provinces.keys().any(crown) {
        Some(FallReason::NoCrownLand)
    } else if !crown(&w.capital.province) {
        Some(FallReason::CapitalLost)
    } else {
        None
    }
}

/// An entry for every province that left the realm (crown and vassals) or joined it since
/// `holders` (in province order).
fn province_entries(g: &Game, holders: &[Holder], c: &mut Chronicle, r: &mut Reign, salt: u64) {
    let t = &g.data.sim.texts;
    let w = &g.world;
    for (p, was) in w.provinces.values().zip(holders) {
        let ((title, text), foreign, key) = match (was, &p.holder) {
            (Holder::Foreign(_), Holder::Foreign(_)) => continue,
            (_, Holder::Foreign(n)) => (&t.province_lost, n, "province_lost"),
            (Holder::Foreign(n), _) => (&t.province_gained, n, "province_gained"),
            _ => continue,
        };
        let neighbour = w.neighbours.get(foreign).map_or("", |n| &n.name);
        let named = [
            ("province", p.name.as_str(), None),
            ("neighbour", neighbour, None),
            ("ruler", w.ruler.name.as_str(), Some(w.ruler.sex)),
        ];
        let fill = |s: &str| text::fill(s, &g.data.names, &named);
        let causes = causes(w, [MarkKey::Province(p.id.clone())].into());
        let text = variant(t, key, text, salt, c.entries.len());
        let told = (fill(title), fill(&text));
        c.entries.push(entry(g, told, g.data.sim.notable, causes));
        r.count(key);
    }
}

/// An entry when the law in force is no longer `was`.
fn law_entry(g: &Game, was: Option<String>, c: &mut Chronicle, r: &mut Reign, salt: u64) {
    let Some(law) = g
        .data
        .heirs
        .law(&g.world)
        .filter(|l| Some(&l.flag) != was.as_ref())
    else {
        return;
    };
    let t = &g.data.sim.texts;
    let (title, text) = &t.law_changed;
    let text = variant(t, "law_changed", text, salt, c.entries.len());
    let told = (title.clone(), ruled(g, &text).replace("{law}", &law.name));
    c.entries.push(entry(g, told, g.data.sim.notable, vec![]));
    r.count("law");
}

/// `s` with the ruler of `g` filled in.
fn ruled(g: &Game, s: &str) -> String {
    let r = &g.world.ruler;
    text::fill(s, &g.data.names, &[("ruler", &r.name, Some(r.sex))])
}

/// An entry for every law other than of succession that came into force since `was` (the
/// laws then in force), and for every one repealed with none of its group in its place.
fn laws_entry(g: &Game, was: &[String], c: &mut Chronicle, r: &mut Reign, salt: u64) {
    let (d, t) = (&g.data, &g.data.sim.texts);
    let now: Vec<&LawDef> = d.laws_in_force(&g.world).collect();
    let succession = |l: &LawDef| d.heirs.laws.iter().any(|h| h.flag == l.id);
    let new = (now.iter()).filter(|l| !was.contains(&l.id) && !succession(l));
    let new = new.map(|l| ((&t.law_enacted, "law_enacted"), *l));
    let gone = (was.iter().filter_map(|id| d.law(id))).filter(|l| {
        !now.iter()
            .any(|n| n.id == l.id || !l.group.is_empty() && n.group == l.group)
    });
    let gone = gone.map(|l| ((&t.law_repealed, "law_repealed"), l));
    for (((title, text), key), l) in new.chain(gone) {
        let text = variant(t, key, text, salt, c.entries.len());
        let told = (title.clone(), ruled(g, &text).replace("{law}", &l.name));
        c.entries.push(entry(g, told, d.sim.notable, vec![]));
        if key == "law_enacted" {
            r.count("law");
        }
    }
}

fn entry(
    g: &Game,
    (title, text): (String, String),
    importance: u32,
    causes: Vec<CauseTag>,
) -> ChronicleEntry {
    let main = causes
        .first()
        .filter(|c| c.weight >= g.data.sim.hint_weight);
    // Hints are lowercase clauses; the entry tells one as a sentence of its own.
    let hint = main.and_then(|c| g.data.hints.get(&c.cause_tag));
    let hint = hint.map(|h| format!("{}.", capital(h)));
    ChronicleEntry {
        tick: g.world.tick,
        event: None,
        title,
        text,
        hint,
        importance,
        causes,
        snapshot: g.world.snapshot(),
        chain: None,
        joined: false,
    }
}

fn capital(s: &str) -> String {
    let mut chars = s.chars();
    let first = chars.next().into_iter().flat_map(char::to_uppercase);
    first.chain(chars).collect()
}

/// At most this many nodes in a `Chain`.
const CHAIN: usize = 3;

/// The chain behind event `e` (docs/design/hidden-state.html, section 7). It starts at the
/// node of the condition: the first axis of `when`, then of the `weight_bonus` that hold,
/// whose bound holds and which something pushes (`graph::pushes`), going the way the bound
/// asks. From each node it goes back along the largest push that way above
/// `graph::MARK_FLOW`, never to a node already in it, up to `CHAIN` nodes or a law. At its
/// start, the law so reached, or one pushing the first node that way, or one scaling an edge
/// on it; the decision behind that law (its mark), else the heaviest mark on the nodes.
/// None without a second node or a start. Told by the `chain:` keys of `data/hints.ron`:
/// `chain:<event>` the lead, `chain:<axis>+` and `-` the node going up or down,
/// `chain:then` between nodes, `chain:since_decision`, `since_law` and `since_mark` the
/// start ({year}, {law}, {hint}).
fn chain(d: &Data, w: &World, e: &Event) -> Option<Chain> {
    fn bounds(p: &Predicate, w: &World, out: &mut Vec<(AxisId, i64)>) {
        match p {
            Predicate::AxisAbove(a, _) if p.eval(w) => out.push((a.clone(), 1)),
            Predicate::AxisBelow(a, _) if p.eval(w) => out.push((a.clone(), -1)),
            Predicate::All(ps) | Predicate::Any(ps) => ps.iter().for_each(|p| bounds(p, w, out)),
            _ => {}
        }
    }
    let mut found = vec![];
    let bonus = e.weight_bonus.iter().filter(|(b, _)| b.eval(w));
    for p in std::iter::once(&e.when).chain(bonus.map(|(b, _)| b)) {
        bounds(p, w, &mut found);
    }
    let pushed = |a: &AxisId| crate::graph::pushes(d, w, a).next().is_some();
    let (first, mut dir) = found.into_iter().find(|(a, _)| pushed(a))?;
    let (mut nodes, mut dirs, mut edges) = (vec![first], vec![dir], vec![]);
    let way = |push: &(crate::graph::Push, Fx), dir: i64| push.1.0 * dir > MARK_FLOW_MILLI;
    let mut law = None;
    loop {
        let at = nodes.last().expect("one node at least").clone();
        let back = (crate::graph::pushes(d, w, &at)).filter(|p| {
            way(p, dir)
                && match p.0 {
                    crate::graph::Push::Edge(i) => !nodes.contains(&d.influences[i].from),
                    crate::graph::Push::Law(_) => true,
                }
        });
        // The first of the largest.
        let best = back.fold(None, |b: Option<(crate::graph::Push, Fx)>, p| match b {
            Some(b) if b.1.0.abs() >= p.1.0.abs() => Some(b),
            _ => Some(p),
        });
        match best {
            Some((crate::graph::Push::Law(l), _)) => {
                law = d.law(&l.id);
                break;
            }
            Some((crate::graph::Push::Edge(i), _)) if nodes.len() < CHAIN => {
                let edge = &d.influences[i];
                dir = (edge.source(i, w) - edge.rest).0.signum();
                if dir == 0 {
                    break;
                }
                nodes.push(edge.from.clone());
                dirs.push(dir);
                edges.push(i);
            }
            _ => break,
        }
    }
    // A law scaling an edge on the chain, the deepest first.
    let scaling = edges.iter().rev().find_map(|i| {
        let id = &d.influences[*i].id;
        let mut laws = d.laws_in_force(w);
        laws.find(|l| {
            l.edges
                .iter()
                .any(|(e, k)| e == id && *k != Fx::from_int(1))
        })
    });
    let law = law.or(scaling);
    let heaviest = |keys: Vec<MarkKey>| {
        let tags = keys.iter().filter_map(|k| w.marks.get(k)).flatten();
        tags.fold(None, |b: Option<&CauseTag>, t| match b {
            Some(b) if b.weight >= t.weight => Some(b),
            _ => Some(t),
        })
    };
    let mark = match law {
        Some(l) => heaviest(vec![MarkKey::Flag(l.id.clone())]),
        None => heaviest(
            nodes
                .iter()
                .rev()
                .map(|a| MarkKey::Axis(a.clone()))
                .collect(),
        ),
    };
    if nodes.len() < 2 && law.is_none() && mark.is_none() {
        return None;
    }
    let say = |k: &str| d.hints.get(&format!("chain:{k}")).cloned();
    let fill = |s: String, k: &str, v: &str| s.replace(k, v);
    let year = |l: &LawDef| {
        let at = w.laws.get(&l.id).map_or(0, |t| t.year(w.time_unit));
        (w.start_year + at).to_string()
    };
    let hint = |t: &CauseTag| d.hints.get(&t.cause_tag).cloned().unwrap_or_default();
    let since = match (law, mark) {
        (Some(l), Some(t)) => {
            say("since_decision").map(|s| fill(fill(s, "{year}", &year(l)), "{hint}", &hint(t)))
        }
        (Some(l), None) => {
            say("since_law").map(|s| fill(fill(s, "{year}", &year(l)), "{law}", &l.name))
        }
        (None, Some(t)) => say("since_mark").map(|s| fill(s, "{hint}", &hint(t))),
        (None, None) => None,
    };
    let name = |a: &AxisId| {
        let def = d.axes.iter().find(|x| x.id == *a);
        def.map_or(a.0.clone(), |x| x.name.to_lowercase())
    };
    let states = (nodes.iter().zip(&dirs).rev()).map(|(a, dir)| {
        let way = if *dir > 0 { "+" } else { "-" };
        say(&format!("{}{way}", a.0)).unwrap_or_else(|| name(a))
    });
    let then = say("then").map_or(String::new(), |t| format!("{t} "));
    let mut text = String::new();
    for (k, s) in states.enumerate() {
        match k {
            0 => text += &s,
            _ => text += &format!(", {then}{s}"),
        }
        if let (0, Some(since)) = (k, &since) {
            text += &format!(" ({since})");
        }
    }
    let text = match say(&e.id) {
        Some(lead) => format!("{}: {text}.", capital(&lead)),
        None => format!("{}.", capital(&text)),
    };
    nodes.reverse();
    Some(Chain {
        nodes,
        law: law.map(|l| l.id.clone()),
        decision: mark.map(|t| t.decision_idx),
        text,
    })
}

const MARK_FLOW_MILLI: i64 = crate::graph::MARK_FLOW.0;

/// The marks on `keys`, summed per decision, heaviest first, earlier decisions first on a tie.
fn causes(w: &World, keys: BTreeSet<MarkKey>) -> Vec<CauseTag> {
    let mut by_decision: BTreeMap<usize, CauseTag> = BTreeMap::new();
    for t in keys.iter().filter_map(|k| w.marks.get(k)).flatten() {
        let c = by_decision.entry(t.decision_idx).or_insert(CauseTag {
            weight: Fx(0),
            ..t.clone()
        });
        c.weight = c.weight + t.weight;
    }
    let mut causes: Vec<_> = by_decision.into_values().collect();
    causes.sort_by_key(|c| Reverse(c.weight));
    causes
}

/// What an event is about: the axes, flags and matching provinces of its `when` and of the
/// `weight_bonus` that hold, and its targets; a vassal's province brings all of his.
fn event_keys(w: &World, e: &Event, p: &PendingEvent) -> BTreeSet<MarkKey> {
    let mut keys = BTreeSet::new();
    let bonus = e.weight_bonus.iter().filter(|(b, _)| b.eval(w));
    for pred in std::iter::once(&e.when).chain(bonus.map(|(b, _)| b)) {
        predicate_keys(pred, w, &mut keys);
    }
    match &p.target {
        Some(Target::Province(id)) => {
            keys.insert(MarkKey::Province(id.clone()));
            if let Some(h @ Holder::Vassal(_)) = w.provinces.get(id).map(|q| &q.holder) {
                let domain = w.provinces.values().filter(|q| q.holder == *h);
                keys.extend(domain.map(|q| MarkKey::Province(q.id.clone())));
            }
        }
        Some(Target::Neighbour(n)) => {
            keys.insert(MarkKey::Neighbour(n.clone()));
        }
        Some(Target::Heir(id)) => {
            keys.insert(MarkKey::Heir(*id));
        }
        None => {}
    }
    keys.extend(p.neighbour.iter().map(|n| MarkKey::Neighbour(n.clone())));
    keys
}

fn predicate_keys(p: &Predicate, w: &World, keys: &mut BTreeSet<MarkKey>) {
    match p {
        Predicate::AxisAbove(a, _) | Predicate::AxisBelow(a, _) => {
            keys.insert(MarkKey::Axis(a.clone()));
        }
        Predicate::Flag(f) | Predicate::NotFlag(f) => {
            keys.insert(MarkKey::Flag(f.clone()));
        }
        Predicate::ProvinceWhere(f) => {
            let matching = w.provinces.values().filter(|q| f.matches(q, w));
            keys.extend(matching.map(|q| MarkKey::Province(q.id.clone())));
        }
        Predicate::All(ps) | Predicate::Any(ps) => {
            ps.iter().for_each(|p| predicate_keys(p, w, keys));
        }
        Predicate::Not(p) => predicate_keys(p, w, keys),
        _ => {}
    }
}

/// The laws a change brings in (+1) and ends (-1): an enacted law and the laws of its group
/// in force, a repealed one; nothing for a law already so.
fn law_changes<'a>(d: &'a Data, w: &'a World, id: &str, enact: bool) -> Vec<(&'a LawDef, Fx)> {
    let Some(l) = d.law(id).filter(|_| enact != w.flags.contains(id)) else {
        return vec![];
    };
    let one = Fx::from_int(1);
    let mates = (d.laws.list.iter())
        .filter(|o| !l.group.is_empty() && o.group == l.group && w.flags.contains(&o.id));
    match enact {
        true => std::iter::once((l, one))
            .chain(mates.map(|o| (o, Fx(0) - one)))
            .collect(),
        false => vec![(l, Fx(0) - one)],
    }
}

/// What the pressing factions (`Laws.pressure`) make of a law in force: the sum of its
/// anchor shifts and resistance on their axes, of a strong faction (above the band) all of
/// them, of a weak one (below) only those against it; below 0 they want it gone. None on a
/// law with no repeal (`keep`): succession is not a matter for the factions.
fn pressure(l: &LawDef, w: &World, d: &Data) -> Fx {
    let Some((low, high)) = d.laws.pressure.filter(|_| !l.keep) else {
        return Fx(0);
    };
    let faction = |a: &AxisId| d.factions.iter().any(|f| f.axis == *a);
    let press = |(a, v): &(AxisId, Fx)| match w.axes[a] {
        x if !faction(a) || (low..=high).contains(&x) => Fx(0),
        x if x < low => (*v).min(Fx(0)),
        _ => *v,
    };
    (l.anchors.iter().chain(&l.resistance)).fold(Fx(0), |s, p| s + press(p))
}

/// Chooses for the simulated rulers. An option scores `sum(weights[key] * amount)` over its
/// effects (see `worth`) plus a roll in `0..=noise`; the best wins, the first on a tie.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoChooser {
    pub weights: BTreeMap<String, Fx>,
    pub noise: Fx,
}

impl AutoChooser {
    /// `sim.auto.base` plus the weights of every trait the ruler has.
    pub fn for_ruler(data: &Data, ruler: &Ruler) -> AutoChooser {
        let a = &data.sim.auto;
        let mut weights = a.base.clone();
        for (k, v) in ruler
            .traits
            .iter()
            .filter_map(|t| a.traits.get(t))
            .flatten()
        {
            let w = weights.entry(k.clone()).or_default();
            *w = *w + *v;
        }
        AutoChooser {
            weights,
            noise: a.noise,
        }
    }

    /// Index of the best choice.
    pub fn choose(&self, g: &mut Game, choices: &[Choice]) -> usize {
        let scores = self.scores(g, choices);
        self.best(&scores, &mut g.rng)
    }

    /// The score of every choice of the event waiting, without the noise.
    pub(crate) fn scores(&self, g: &Game, choices: &[Choice]) -> Vec<Fx> {
        // The neighbour of the event, for a suit among the choices.
        let p = g.pending_event.as_ref();
        let nb = p.and_then(|p| match &p.target {
            Some(Target::Neighbour(n)) => Some(n),
            _ => p.neighbour.as_ref(),
        });
        let scores = choices
            .iter()
            .map(|c| self.worth(&c.effects, &g.world, &g.data, nb));
        scores.collect()
    }

    /// The best action of `available_actions` with a free slot of its kind (its cost counts
    /// as treasury spent), or None when doing nothing (score 0) wins.
    pub fn action(&self, g: &mut Game) -> Option<(ActionId, Option<Target>)> {
        let mut options = vec![None];
        let mut scores = vec![Fx(0)];
        let treasury = self.weight(&g.data.economy.treasury.0);
        let mut actions = g.available_actions();
        for (k, (id, targets)) in actions.iter().enumerate() {
            let a = g.data.actions.iter().find(|a| a.id == *id).expect("listed");
            if !g.data.action_slots.free(&g.world, &g.data.actions, a) {
                continue;
            }
            let (w, d) = (&g.world, &g.data);
            // A repeal only under pressure, and at the full price (`Data::auto_cost`).
            let cost = d.auto_cost(a);
            let pressed = |id: &String| d.law(id).is_some_and(|l| pressure(l, w, d) < Fx(0));
            match a.on_complete.first() {
                Some(Effect::RepealLaw(id)) if !pressed(id) => continue,
                _ if cost > w.axes[&d.economy.treasury] => continue,
                _ => {}
            }
            let years = Fx::from_int(a.duration_years.0.max(1) as i64);
            let score = |nb| {
                self.worth(&a.on_complete, w, d, nb) - treasury * cost
                    + self.worth(&a.yearly, w, d, None) * years
            };
            let first = score(None);
            for t in 0..targets.len().max(1) {
                // Only a suit weighs its court; the rest score alike on every target.
                let score = match targets.get(t) {
                    Some(Target::Neighbour(n)) if a.marries() => score(Some(n)),
                    _ => first,
                };
                options.push(Some((k, t)));
                scores.push(score);
            }
        }
        let (k, t) = options[self.best(&scores, &mut g.rng)]?;
        let (id, mut targets) = actions.swap_remove(k);
        Some((id, (t < targets.len()).then(|| targets.swap_remove(t))))
    }

    fn best(&self, scores: &[Fx], rng: &mut Rng) -> usize {
        let mut best: Option<(usize, Fx)> = None;
        for (i, s) in scores.iter().enumerate() {
            let s = *s + Fx(rng.range(0, self.noise.0 + 1));
            if best.is_none_or(|(_, b)| s > b) {
                best = Some((i, s));
            }
        }
        best.map_or(0, |(i, _)| i)
    }

    /// A law in force: its own weight (by its id), its anchor shifts by the weights of their
    /// axes, its yearly treasury by `income`, and `pressure` by `pressure`.
    fn law_worth(&self, l: &LawDef, w: &World, d: &Data) -> Fx {
        let anchors = l.anchors.iter();
        let anchors = anchors.fold(Fx(0), |s, (a, v)| s + self.weight(&a.0) * *v);
        self.weight(&l.id)
            + anchors
            + self.weight("income") * l.treasury
            + self.weight("pressure") * pressure(l, w, d)
    }

    fn weight(&self, key: &str) -> Fx {
        self.weights.get(key).copied().unwrap_or_default()
    }

    /// Keys: axis ids (by the delta), flag ids (+1 set, -1 cleared; a flag already so counts
    /// nothing), `province_income`,
    /// `province_population`, `province_loyalty`, `health`, `relation`, `crown_power`
    /// (by the delta), `build`, `grant`, `revoke`, `secede`, `war`, `hostage`, `death`,
    /// `abdicate` (+1 each), `overreach` (+1 for a grant while the crown holds more than its
    /// room, `crown_capacity`), `province` (+1 gained, -1 given away), `heir` (+1 born, -1 lost),
    /// `heir_ability`, `heir_claim` (by the delta), `bequeath` (+1 an heir named in a will), `army_upkeep` (by the change in the yearly
    /// upkeep a change of the army brings), `law` (+1 for a change of the laws in force, then
    /// each law brought in or ended by `law_worth`, a law brought in also by its resistance). A chance weighs both branches by its odds.
    pub(crate) fn worth(
        &self,
        effects: &[Effect],
        w: &World,
        data: &Data,
        nb: Option<&NeighbourId>,
    ) -> Fx {
        let one = Fx::from_int(1);
        let mut sum = Fx(0);
        for e in effects {
            let (key, v): (&str, Fx) = match e {
                Effect::Axis(a, d) if *a == data.war.army => {
                    // An army is paid for every year: `army_upkeep` by the change in upkeep.
                    let (upkeep, army) = (&data.war.army_upkeep, w.axes[a]);
                    let more =
                        crate::data::curve(upkeep, army + *d) - crate::data::curve(upkeep, army);
                    sum = sum + self.weight("army_upkeep") * more;
                    (&a.0, *d)
                }
                Effect::Axis(a, d) => (&a.0, *d),
                Effect::Tribute(v) => (&data.economy.treasury.0, *v),
                Effect::Province(_, ProvinceField::Income, d) => ("province_income", *d),
                Effect::Province(_, ProvinceField::Population, d) => ("province_population", *d),
                Effect::Province(_, ProvinceField::Loyalty, d) => ("province_loyalty", *d),
                // Only a change counts: a flag set again or an absent one cleared is nothing.
                Effect::SetFlag(f) if w.flags.contains(f) => continue,
                Effect::ClearFlag(f) if !w.flags.contains(f) => continue,
                Effect::SetFlag(f) => (f, one),
                Effect::ClearFlag(f) => (f, Fx::from_int(-1)),
                Effect::RulerHealth(d) => ("health", *d),
                Effect::Relation(_, d) | Effect::OtherRelations(d) => ("relation", *d),
                Effect::CrownPower(_, d) => ("crown_power", *d),
                Effect::Build(..) => ("build", one),
                Effect::Grant(_) => {
                    // A crown beyond its room is glad to give land away.
                    if !data.crown_capacity.over(w).is_empty() {
                        sum = sum + self.weight("overreach");
                    }
                    ("grant", one)
                }
                Effect::Revoke(_) => ("revoke", one),
                Effect::Secede(_) => ("secede", one),
                Effect::StartWar(_) => ("war", one),
                Effect::TakeHostage(..) => ("hostage", one),
                Effect::RulerDies(_) => ("death", one),
                Effect::Abdicate => ("abdicate", one),
                Effect::TransferProvince(_, NewHolder::Crown) => ("province", one),
                Effect::TransferProvince(_, NewHolder::Foreign(_)) => {
                    ("province", Fx::from_int(-1))
                }
                Effect::HeirOp(HeirOp::Add) => ("heir", one),
                Effect::HeirOp(HeirOp::TargetMarry) => ("marriage", one),
                Effect::HeirOp(HeirOp::TargetBequeath) => ("bequeath", one),
                Effect::HeirOp(HeirOp::Remove(_) | HeirOp::TargetRemove) => {
                    ("heir", Fx::from_int(-1))
                }
                Effect::HeirOp(HeirOp::Ability(_, d) | HeirOp::TargetAbility(d)) => {
                    ("heir_ability", *d)
                }
                Effect::HeirOp(HeirOp::Claim(_, d) | HeirOp::TargetClaim(d)) => ("heir_claim", *d),
                Effect::HeirOp(HeirOp::Designate(i)) => {
                    // Only the penalty of naming one not rightful weighs.
                    let named = w.heirs.get(*i as usize).map(|h| h.id);
                    let rightful = rightful(w, data).map(|r| w.heirs[r].id);
                    if named.is_some() && named != rightful && named != w.designated {
                        let p = &data.heirs.designate_penalty;
                        sum = p.iter().fold(sum, |s, (a, v)| s + self.weight(&a.0) * *v);
                    }
                    continue;
                }
                Effect::Chance(c) => {
                    let hit = c.percent(w) / Fx::from_int(100);
                    sum = sum + self.worth(&c.then, w, data, nb) * hit;
                    sum = sum + self.worth(&c.otherwise, w, data, nb) * (one - hit);
                    continue;
                }
                Effect::Marry { then, otherwise } => {
                    let chance = nb.map_or(Fx(0), |n| data.marriage.chance(w, data, n));
                    let yes = chance / Fx::from_int(100);
                    let wed = self.worth(then, w, data, nb) + self.weight("marriage");
                    sum = sum + wed * yes + self.worth(otherwise, w, data, nb) * (one - yes);
                    continue;
                }
                Effect::IfFriendly(es) => {
                    sum = sum + self.worth(es, w, data, nb);
                    continue;
                }
                Effect::EnactLaw(id) | Effect::RepealLaw(id) => {
                    let enact = matches!(e, Effect::EnactLaw(_));
                    let changes = law_changes(data, w, id, enact);
                    if !changes.is_empty() {
                        sum = sum + self.weight("law");
                    }
                    // Brought in against a faction: its resistance as a change of its axis.
                    let against = (data.law(id).filter(|_| enact && !changes.is_empty()))
                        .map_or(&[][..], |l| &l.resistance);
                    for (a, v) in against {
                        sum = sum + self.weight(&a.0) * *v;
                    }
                    for (l, k) in changes {
                        sum = sum + self.law_worth(l, w, data) * k;
                    }
                    continue;
                }
                // A bastard to recognize: `recognize`, and an `heir` when there is none.
                Effect::HeirOp(HeirOp::Recognize) if w.bastards.is_empty() => continue,
                Effect::HeirOp(HeirOp::Recognize) => {
                    if w.heirs.is_empty() {
                        sum = sum + self.weight("heir");
                    }
                    ("recognize", one)
                }
                Effect::HeirOp(HeirOp::SetStatus(..) | HeirOp::TargetStatus(_))
                | Effect::HeirOp(HeirOp::TargetDesignate)
                | Effect::SpawnEvent(..)
                | Effect::Clash
                | Effect::EndWar(_)
                | Effect::SetWarStage(_)
                | Effect::Mark(_)
                | Effect::Unmark(_) => continue,
            };
            sum = sum + self.weight(key) * v;
        }
        sum
    }
}
