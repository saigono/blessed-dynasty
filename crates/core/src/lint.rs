//! Content lint: references across data files and the format of texts. Each check returns
//! its remarks, empty when the data is clean; `crates/core/tests/data_lint.rs` holds every
//! one of them to zero on `data/`, the editor shows them next to the records.

use crate::data::Data;
use crate::rules::{Choice, Effect};
use crate::sim::FallReason;
use crate::state::World;
use std::collections::BTreeSet;

/// Every check below on the data and the world of its preset.
pub fn lint(data: &Data, world: &World) -> Vec<String> {
    [
        hints(data),
        spawned(data),
        named(data),
        told(data),
        templates(data),
        names(data, world),
        epithets(data),
        buildings(data),
        variants(data),
    ]
    .concat()
}

fn has_digits(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit())
}

/// Every cause_tag of events (simulation ones included) and actions has a hint, and every
/// hint has a tag. A hint is told as a sentence of its own (sim.rs adds the capital and the
/// full stop): lowercase start, no final punctuation, no numbers. The chain templates
/// (`chain:` keys, stage 20) are checked by `chain_templates_cover_the_graph`.
pub fn hints(data: &Data) -> Vec<String> {
    let hints = &data.hints;
    let events = data.events.iter().chain(&data.sim_events);
    let tags = events.flat_map(|e| &e.choices).map(|c| &c.cause_tag);
    // Stage 24: the testament is a decision of its own (Game::write_testament).
    let testament = crate::testament::TAG.to_string();
    let tags: BTreeSet<_> = tags
        .chain(data.actions.iter().map(|a| &a.cause_tag))
        .chain([&testament])
        .collect();
    let missing = (tags.iter()).filter(|t| !hints.contains_key(**t));
    let mut out: Vec<String> = missing.map(|t| format!("{t}: нет намёка")).collect();
    let unused = (hints.keys()).filter(|k| !k.starts_with("chain:") && !tags.contains(k));
    out.extend(unused.map(|k| format!("{k}: намёк без cause_tag")));
    let bad = hints.iter().filter(|(_, h)| {
        let first = h.chars().next().is_some_and(char::is_lowercase);
        !first || h.ends_with(['.', '!', '?', ',', ' ']) || has_digits(h)
    });
    out.extend(bad.map(|(k, h)| {
        format!("{k}: намёк «{h}» не в формате хроники (строчная буква, без точки и цифр)")
    }));
    out
}

/// Every `SpawnEvent` of a choice or an action names an event.
pub fn spawned(data: &Data) -> Vec<String> {
    let all = || data.events.iter().chain(&data.sim_events);
    let ids: BTreeSet<_> = all().map(|e| e.id.as_str()).collect();
    let choices = all().flat_map(|e| e.choices.iter().map(move |c| (&e.id, &c.effects)));
    let actions = data.actions.iter().map(|a| (&a.id, &a.on_complete));
    let effects = choices
        .chain(actions)
        .flat_map(|(id, es)| es.iter().map(move |e| (id, e)));
    effects
        .filter_map(|(at, e)| match e {
            Effect::SpawnEvent(id, _) if !ids.contains(id.as_str()) => {
                Some(format!("{at}: нет события {id}"))
            }
            _ => None,
        })
        .collect()
}

/// Events `rules.ron` names exist: the war start, neighbour AI events (simulation ones
/// included), death and abdication.
pub fn named(data: &Data) -> Vec<String> {
    let ai = &data.neighbour_ai;
    let stances = [&ai.expand, &ai.defend, &ai.trade, &ai.wait];
    let named = (stances.iter().flat_map(|s| &s.events)).map(|(id, _)| id);
    let death = data.death.risks.iter().map(|r| &r.event);
    let fixed = [
        &data.war.start_event,
        &data.death.event,
        &data.abdication.event,
    ];
    let all = data.events.iter().chain(&data.sim_events);
    let missing: BTreeSet<_> = (named.chain(death).chain(fixed))
        .filter(|id| !all.clone().any(|e| e.id == **id))
        .collect();
    let missing = missing.into_iter();
    missing
        .map(|id| format!("rules.ron: нет события {id}"))
        .collect()
}

/// Every reign choice the player sees carries a hint, and every choice (simulation ones too)
/// tells the chronicle what was done; both in words, without numbers.
pub fn told(data: &Data) -> Vec<String> {
    fn choices(es: &[crate::rules::Event]) -> impl Iterator<Item = (&String, &Choice)> + '_ {
        es.iter()
            .flat_map(|e| e.choices.iter().map(move |c| (&e.id, c)))
    }
    let unhinted =
        choices(&data.events).filter(|(_, c)| c.hint.as_ref().is_none_or(|h| has_digits(h)));
    let mut out: Vec<String> = unhinted
        .map(|(id, c)| format!("{id}: вариант «{}» без подсказки или с цифрами", c.text))
        .collect();
    let untold = (choices(&data.events).chain(choices(&data.sim_events))).filter(|(_, c)| {
        c.told.trim().is_empty() || has_digits(&c.told) || c.retold.iter().any(|t| has_digits(t))
    });
    out.extend(untold.map(|(id, c)| format!("{id}: вариант «{}» без told или с цифрами", c.text)));
    out
}

/// The `{…}` of a template that `text::fill` would leave as they are: an unknown key, an
/// unknown case, a sex choice without two forms.
pub fn bad_braces(s: &str) -> Vec<String> {
    // Stage 24 (testament.rs): founder, forebear, precept, order, will. Stage 25 (war):
    // war_won, war_lost.
    // Stage 26c: prev_event, then_event and their years (compound events), a and b (the
    // joins of `sim.texts.fuse`).
    const KEYS: [&str; 27] = [
        "prev_event",
        "prev_year",
        "then_event",
        "then_year",
        "a",
        "b",
        "founder",
        "forebear",
        "precept",
        "order",
        "will",
        "ruler",
        "prev",
        "heir",
        "province",
        "neighbour",
        "vassal",
        "house",
        "war_target",
        "war_won",
        "war_lost",
        "year",
        "years",
        "law",
        "lands",
        "deed",
        "epithet",
    ];
    let mut bad = Vec::new();
    for token in s
        .split('{')
        .skip(1)
        .filter_map(|t| t.split_once('}'))
        .map(|t| t.0)
    {
        let end = token.find(['.', ':']).unwrap_or(token.len());
        let (key, how) = token.split_at(end);
        let ok = KEYS.contains(&key)
            && match how.chars().next() {
                Some('.') => crate::text::CASES.contains(&&how[1..]),
                Some(_) => how[1..].split('|').count() == 2,
                None => true,
            };
        if !ok {
            bad.push(format!("{{{token}}} in «{s}»"));
        }
    }
    bad
}

/// Every template the game and the simulation fill: events, choices, `told`, the texts of
/// the simulation, the epithets and the life.
pub fn template_texts(data: &Data) -> Vec<&String> {
    let t = &data.sim.texts;
    let mut all: Vec<&String> = Vec::new();
    for e in data.events.iter().chain(&data.sim_events) {
        all.extend([&e.title, &e.text]);
        all.extend(e.texts.iter().chain(e.texts_when.iter().map(|(_, t)| t)));
        all.extend(e.choices.iter().flat_map(|c| [&c.text, &c.told]));
        all.extend(e.choices.iter().flat_map(|c| &c.retold));
    }
    for (a, b) in [
        &t.crowned,
        &t.province_lost,
        &t.province_gained,
        &t.heir_died,
        &t.partition,
        &t.law_changed,
        &t.law_enacted,
        &t.law_repealed,
    ] {
        all.extend([a, b]);
    }
    all.extend(t.reign_ends.values().chain(t.falls.iter().map(|f| &f.1)));
    all.extend(t.variants.values().flatten());
    all.extend(t.fall_told.iter().flat_map(|f| &f.1));
    for e in &t.epithets {
        all.extend(e.told.iter().chain([&e.name.0, &e.name.1]));
        all.extend(e.also.iter().flat_map(|(a, b)| [a, b]));
    }
    for r in &data.sim.traits {
        all.extend(
            std::iter::once(&r.told)
                .chain(&r.retold)
                .flat_map(|(a, b)| [a, b]),
        );
    }
    let f = &t.fuse;
    all.extend(f.same_year.iter().chain(&f.next_year));
    all.extend(
        f.pairs
            .iter()
            .flat_map(|p| p.joins.iter().chain([&p.title])),
    );
    all.extend(testament_texts(data).into_iter().flat_map(|(_, v)| v));
    let l = &t.life;
    for v in [
        &l.founder,
        &l.regency,
        &l.designated,
        &l.contested,
        &l.lawful,
        &l.deed,
        &l.same_year,
        &l.soon,
        &l.later,
    ] {
        all.extend(v);
    }
    all.extend(
        l.ends
            .values()
            .flatten()
            .chain(l.falls.iter().flat_map(|f| &f.1)),
    );
    all
}

/// The texts of the testament (stage 24) by their place in `rules.ron`, each 2-3 variants.
fn testament_texts(data: &Data) -> Vec<(String, Vec<&String>)> {
    let Some(r) = &data.testament else {
        return vec![];
    };
    let tx = &r.texts;
    let mut all: Vec<(String, Vec<&String>)> = [
        ("read", &tx.read.1),
        ("precept", &tx.precept),
        ("order", &tx.order),
        ("heir", &tx.heir),
        ("faithful", &tx.faithful),
        ("crowned", &tx.crowned),
        ("willed", &tx.willed),
        ("life_founder", &tx.life_founder),
        ("life_kept", &tx.life_kept),
        ("life_broken", &tx.life_broken),
    ]
    .into_iter()
    .map(|(k, v)| (format!("testament.texts.{k}"), v.iter().collect()))
    .collect();
    let o = &r.orders;
    for (k, rule) in [
        ("keep_law", &o.keep_law),
        ("keep_province", &o.keep_province),
        ("peace", &o.peace),
    ] {
        all.push((
            format!("testament.orders.{k}.breach"),
            rule.breach.iter().collect(),
        ));
        all.push((
            format!("testament.orders.{k}.what"),
            vec![&rule.what, &rule.what],
        ));
    }
    for (k, v) in [("rumour", &tx.rumour), ("sealed", &tx.sealed)] {
        all.push((format!("testament.texts.{k}"), vec![v, v]));
    }
    all
}

/// `bad_braces` of every `template_texts`, a life without its parts, a text of the
/// testament without 2-3 variants.
pub fn templates(data: &Data) -> Vec<String> {
    let l = &data.sim.texts.life;
    let parts = [
        ("founder", &l.founder),
        ("regency", &l.regency),
        ("designated", &l.designated),
        ("contested", &l.contested),
        ("lawful", &l.lawful),
        ("deed", &l.deed),
        ("same_year", &l.same_year),
    ];
    let empty = parts.iter().filter(|(_, v)| v.is_empty());
    let mut out: Vec<String> = empty
        .map(|(k, _)| format!("sim.texts.life.{k}: пусто"))
        .collect();
    let few = testament_texts(data)
        .into_iter()
        .filter(|(_, v)| !(2..=3).contains(&v.len()));
    out.extend(few.map(|(k, _)| format!("{k}: нужно 2–3 варианта")));
    out.extend(template_texts(data).into_iter().flat_map(|s| bad_braces(s)));
    out
}

/// Every name a text may decline has its six cases: the pools, the lands of the map, the
/// houses and neighbours of the preset, the epithets.
pub fn names(data: &Data, world: &World) -> Vec<String> {
    let n = &data.names;
    let mut all: Vec<&str> = [&n.rulers, &n.heirs, &n.daughters, &n.vassals]
        .into_iter()
        .flatten()
        .map(|s| s.as_str())
        .collect();
    all.extend(world.provinces.values().map(|p| p.name.as_str()));
    all.extend(world.vassals.values().map(|v| v.name.as_str()));
    all.extend(world.neighbours.values().map(|v| v.name.as_str()));
    let realms = world.neighbours.values().filter_map(|n| n.realm.as_ref());
    all.extend(realms.flat_map(|r| [r.house.as_str(), r.ruler.as_str()]));
    let epithets = data.sim.texts.epithets.iter();
    let names = epithets.flat_map(|e| std::iter::once(&e.name).chain(&e.also));
    all.extend(names.flat_map(|(a, b)| [a.as_str(), b.as_str()]));
    let undeclined = n.undeclined(all).into_iter();
    undeclined
        .map(|s| format!("{s}: нет падежей (names.ron forms)"))
        .collect()
}

/// Every `Build` of an event or an action has its icon in `Data.buildings` (stage 26b): the
/// map would not show it.
pub fn buildings(data: &Data) -> Vec<String> {
    let events = data.events.iter().chain(&data.sim_events);
    let choices = events.flat_map(|e| e.choices.iter().map(move |c| (&e.id, &c.effects)));
    let actions = data.actions.iter().map(|a| (&a.id, &a.on_complete));
    let effects = choices
        .chain(actions)
        .flat_map(|(id, es)| es.iter().map(move |e| (id, e)));
    effects
        .filter_map(|(at, e)| match e {
            Effect::Build(_, b) if !data.buildings.iter().any(|d| d.id == *b) => {
                Some(format!("{at}: постройки {b} нет в rules.ron buildings"))
            }
            _ => None,
        })
        .collect()
}

/// The deeds an epithet counts are ones a reign counts, an epithet always holds, every way
/// a reign ends has its phrase in a life, and every fall its words (a life tells NoHeir as
/// the death before it).
pub fn epithets(data: &Data) -> Vec<String> {
    let t = &data.sim.texts;
    let mut known: BTreeSet<String> = ["law", "province_gained", "province_lost", "heir_died"]
        .map(String::from)
        .into();
    let choices = (data.events.iter().chain(&data.sim_events)).flat_map(|e| &e.choices);
    known.extend(choices.map(|c| c.cause_tag.clone()));
    known.extend(data.actions.iter().map(|a| a.cause_tag.clone()));
    known.extend(data.sim.traits.iter().map(|r| format!("trait:{}", r.id)));
    let unknown = (t.epithets.iter()).flat_map(|e| e.deeds.iter().map(move |d| (&e.name.0, d)));
    let unknown = unknown.filter(|(_, d)| !known.contains(*d));
    let mut out: Vec<String> = unknown
        .map(|(e, d)| format!("{e}: неизвестное деяние {d}"))
        .collect();
    if !t.epithets.iter().any(|e| e.min == 0) {
        out.push("sim.texts.epithets: нет прозвища с min 0".into());
    }
    let untold = (t.reign_ends.keys()).filter(|k| !t.life.ends.contains_key(*k));
    out.extend(untold.map(|k| format!("sim.texts.life.ends: нет {k}")));
    use FallReason::*;
    for f in [NoHeir, Conquered, Usurped, NoCrownLand, Alive] {
        if !t.fall_told.iter().any(|(r, _)| *r == f) {
            out.push(format!("sim.texts.fall_told: нет {f:?}"));
        }
        if f != NoHeir && !t.life.falls.iter().any(|(r, _)| *r == f) {
            out.push(format!("sim.texts.life.falls: нет {f:?}"));
        }
    }
    out
}

/// Stage 26c: texts enough for a repeat to read anew. An event says itself 3 ways without
/// conditions, a choice is told 3 ways, a record of the simulation, an epilogue and every
/// phrase of a life (the epithet's included) 5 ways, an epithet has 2 names.
pub fn variants(data: &Data) -> Vec<String> {
    let mut all: Vec<(String, usize, usize)> = vec![];
    for e in data.events.iter().chain(&data.sim_events) {
        all.push((format!("{}: текст", e.id), 1 + e.texts.len(), 3));
        for c in &e.choices {
            all.push((
                format!("{}: told «{}»", e.id, c.text),
                1 + c.retold.len(),
                3,
            ));
        }
    }
    let t = &data.sim.texts;
    let more = |k: &str| t.variants.get(k).map_or(0, Vec::len);
    for k in [
        "crowned",
        "province_lost",
        "province_gained",
        "heir_died",
        "law_changed",
        "law_enacted",
        "law_repealed",
        "partition",
    ] {
        all.push((format!("sim.texts.{k}"), 1 + more(k), 5));
    }
    let falls = t
        .fall_told
        .iter()
        .map(|(f, v)| (format!("fall_told {f:?}"), v.len()));
    let l = &t.life;
    let parts = [
        ("founder", &l.founder),
        ("regency", &l.regency),
        ("designated", &l.designated),
        ("contested", &l.contested),
        ("lawful", &l.lawful),
        ("deed", &l.deed),
        ("same_year", &l.same_year),
        ("soon", &l.soon),
        ("later", &l.later),
    ];
    let parts = parts
        .into_iter()
        .map(|(k, v)| (format!("life.{k}"), v.len()));
    let ends = l
        .ends
        .iter()
        .map(|(k, v)| (format!("life.ends.{k}"), v.len()));
    let lf = l
        .falls
        .iter()
        .map(|(f, v)| (format!("life.falls {f:?}"), v.len()));
    let told = t
        .epithets
        .iter()
        .map(|e| (format!("epithet {}", e.name.0), e.told.len()));
    let five = falls.chain(parts).chain(ends).chain(lf).chain(told);
    all.extend(five.map(|(k, n)| (format!("sim.texts.{k}"), n, 5)));
    let names = t
        .epithets
        .iter()
        .map(|e| (format!("{}: имена", e.name.0), 1 + e.also.len(), 2));
    all.extend(names);
    (all.into_iter())
        .filter(|(_, n, min)| n < min)
        .map(|(k, n, min)| format!("{k}: вариантов {n}, нужно не меньше {min}"))
        .collect()
}
