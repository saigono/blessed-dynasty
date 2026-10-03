//! The founder's testament (stage 24): a precept that weighs the choices of his dynasty, an
//! order it should keep, an heir named under seal. Its strength is the founder's legend at
//! his death, fading with the years, times the zeal of the ruler who reads it. Every kind of
//! precept, every number and text lives in `rules.ron` `testament`.

use crate::data::{Data, OrderRule, TestamentRules, curve};
use crate::fx::Fx;
use crate::game::Game;
use crate::rules::{Action, Choice};
use crate::sim::AutoChooser;
use crate::state::{AxisId, Holder, NeighbourId, ProvinceId, Sex, World};
use crate::text::{self, Named};
use crate::time::Tick;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The cause tag of the decision to write one (`Game::write_testament`).
pub const TAG: &str = "testament";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Testament {
    /// An id of `testament.precepts`.
    #[serde(default)]
    pub precept: Option<String>,
    #[serde(default)]
    pub order: Option<Order>,
    /// The heir named under seal (`Heir.id`): crowned at the next coronation over the law,
    /// then gone. The simulation's rulers name one too (`HeirOp::TargetBequeath`).
    #[serde(default)]
    pub heir: Option<u32>,
    /// Who wrote it and his sex, for the texts; `Game::write_testament` sets it.
    #[serde(default)]
    pub by: (String, Sex),
    /// The founder's legend, set at his death by the simulation (`legend`).
    #[serde(default)]
    pub legend: Fx,
    /// The founder's death; None while he lives.
    #[serde(default)]
    pub since: Option<Tick>,
    /// When the order was broken: it binds no more.
    #[serde(default)]
    pub broken: Option<Tick>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Order {
    /// Never repeal this law of `Data.laws`, nor replace it with another of its group.
    KeepLaw(String),
    /// Never let this province go to a foreign state.
    KeepProvince(ProvinceId),
    /// Never be at war with this neighbour.
    Peace(NeighbourId),
}

impl TestamentRules {
    pub fn precept(&self, id: &str) -> Option<&crate::data::Precept> {
        self.precepts.iter().find(|p| p.id == id)
    }

    pub fn order(&self, o: &Order) -> &OrderRule {
        match o {
            Order::KeepLaw(_) => &self.orders.keep_law,
            Order::KeepProvince(_) => &self.orders.keep_province,
            Order::Peace(_) => &self.orders.peace,
        }
    }
}

/// A testament that names something, all of it known: a precept of the data, a law of
/// `Data.laws`, a province of the realm, a neighbour, an heir in the line.
pub fn valid(d: &Data, w: &World, t: &Testament) -> bool {
    let Some(r) = &d.testament else {
        return false;
    };
    let precept = (t.precept.as_ref()).is_none_or(|p| r.precept(p).is_some());
    let order = match &t.order {
        None => true,
        Some(Order::KeepLaw(l)) => d.law(l).is_some(),
        Some(Order::KeepProvince(p)) => {
            (w.provinces.get(p)).is_some_and(|p| !matches!(p.holder, Holder::Foreign(_)))
        }
        Some(Order::Peace(n)) => w.neighbours.contains_key(n),
    };
    let heir = t.heir.is_none_or(|id| w.heir_index(id).is_some());
    let some = t.precept.is_some() || t.order.is_some() || t.heir.is_some();
    precept && order && heir && some
}

/// What writing one costs now: rumours that a ruler young and hale prepares for death,
/// `rumours.per_year` for every year he is younger than `rumours.age`; nothing in old age
/// or with health below `rumours.health`.
pub fn cost(d: &Data, w: &World) -> Vec<(AxisId, Fx)> {
    let Some(r) = d.testament.as_ref().map(|t| &t.rumours) else {
        return vec![];
    };
    let years = r.age.saturating_sub(w.ruler.age);
    if years == 0 || w.ruler.health < r.health {
        return vec![];
    }
    let years = Fx::from_int(years as i64);
    (r.per_year.iter())
        .map(|(a, k)| (a.clone(), *k * years))
        .collect()
}

/// The founder's legend from the world at his death (`testament.legend`): `base`, the axes,
/// the years of his reign, his deeds (`sim::founder_deeds`: the tags of his decisions,
/// `law` for every law he brought in), clamped to `0..=max`.
pub fn legend(d: &Data, w: &World) -> Fx {
    let Some(l) = d.testament.as_ref().map(|t| &t.legend) else {
        return Fx(0);
    };
    let deeds = crate::sim::founder_deeds(w);
    let count = |tags: &[String]| {
        tags.iter()
            .map(|t| deeds.get(t).copied().unwrap_or(0))
            .sum::<u32>()
    };
    let years = (w.tick.0 - w.ruler.reign_start.0) / w.time_unit.ticks_per_year;
    let sum = (l.axes.iter()).fold(l.base, |s, (a, k)| s + w.axes[a] * *k);
    let sum = (l.deeds.iter()).fold(sum, |s, (tags, k)| {
        s + *k * Fx::from_int(count(tags) as i64)
    });
    (sum + l.years * Fx::from_int(years as i64)).clamp(Fx(0), l.max)
}

/// The parts of the strength now: the legend, the share left after the years since the
/// founder's death (`decay`), the zeal of the ruler (`zeal`: by his legitimacy, plus his
/// traits, at least 0). None before the founder's death.
pub fn parts(d: &Data, w: &World) -> Option<(Fx, Fx, Fx)> {
    let r = d.testament.as_ref()?;
    let t = w.testament.as_ref()?;
    let since = t.since?;
    let years = (w.tick.0.saturating_sub(since.0)) / w.time_unit.ticks_per_year;
    let decay = curve(&r.decay, Fx::from_int(years as i64));
    let traits = (r.zeal.traits.iter()).filter(|(k, _)| w.ruler.traits.contains(*k));
    let zeal = traits.fold(curve(&r.zeal.curve, w.axes[&r.zeal.axis]), |s, (_, v)| {
        s + *v
    });
    Some((t.legend, decay, zeal.max(Fx(0))))
}

/// Legend × decay × zeal; 0 before the founder's death.
pub fn strength(d: &Data, w: &World) -> Fx {
    parts(d, w).map_or(Fx(0), |(l, k, z)| l * k * z)
}

/// The weights the testament adds to the automaton, times `k`: the precept's, and the
/// order's `weight` on the law's id, on `province` (land gained or given) or on `war`; the
/// order no more once broken.
pub fn weights(d: &Data, w: &World, k: Fx) -> BTreeMap<String, Fx> {
    let mut out = BTreeMap::new();
    let (Some(r), Some(t)) = (&d.testament, &w.testament) else {
        return out;
    };
    let precept = t.precept.as_ref().and_then(|p| r.precept(p));
    let order = t.order.as_ref().filter(|_| t.broken.is_none()).map(|o| {
        let key = match o {
            Order::KeepLaw(l) => l.as_str(),
            Order::KeepProvince(_) => "province",
            Order::Peace(_) => "war",
        };
        (key.to_string(), r.order(o).weight)
    });
    for (key, v) in precept
        .into_iter()
        .flat_map(|p| p.weights.clone())
        .chain(order)
    {
        let e = out.entry(key).or_insert(Fx(0));
        *e = *e + v * k;
    }
    out
}

/// `base` with the testament's weights at its strength now: the ruler who reads it.
pub fn chooser(base: &AutoChooser, d: &Data, w: &World) -> AutoChooser {
    let mut auto = base.clone();
    for (key, v) in weights(d, w, strength(d, w)) {
        let e = auto.weights.entry(key).or_insert(Fx(0));
        *e = *e + v;
    }
    auto
}

/// The testament's own chooser, at full strength and without noise.
fn own(d: &Data, w: &World) -> AutoChooser {
    AutoChooser {
        weights: weights(d, w, Fx::from_int(1)),
        noise: Fx(0),
    }
}

/// Choice `idx` is true to the testament and made for it: the testament's own chooser
/// scores it highest, at least `faithful.margin` above the lowest, and the ruler's `base`
/// would have scored another higher. Only after the founder's death.
pub fn faithful(g: &Game, choices: &[Choice], idx: usize, base: &AutoChooser) -> bool {
    let (Some(r), Some(_)) = (&g.data.testament, parts(&g.data, &g.world)) else {
        return false;
    };
    let scores = own(&g.data, &g.world).scores(g, choices);
    let (max, min) = (scores.iter().max(), scores.iter().min());
    let (Some(&max), Some(&min)) = (max, min) else {
        return false;
    };
    let swayed = base.scores(g, choices);
    let swayed = swayed.iter().any(|s| *s > swayed[idx]);
    scores[idx] == max && max - min >= r.faithful.margin && swayed
}

/// An action started is true to the testament and done for it: its own chooser scores what
/// it brings at least `faithful.margin`, the ruler's `base` not above its price.
pub fn faithful_action(g: &Game, a: &Action, base: &AutoChooser) -> bool {
    let (Some(r), Some(_)) = (&g.data.testament, parts(&g.data, &g.world)) else {
        return false;
    };
    let (w, d) = (&g.world, &g.data);
    let treasury = base
        .weights
        .get(&d.economy.treasury.0)
        .copied()
        .unwrap_or_default();
    let worth = base.worth(&a.on_complete, w, d, None) - treasury * d.auto_cost(a);
    own(d, w).worth(&a.on_complete, w, d, None) >= r.faithful.margin && worth <= Fx(0)
}

/// The order was broken since the world stood at `laws` (the laws then in force),
/// `holders` (the holders of the provinces in order) and `war` (the enemy then, or the
/// neighbour who attacked since: a war he starts breaks no Peace order).
pub fn broken(w: &World, laws: &[String], holders: &[Holder], war: Option<&NeighbourId>) -> bool {
    let t = w.testament.as_ref();
    let Some(t) = t.filter(|t| t.since.is_some() && t.broken.is_none()) else {
        return false;
    };
    let foreign = |h: &Holder| matches!(h, Holder::Foreign(_));
    match &t.order {
        Some(Order::KeepLaw(l)) => laws.contains(l) && !w.flags.contains(l),
        Some(Order::KeepProvince(p)) => {
            let i = w.provinces.keys().position(|k| k == p);
            let was = i.and_then(|i| holders.get(i)).is_some_and(|h| !foreign(h));
            was && w.provinces.get(p).is_some_and(|p| foreign(&p.holder))
        }
        Some(Order::Peace(n)) => war != Some(n) && w.war.as_ref().is_some_and(|x| x.enemy == *n),
        None => false,
    }
}

/// The founder as his heirs call him (`texts.forebears`, in the genitive): of the ruler's
/// generation in the family tree from him, the last word for all after.
pub fn forebear<'a>(d: &'a Data, w: &World) -> &'a str {
    let (Some(r), Some(t)) = (&d.testament, &w.testament) else {
        return "";
    };
    let mut at = w.kin.iter().rposition(|k| k.crowned.is_some());
    let mut generation = 0;
    while let Some(parent) = at.and_then(|i| w.kin[i].parent) {
        (at, generation) = (Some(parent), generation + 1);
    }
    let f = &r.texts.forebears;
    let Some((male, female)) = f.get(generation.max(1) - 1).or(f.last()) else {
        return "";
    };
    if t.by.1 == Sex::Female { female } else { male }
}

/// `s` with the founder, the ruler, the forebear, the precept and the order's law, land or
/// neighbour filled in.
pub fn fill(d: &Data, w: &World, s: &str) -> String {
    let Some(t) = &w.testament else {
        return s.to_string();
    };
    let r = d.testament.as_ref();
    let precept = (t.precept.as_ref()).and_then(|p| r?.precept(p));
    let mut named: Vec<Named> = vec![
        ("founder", &t.by.0, Some(t.by.1)),
        ("ruler", &w.ruler.name, Some(w.ruler.sex)),
        ("forebear", forebear(d, w), None),
        ("precept", precept.map_or("", |p| &p.name), None),
    ];
    match &t.order {
        Some(Order::KeepLaw(l)) => named.push(("law", d.law(l).map_or(l, |l| &l.name), None)),
        Some(Order::KeepProvince(p)) => named.extend(
            w.provinces
                .get(p)
                .map(|p| ("province", p.name.as_str(), None)),
        ),
        Some(Order::Peace(n)) => named.extend(
            w.neighbours
                .get(n)
                .map(|n| ("neighbour", n.name.as_str(), None)),
        ),
        None => {}
    }
    text::fill(s, &d.names, &named)
}

/// The order in words (`OrderRule.what`): «не отменять закон «Городские вольности»».
pub fn order_text(d: &Data, w: &World) -> String {
    let order = w.testament.as_ref().and_then(|t| t.order.as_ref());
    match (&d.testament, order) {
        (Some(r), Some(o)) => fill(d, w, &r.order(o).what),
        _ => String::new(),
    }
}

/// The founder's testament read at his death, `(title, text)`: its first sentence, then the
/// precept, the order, the heir named (`{heir}`), each a variant of its own by `n`.
pub fn read(d: &Data, w: &World, salt: u64, n: u64) -> Option<(String, String)> {
    let (r, t) = (d.testament.as_ref()?, w.testament.as_ref()?);
    let tx = &r.texts;
    let heir = t.heir.and_then(|id| w.heir_index(id)).map(|i| &w.heirs[i]);
    let order = order_text(d, w);
    let parts = [
        (true, &tx.read.1),
        (t.precept.is_some(), &tx.precept),
        (t.order.is_some(), &tx.order),
        (heir.is_some(), &tx.heir),
    ];
    let said: Vec<String> = (parts.iter().enumerate())
        .filter(|(_, (some, _))| *some)
        .map(|(k, (_, v))| {
            let s = fill(d, w, text::pick(v, salt, n + k as u64)).replace("{order}", &order);
            let named = heir.map(|h| ("heir", h.name.as_str(), Some(h.sex)));
            text::fill(&s, &d.names, &Vec::from_iter(named))
        })
        .collect();
    Some((tx.read.0.clone(), said.join(" ")))
}

/// A sentence after the entry of a choice true to the testament (`texts.faithful`).
pub fn faithful_text(d: &Data, w: &World, salt: u64, n: u64) -> String {
    let v = d.testament.as_ref().map_or(&[][..], |r| &r.texts.faithful);
    fill(d, w, text::pick(v, salt, n))
}

/// The entry of a broken order, `(title, text)` (`OrderRule.breach`).
pub fn breach_text(d: &Data, w: &World, salt: u64, n: u64) -> Option<(String, String)> {
    let r = d.testament.as_ref()?;
    let o = w.testament.as_ref()?.order.as_ref()?;
    let told = fill(d, w, text::pick(&r.order(o).breach, salt, n));
    Some((r.texts.breach.clone(), told))
}

/// The testament in a life (`texts.life_founder`, `life_kept`, `life_broken`): of the
/// founder who left one (`{will}`: the precept and the order), of a ruler true to it or who
/// broke it (by his deeds `testament_kept`, `testament_broken`); empty otherwise.
pub fn life(d: &Data, w: &World, deeds: &BTreeMap<String, u32>, salt: u64, n: u64) -> String {
    let (Some(r), Some(t)) = (&d.testament, &w.testament) else {
        return String::new();
    };
    let tx = &r.texts;
    let has = |k: &str| deeds.get(k).is_some_and(|n| *n > 0);
    let v = match () {
        _ if t.since.is_none() && (t.precept.is_some() || t.order.is_some()) => &tx.life_founder,
        _ if has("testament_broken") => &tx.life_broken,
        _ if has("testament_kept") && t.since.is_some() => &tx.life_kept,
        _ => return String::new(),
    };
    let precept = (t.precept.as_ref()).and_then(|p| r.precept(p));
    let will: Vec<String> = (precept.map(|p| format!("«{}»", p.name)).into_iter())
        .chain(t.order.is_some().then(|| order_text(d, w)))
        .collect();
    fill(d, w, text::pick(v, salt, n)).replace("{will}", &will.join(", а также "))
}
