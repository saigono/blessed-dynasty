//! A game as a URL fragment `#p=...`: the preset id, the seed and the decisions in a compact
//! binary form, base64url. Opening it plays the decisions again from the seed.
//!
//! Bytes: `VERSION`, the preset id, the seed, the tick the game was at, then per decision
//! the ticks since the previous one and a step: 0 and the choice index, 1 and the action
//! index in `Data.actions` with its target, 2 for abdication. Numbers are LEB128, strings
//! are a length and UTF-8. Event ids, cause tags and choice targets are not stored: the
//! replay finds them again.

use crate::game::{Decision, DecisionKind, Game};
use crate::rules::Target;
use crate::state::{NeighbourId, ProvinceId};
use crate::time::Tick;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

/// The first byte. A link of another version is refused rather than misread.
pub const VERSION: u8 = 1;

const CHOICE: u8 = 0;
const ACTION: u8 = 1;
const ABDICATE: u8 = 2;

/// A decoded link; `play` puts its decisions on a new game of `preset_id` and `seed`.
#[derive(Debug, PartialEq)]
pub struct Link {
    pub preset_id: String,
    pub seed: u64,
    end: Tick,
    steps: Vec<u8>,
}

/// `g` was started from `preset_id` and `seed`; the link ends at its current tick.
pub fn encode(preset_id: &str, seed: u64, g: &Game) -> String {
    let mut out = vec![VERSION];
    put_str(&mut out, preset_id);
    put(&mut out, seed);
    put(&mut out, g.world.tick.0 as u64);
    let mut tick = 0;
    for d in &g.decisions {
        put(&mut out, (d.tick.0 - tick) as u64);
        tick = d.tick.0;
        match &d.kind {
            DecisionKind::EventChoice { choice_idx, .. } => {
                out.push(CHOICE);
                put(&mut out, *choice_idx as u64);
            }
            DecisionKind::ActionStarted { action_id, target } => {
                out.push(ACTION);
                let idx = g.data.actions.iter().position(|a| a.id == *action_id);
                put(&mut out, idx.expect("started actions exist") as u64);
                match target {
                    None => out.push(0),
                    Some(Target::Province(p)) => {
                        out.push(1);
                        put_str(&mut out, &p.0);
                    }
                    Some(Target::Neighbour(n)) => {
                        out.push(2);
                        put_str(&mut out, &n.0);
                    }
                    Some(Target::Heir(i)) => {
                        out.push(3);
                        put(&mut out, *i as u64);
                    }
                }
            }
            DecisionKind::Abdicate => out.push(ABDICATE),
        }
    }
    B64.encode(out)
}

pub fn decode(text: &str) -> Result<Link, String> {
    let bytes = B64
        .decode(text.trim())
        .map_err(|e| format!("base64: {e}"))?;
    let b = &mut &bytes[..];
    let version = byte(b)?;
    if version != VERSION {
        return Err(format!("версия ссылки {version}, нужна {VERSION}"));
    }
    Ok(Link {
        preset_id: string(b)?,
        seed: num(b)?,
        end: tick(b)?,
        steps: b.to_vec(),
    })
}

impl Link {
    /// Plays the decisions on `g`, a new game of the link's preset and seed, and waits up to
    /// the tick the link ends at.
    pub fn play(&self, g: &mut Game) -> Result<(), String> {
        let b = &mut &self.steps[..];
        let mut at = 0u32;
        while !b.is_empty() {
            at = at.checked_add(tick(b)?.0).ok_or("тик вне диапазона")?;
            advance(g, Tick(at))?;
            let res = match byte(b)? {
                CHOICE => g.choose(num(b)? as usize),
                ACTION => {
                    let idx = num(b)? as usize;
                    let a = g
                        .data
                        .actions
                        .get(idx)
                        .ok_or(format!("нет действия {idx}"))?;
                    let id = a.id.clone();
                    let target = match byte(b)? {
                        0 => None,
                        1 => Some(Target::Province(ProvinceId(string(b)?))),
                        2 => Some(Target::Neighbour(NeighbourId(string(b)?))),
                        3 => Some(Target::Heir(num(b)? as u32)),
                        x => return Err(format!("неизвестная цель {x}")),
                    };
                    g.start_action(&id, target)
                }
                ABDICATE => g.abdicate(),
                x => return Err(format!("неизвестный шаг {x}")),
            };
            res.map_err(|e| format!("ссылка не совпадает с игрой на тике {at}: {e:?}"))?;
        }
        advance(g, self.end)
    }
}

/// Repeats the decisions at their ticks, then waits until `end`.
pub fn replay(g: &mut Game, decisions: &[Decision], end: Tick) -> Result<(), String> {
    for d in decisions {
        advance(g, d.tick)?;
        let res = match &d.kind {
            DecisionKind::ActionStarted { action_id, target } => g
                .start_action(action_id, target.clone())
                .map_err(|e| format!("{e:?}")),
            DecisionKind::Abdicate => g.abdicate().map_err(|e| format!("{e:?}")),
            DecisionKind::EventChoice {
                event_id,
                choice_idx,
                ..
            } => match &g.pending_event {
                Some(p) if p.event_id == *event_id => {
                    g.choose(*choice_idx).map_err(|e| format!("{e:?}"))
                }
                p => Err(format!("ждали событие {event_id}, а есть {p:?}")),
            },
        };
        res.map_err(|e| format!("журнал не совпадает на тике {}: {e}", d.tick.0))?;
    }
    advance(g, end)
}

/// Waits up to `tick`. An event before it means the journal skipped a choice.
pub fn advance(g: &mut Game, tick: Tick) -> Result<(), String> {
    while g.world.tick < tick {
        if let Some(p) = &g.pending_event {
            let (id, now) = (&p.event_id, g.world.tick.0);
            return Err(format!(
                "журнал не совпадает: событие {id} на тике {now} без выбора"
            ));
        }
        g.wait().map_err(|e| format!("{e:?}"))?;
    }
    Ok(())
}

fn put(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn byte(b: &mut &[u8]) -> Result<u8, String> {
    let (&x, rest) = b.split_first().ok_or("ссылка обрезана")?;
    *b = rest;
    Ok(x)
}

fn num(b: &mut &[u8]) -> Result<u64, String> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let x = byte(b)?;
        v |= ((x & 0x7f) as u64) << shift;
        if x < 0x80 {
            return Ok(v);
        }
    }
    Err("число длиннее 64 бит".into())
}

fn tick(b: &mut &[u8]) -> Result<Tick, String> {
    let v = u32::try_from(num(b)?).map_err(|_| "тик вне диапазона")?;
    Ok(Tick(v))
}

fn string(b: &mut &[u8]) -> Result<String, String> {
    let n = num(b)? as usize;
    if n > b.len() {
        return Err("ссылка обрезана".into());
    }
    let (s, rest) = b.split_at(n);
    *b = rest;
    String::from_utf8(s.to_vec()).map_err(|_| "строка не UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Step;
    use crate::state::Preset;

    const RULES: &str = include_str!("../../../data/rules.ron");
    const PRESET: &str = include_str!("../../../data/presets/default.ron");
    const MAP: &str = include_str!("../../../data/maps/default.ron");

    /// The real actions, one more on an heir, the death events (abdication among them), and an event "e" with
    /// three choices that, with no quiet weight, fires on most ticks.
    fn game(seed: u64) -> Game {
        let mut data = crate::data::load(RULES).unwrap();
        data.quiet_weight = 0;
        data.add_events(include_str!("../../../data/events/death.ron"))
            .unwrap();
        let choice = |t: &str| format!("(text: \"{t}\", effects: [], cause_tag: \"{t}\")");
        let choices = ["a", "b", "c"].map(choice).join(", ");
        data.add_events(&format!(
            "[(id: \"e\", title: \"\", text: \"\", when: All([]), weight: 1, once: false, \
             cooldown_years: 0, importance: 1, target: None, choices: [{choices}])]"
        ))
        .unwrap();
        data.add_actions(include_str!("../../../data/actions.ron"))
            .unwrap();
        // No action of the data targets an heir yet.
        data.add_actions(
            "[(id: \"tutor\", name: \"\", duration_years: 1, cost: 0, requires: All([]), \
             min_crown_power: 0, target: Heir, on_complete: [], cause_tag: \"tutor\")]",
        )
        .unwrap();
        let preset = Preset::load_with_map(PRESET, MAP, &data).unwrap();
        Game::new(data, &preset, seed)
    }

    /// Up to `n` decisions: an action with some target every third tick, the choices in
    /// turn. Stops early if the ruler dies.
    fn play(seed: u64, n: usize) -> Game {
        let mut g = game(seed);
        let mut i = 0;
        while g.decisions.len() < n && g.ended.is_none() {
            let actions = g.available_actions();
            if i % 3 == 0 && !actions.is_empty() {
                let (id, targets) = &actions[i / 3 % actions.len()];
                let _ = g.start_action(id, targets.get(i % targets.len().max(1)).cloned());
            }
            if let Step::Event(v) = g.wait().unwrap() {
                g.choose(i % v.choices.len()).unwrap();
            }
            i += 1;
        }
        g
    }

    fn reopen(g: &Game, seed: u64) -> Game {
        let link = decode(&encode("default", seed, g)).unwrap();
        assert_eq!((link.preset_id.as_str(), link.seed), ("default", seed));
        let mut r = game(seed);
        link.play(&mut r).unwrap();
        r
    }

    #[test]
    fn a_journal_of_80_decisions_comes_back_the_same() {
        let (seed, g) = (1..)
            .map(|s| (s, play(s, 80)))
            .find(|(_, g)| g.ended.is_none())
            .unwrap();
        assert_eq!(g.decisions.len(), 80);
        // Every kind of step and target is in the journal.
        let shape = |d: &Decision| match &d.kind {
            DecisionKind::ActionStarted { target, .. } => match target {
                Some(Target::Province(_)) => "province",
                Some(Target::Neighbour(_)) => "neighbour",
                Some(Target::Heir(_)) => "heir",
                None => "action",
            },
            DecisionKind::EventChoice { .. } => "choice",
            DecisionKind::Abdicate => "abdicate",
        };
        let shapes: std::collections::BTreeSet<_> = g.decisions.iter().map(shape).collect();
        let all = ["province", "neighbour", "heir", "action", "choice"];
        assert!(all.iter().all(|s| shapes.contains(s)), "{shapes:?}");
        let r = reopen(&g, seed);
        assert_eq!(r.decisions, g.decisions);
        assert_eq!(r.world, g.world);
        assert_eq!(r.pending_event, g.pending_event);
        // Four bytes or so a decision.
        assert!(encode("default", seed, &g).len() < 600);
    }

    #[test]
    fn idle_years_after_the_last_decision_and_an_abdication_come_back() {
        let mut g = play(5, 10);
        while g.pending_event.is_none() {
            if let Step::Event(_) = g.wait().unwrap() {
                break;
            }
        }
        // The link ends with the event on screen, not yet chosen.
        let r = reopen(&g, 5);
        assert_eq!(
            (r.world.tick, &r.pending_event),
            (g.world.tick, &g.pending_event)
        );
        g.choose(0).unwrap();
        g.abdicate().unwrap();
        let ev = g.data.events.iter().find(|e| e.id == "abdication").unwrap();
        let confirm = ev
            .choices
            .iter()
            .position(|c| c.effects.contains(&crate::rules::Effect::Abdicate));
        g.choose(confirm.unwrap()).unwrap();
        assert!(g.ended.is_some());
        let r = reopen(&g, 5);
        assert_eq!((&r.ended, &r.world), (&g.ended, &g.world));
        assert_eq!(r.decisions, g.decisions);
    }

    #[test]
    fn broken_links_are_refused() {
        let g = play(2, 5);
        let mut bytes = B64.decode(encode("default", 2, &g)).unwrap();
        assert!(decode(&B64.encode(&bytes[..5])).is_err());
        assert!(decode("не base64").unwrap_err().starts_with("base64"));
        bytes[0] = VERSION + 1;
        assert!(decode(&B64.encode(&bytes)).unwrap_err().contains("версия"));
        // A journal for another seed does not fit: some choice comes where no event waits.
        let link = decode(&encode("default", 2, &g)).unwrap();
        let other = (3..50).any(|s| link.play(&mut game(s)).is_err());
        assert!(other);
        // Steps cut short.
        let mut cut = decode(&encode("default", 2, &g)).unwrap();
        cut.steps.pop();
        cut.steps.push(9);
        assert!(cut.play(&mut game(2)).is_err());
    }
}
