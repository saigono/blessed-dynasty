//! Stage 29: the reign screen round the map. The header over it, the court along its foot,
//! and in the right column the card of what was clicked (the kingdom, a province of ours, a
//! neighbour, a person of the court) with the decrees aimed at it; the year's summary under it.

use super::*;
use crate::map::{CROWN, Click, cartouche_text, holder_color, map_font};
use bd_core::rules::HolderKind;

/// What the right column shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Card {
    Kingdom,
    /// A province of ours: the crown's or a vassal's.
    Province(ProvinceId),
    Neighbour(NeighbourId),
    /// The ruler (None) or an heir.
    Person(Option<u32>),
}

/// The card an action is offered on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Host {
    Kingdom,
    Province,
    Neighbour,
    Person,
}

/// The card of action `a`, by its target in the data: our land the province card; foreign
/// land, a court or the enemy the neighbour card; an heir the person card; nothing the kingdom
/// card. The laws have their list, opened from the kingdom card.
pub fn host(a: &Action) -> Host {
    match &a.target {
        ActionTarget::Province(f) if f.holder == Some(HolderKind::Foreign) => Host::Neighbour,
        ActionTarget::Province(_) => Host::Province,
        ActionTarget::Neighbour | ActionTarget::Enemy => Host::Neighbour,
        ActionTarget::Heir => Host::Person,
        ActionTarget::None => Host::Kingdom,
    }
}

/// `Game::available_actions`.
type Open = [(String, Vec<Target>)];

/// The card a click on the map opens: a foreign land its state's.
pub fn clicked(w: &World, click: Option<Click>) -> Option<Cmd> {
    let card = match click? {
        Click::Province(id) => match &w.provinces.get(&id)?.holder {
            Holder::Foreign(n) => Card::Neighbour(n.clone()),
            _ => Card::Province(id),
        },
        Click::State(n) => Card::Neighbour(n),
        Click::Kingdom => Card::Kingdom,
    };
    Some(Cmd::Card(card))
}

/// The dark band over the map: the date, the ruler and the war, the buttons of the year; the
/// treasury, the army, the land and the crown's affairs.
pub fn header(ui: &mut Ui, g: &Game, frame_ms: Option<f32>) -> Option<Cmd> {
    let (w, d) = (&g.world, &g.data);
    let mut cmd = None;
    let light = |t: &str| RichText::new(t).color(BG);
    let dim = |t: &str| RichText::new(t).color(BG2);
    ui.horizontal(|ui| {
        let date = w.tick.date(w.time_unit, w.start_year);
        ui.label(RichText::new(date).font(map_font(24.0)).color(BG));
        let age = w.ruler.age;
        let reign = w.tick.0.saturating_sub(w.ruler.reign_start.0) / w.time_unit.ticks_per_year + 1;
        let ruler = format!("{}, {age} {} · {reign}-й год правления", w.ruler.name, years(age));
        ui.label(dim(&ruler));
        if let Some(war) = &w.war {
            let enemy = target_name(w, &Target::Neighbour(war.enemy.clone()));
            let flag = RichText::new(format!("⚔ Война: {enemy}")).color(BG).strong();
            if ui.add(Button::new(flag).fill(RUBRIC)).clicked() {
                cmd = Some(Cmd::Card(Card::Neighbour(war.enemy.clone())));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let wait = Button::new(RichText::new("Подождать год ▸").color(FG).strong()).fill(CROWN);
            if ui.add(wait).on_hover_text(WAIT_TIP).clicked() {
                cmd = Some(Cmd::Wait);
            }
            if ui.button("Скопировать ссылку").on_hover_text(LINK_TIP).clicked() {
                cmd = Some(Cmd::CopyLink);
            }
            if ui.button("Родословная").clicked() {
                cmd = Some(Cmd::Tree(true));
            }
            if let Some(ms) = frame_ms {
                ui.small(dim(&format!("кадр {ms:.1} мс")));
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(dim("Казна"));
        ui.label(light(&round(w.axes[&d.economy.treasury])));
        // What a year brings, net, apart from the treasury itself (stage 25).
        let income = bd_core::war::yearly_income(w, d);
        tip_label(ui, dim(&format!("за год {} {INFO}", plus(income))), |ui| money_tip(ui, g));
        ui.separator();
        if let Some(army) = w.axes.get(&d.war.army) {
            ui.label(dim(axis_name(d, &d.war.army)));
            ui.label(light(&round(*army)));
            ui.separator();
        }
        ui.label(dim("Земли"));
        ui.label(light(&realm(w)));
        ui.separator();
        let (free, all) = slots(g);
        let affairs = light(&format!("Дела короны: свободно {free} из {all} {INFO}"));
        tip_label(ui, affairs, |ui| {
            ui.label(running(g));
        });
    });
    cmd
}

/// The crown's affairs of peace: free and all.
fn slots(g: &Game) -> (usize, usize) {
    let (w, d) = (&g.world, &g.data);
    let war = |id: &str| (d.actions.iter()).any(|a| a.id == id && a.target == ActionTarget::Enemy);
    let busy = (w.active_actions.iter()).filter(|a| !war(&a.id)).count();
    let all = d.action_slots.slots(w) as usize;
    (all.saturating_sub(busy), all)
}

/// The court along the foot of the map: the ruler, the succession law and who comes first,
/// the heirs and the bastards; a click on a person opens his card.
pub fn court(ui: &mut Ui, g: &Game, card: &Card) -> Option<Cmd> {
    let (w, d) = (&g.world, &g.data);
    let mut cmd = None;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let mut chip = |ui: &mut Ui, text: String, who: Option<u32>| {
            let on = *card == Card::Person(who);
            if ui.add(Button::new(text).selected(on)).clicked() {
                cmd = Some(Cmd::Card(Card::Person(who)));
            }
        };
        chip(ui, format!("♔ {}, {}", w.ruler.name, w.ruler.age), None);
        if let Some(wed) = wedded(g, None) {
            ui.label(RichText::new(wed).color(FG2));
        }
        ui.separator();
        match d.heirs.law(w) {
            Some(l) => {
                tip_label(ui, format!("Закон: {} {INFO}", l.name), |ui| {
                    ui.strong(&l.name);
                    ui.label(l.text());
                });
            }
            None => {
                ui.label("Закона наследования нет");
            }
        }
        let first = sim::successor(w, d);
        let line = first.map_or("никого".into(), |i| w.heirs[i].name.clone());
        ui.label(format!("Первый в очереди: {line}"));
        ui.separator();
        if w.heirs.is_empty() {
            ui.label(RichText::new("Наследников нет").color(RUBRIC));
        }
        for (i, h) in w.heirs.iter().enumerate() {
            let sex = match h.sex {
                Sex::Male => "♂",
                Sex::Female => "♀",
            };
            let status = match status(g, i) {
                (s, _) if s.is_empty() => String::new(),
                (s, _) => format!(" · {s}"),
            };
            let wed = if h.married { " ∞" } else { "" };
            chip(ui, format!("{sex} {}, {}{wed}{status}", h.name, h.age), Some(h.id));
        }
        if !w.bastards.is_empty() {
            let names: Vec<String> = (w.bastards.iter())
                .map(|h| format!("{}, {}", h.name, h.age))
                .collect();
            tip_label(ui, format!("Бастарды: {} {INFO}", names.join("; ")), |ui| {
                ui.label("Рождены вне брака и не наследуют, пока их не признают.");
            });
        }
    });
    ui.add_space(2.0);
    cmd
}

/// «назначен», «первый», «по закону», «учится: …», «заложник: …» of heir `i`, and its colour.
fn status(g: &Game, i: usize) -> (String, egui::Color32) {
    let (w, d) = (&g.world, &g.data);
    let (first, rightful) = (sim::successor(w, d), sim::rightful(w, d));
    match &w.heirs[i].status {
        HeirStatus::Home if Some(i) == first && first != rightful => ("назначен".into(), RUBRIC),
        HeirStatus::Home if Some(i) == first => ("первый".into(), FG2),
        HeirStatus::Home if Some(i) == rightful => ("по закону".into(), FG2),
        HeirStatus::Home => (String::new(), FG2),
        HeirStatus::Studying(place) => (format!("учится: {place}"), FG2),
        HeirStatus::Hostage(n) => {
            let n = w.neighbours.get(n).map_or(&n.0, |n| &n.name);
            (format!("заложник: {n}"), RUBRIC)
        }
    }
}

/// «в браке, родня: Веструм» of the ruler (None) or an heir; None unwed.
fn wedded(g: &Game, who: Option<u32>) -> Option<String> {
    let w = &g.world;
    let wed = match who {
        None => w.flags.contains(&g.data.heirs.married_flag),
        Some(id) => w.heir_index(id).is_some_and(|i| w.heirs[i].married),
    };
    let court = (w.unions.iter()).find(|(_, u)| u.spouse == who);
    let court = court.and_then(|(n, _)| w.neighbours.get(n)).map(|n| &n.name);
    match (wed, court) {
        (_, Some(n)) => Some(format!("∞ в браке, родня: {n}")),
        (true, None) => Some("∞ в браке".into()),
        _ => None,
    }
}

/// The card on the right, with a way back to the kingdom's.
pub fn card(ui: &mut Ui, g: &Game, card: &Card) -> Option<Cmd> {
    let open = g.available_actions();
    let mut cmd = None;
    if *card != Card::Kingdom && ui.small_button("◂ Королевство").clicked() {
        cmd = Some(Cmd::Card(Card::Kingdom));
    }
    let asked = match card {
        Card::Kingdom => kingdom(ui, g, &open),
        Card::Province(id) => province(ui, g, &open, id),
        Card::Neighbour(id) => neighbour(ui, g, &open, id),
        Card::Person(who) => person(ui, g, &open, *who),
    };
    asked.or(cmd)
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(map_font(24.0)).color(FG));
}

/// The kingdom: its decrees (the laws, those of no target, the testament, the abdication),
/// the war, the laws in force, the state of the realm and the neighbours.
fn kingdom(ui: &mut Ui, g: &Game, open: &Open) -> Option<Cmd> {
    let (w, d) = (&g.world, &g.data);
    let mut cmd = None;
    let (name, year) = cartouche_text(w, d);
    title(ui, &name);
    ui.label(RichText::new(year).color(RUBRIC));
    heading(ui, "Указы");
    if !d.laws.list.is_empty() && ui.button("Ввести закон").on_hover_text(LAWS_TIP).clicked() {
        cmd = Some(Cmd::Laws(true));
    }
    cmd = decrees(ui, g, open, Host::Kingdom, |_| Some(vec![])).or(cmd);
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        // Stage 24: written once, rewritten any time.
        let written = w.testament.as_ref().filter(|t| !t.by.0.is_empty());
        let label = match written {
            Some(_) => "Переписать завещание",
            None => "Составить завещание",
        };
        if d.testament.is_some() && ui.button(label).on_hover_text(WILL_TIP).clicked() {
            let draft = written.map_or_else(Testament::default, |t| Testament {
                precept: t.precept.clone(),
                order: t.order.clone(),
                heir: t.heir,
                ..Default::default()
            });
            cmd = Some(Cmd::Will(Some(draft)));
        }
        let abdicate = Button::new(RichText::new("Отречься").color(RUBRIC)).stroke((1.0, RUBRIC));
        if ui.add(abdicate).on_hover_text(ABDICATE_TIP).clicked() {
            cmd = Some(Cmd::Abdicate);
        }
    });
    if let Some(war) = &w.war {
        war_panel(ui, g, war);
    }
    heading(ui, "Соседи");
    Grid::new("neighbours").show(ui, |ui| {
        for n in w.neighbours.values() {
            let value = plus(n.relation);
            let at_war = w.war.as_ref().is_some_and(|x| x.enemy == n.id);
            let label = match (w.unions.contains_key(&n.id), at_war) {
                (_, true) => format!("{} ⚔", n.name),
                (true, _) => format!("{} ♥", n.name),
                _ => n.name.clone(),
            };
            let (lo, hi) = (Fx::from_int(-100), Fx::from_int(100));
            let row = bar(ui, &label, n.relation, lo, hi, &value);
            if row.on_hover_ui(|ui| about(ui, g, n)).clicked() {
                cmd = Some(Cmd::Card(Card::Neighbour(n.id.clone())));
            }
        }
    });
    // The laws in force and what each does to the graph.
    let laws: Vec<_> = d.laws_in_force(w).collect();
    if !laws.is_empty() {
        heading(ui, "Законы");
        for l in laws {
            tip_label(ui, format!("{} {INFO}", l.name), |ui| {
                ui.strong(&l.name);
                ui.label(&l.description);
            });
            let holds = holds(d, l);
            if !holds.is_empty() {
                ui.small(RichText::new(holds).color(FG2));
            }
        }
    }
    heading(ui, "Состояние");
    axes_panel(ui, g);
    if let Some((line, hint)) = overreach(g) {
        ui.label(RichText::new(line).color(RUBRIC));
        ui.label(RichText::new(hint).small().color(FG2));
    }
    cmd
}

/// A neighbour as the crown knows it: the relation and the mood, the strength and the
/// stance, its kingdom, the ties and the war.
fn about(ui: &mut Ui, g: &Game, n: &Neighbour) {
    let (w, d) = (&g.world, &g.data);
    ui.strong(&n.name);
    let ai = &d.neighbour_ai;
    let mood = match n.relation {
        r if r > ai.friendly_above => "друг",
        r if r < ai.hostile_below => "враг",
        _ => "нейтрален",
    };
    let stance = match n.stance {
        Stance::Expand => "ищет, что захватить",
        Stance::Defend => "обороняется",
        Stance::Trade => "торгует",
        Stance::Wait => "выжидает",
    };
    ui.label(format!("Отношение {}: {mood}", plus(n.relation)));
    ui.label(format!("Сила {}, {stance}", round(n.strength)));
    realm_tip(ui, d, n);
    ally_tip(ui, w, d, n);
    let bonds = g.bonds();
    let ties: Vec<String> = (bonds.iter())
        .filter(|(id, ..)| *id == n.id)
        .map(|(_, a, t)| format!("{} с {}", a.bond, t.date(w.time_unit, w.start_year)))
        .collect();
    for t in &ties {
        ui.label(t);
    }
    // Who is wed into that court: the ruler or an heir.
    if let Some(u) = w.unions.get(&n.id) {
        let spouse = match u.spouse {
            None => Some(&w.ruler.name),
            Some(id) => w.heir_index(id).map(|i| &w.heirs[i].name),
        };
        ui.label(format!("В браке: {}", spouse.map_or("", |s| s)));
    }
    if ties.is_empty() {
        ui.label(RichText::new("Союзов и браков нет").color(FG2));
    }
    if w.war.as_ref().is_some_and(|x| x.enemy == n.id) {
        ui.label(RichText::new("Идёт война").color(RUBRIC));
    }
}

/// A province of ours: what the map's tip tells, then its decrees.
fn province(ui: &mut Ui, g: &Game, open: &Open, id: &ProvinceId) -> Option<Cmd> {
    let (w, d) = (&g.world, &g.data);
    let p = w.provinces.get(id)?;
    target_tip(ui, w, d, &Target::Province(id.clone()));
    if w.war.as_ref().and_then(|x| x.target.as_ref()) == Some(id) {
        ui.label(RichText::new("Цель войны").color(RUBRIC));
    }
    if p.loyalty < d.unrest_below {
        ui.label(RichText::new("Смута: земля на грани мятежа").color(RUBRIC));
    }
    heading(ui, "Указы");
    let kind = match p.holder {
        Holder::Crown => HolderKind::Crown,
        Holder::Vassal(_) => HolderKind::Vassal,
        Holder::Foreign(_) => HolderKind::Foreign,
    };
    decrees(ui, g, open, Host::Province, |a| match &a.target {
        ActionTarget::Province(f) if f.holder.is_none_or(|k| k == kind) => {
            Some(vec![Target::Province(id.clone())])
        }
        _ => None,
    })
}

/// A neighbour: what the crown knows of it, the war (its lands to take and the strengths,
/// or the war going on), the suit and the rest of diplomacy.
fn neighbour(ui: &mut Ui, g: &Game, open: &Open, id: &NeighbourId) -> Option<Cmd> {
    let (w, d) = (&g.world, &g.data);
    let n = w.neighbours.get(id)?;
    let colour = holder_color(w, &Holder::Foreign(id.clone()));
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 2.0, colour);
        title(ui, &n.name);
    });
    about(ui, g, n);
    let lands: Vec<&bd_core::state::Province> = (w.provinces.values())
        .filter(|p| p.holder == Holder::Foreign(id.clone()))
        .collect();
    ui.label(format!("Земель {}", lands.len()));
    let mut cmd = None;
    heading(ui, "Война");
    let (ours, theirs) = bd_core::war::strengths(w, d, id);
    ui.label(format!("Силы: наши {} · их {}", round(ours), round(theirs)));
    let at_war = w.war.as_ref().filter(|x| x.enemy == *id);
    if let Some(war) = at_war {
        war_panel(ui, g, war);
    }
    let border = |f: &bd_core::rules::ProvinceFilter| -> Vec<Target> {
        (lands.iter())
            .filter(|p| f.matches(p, w))
            .map(|p| Target::Province(p.id.clone()))
            .collect()
    };
    let can_take = (d.actions.iter()).any(|a| match &a.target {
        ActionTarget::Province(f) => host(a) == Host::Neighbour && !border(f).is_empty(),
        _ => false,
    });
    if at_war.is_none() && !can_take {
        ui.label(RichText::new("Общей границы нет: войны не объявить").color(FG2));
    }
    cmd = decrees(ui, g, open, Host::Neighbour, |a| match &a.target {
        ActionTarget::Enemy => at_war.map(|_| vec![Target::Neighbour(id.clone())]),
        ActionTarget::Province(f) if at_war.is_none() => Some(border(f)).filter(|t| !t.is_empty()),
        _ => None,
    })
    .or(cmd);
    heading(ui, "Дипломатия");
    decrees(ui, g, open, Host::Neighbour, |a| match a.target {
        ActionTarget::Neighbour => Some(vec![Target::Neighbour(id.clone())]),
        _ => None,
    })
    .or(cmd)
}

/// The ruler (his age, health and marriage, the succession), or an heir (his standing in
/// the line and the decrees aimed at him).
fn person(ui: &mut Ui, g: &Game, open: &Open, who: Option<u32>) -> Option<Cmd> {
    let (w, d) = (&g.world, &g.data);
    let law = |ui: &mut Ui| {
        if let Some(l) = d.heirs.law(w) {
            tip_label(ui, format!("Закон: {} {INFO}", l.name), |ui| {
                ui.strong(&l.name);
                ui.label(l.text());
            });
        }
        let rightful = sim::rightful(w, d).map_or("никто", |i| w.heirs[i].name.as_str());
        ui.label(format!("По закону престол наследует: {rightful}"));
    };
    let Some(id) = who else {
        title(ui, &format!("♔ {}", w.ruler.name));
        ui.label(format!("{} {}", w.ruler.age, years(w.ruler.age)));
        Grid::new("health").show(ui, |ui| {
            let health = round(w.ruler.health);
            bar(ui, "Здоровье", w.ruler.health, Fx(0), Fx::from_int(100), &health)
        });
        ui.label(wedded(g, None).unwrap_or("Не в браке".into()));
        let will = match w.testament.as_ref().filter(|t| !t.by.0.is_empty()) {
            Some(_) => "Завещание составлено",
            None => "Завещания нет",
        };
        ui.label(RichText::new(will).color(FG2));
        heading(ui, "Престол");
        law(ui);
        return None;
    };
    let i = w.heir_index(id)?;
    let h = &w.heirs[i];
    let line = match (h.bastard, h.id >= w.line_from, h.sex) {
        (true, ..) => "бастард",
        (_, true, Sex::Male) => "королевский сын",
        (_, true, Sex::Female) => "королевская дочь",
        _ => "из боковой линии",
    };
    title(ui, &h.name);
    ui.label(format!("{line}, {} {}", h.age, years(h.age)));
    let (s, colour) = status(g, i);
    if !s.is_empty() {
        ui.label(RichText::new(s).color(colour));
    }
    ui.label(format!("Способности {} · претензия {}", round(h.ability), round(h.claim)));
    ui.label(wedded(g, Some(id)).unwrap_or("Не в браке".into()));
    heading(ui, "Престол");
    law(ui);
    let lawful = sim::rightful(w, d) == Some(i);
    let how = if lawful { "по закону" } else { "в обход закона" };
    ui.label(RichText::new(format!("{} · {how}", h.name)).color(FG2));
    heading(ui, "Указы");
    decrees(ui, g, open, Host::Person, |_| Some(vec![Target::Heir(id)]))
}

/// The decrees of a card: every action it hosts with its targets here (`targets`: None
/// leaves the action out, no target is an action without one).
fn decrees(
    ui: &mut Ui,
    g: &Game,
    open: &Open,
    on: Host,
    targets: impl Fn(&Action) -> Option<Vec<Target>>,
) -> Option<Cmd> {
    let own = |a: &&Action| !a.id.starts_with(ENACT) && !a.id.starts_with(REPEAL) && host(a) == on;
    let mut cmd = None;
    for a in g.data.actions.iter().filter(own) {
        if let Some(t) = targets(a) {
            cmd = decree(ui, g, open, a, &t).or(cmd);
        }
    }
    cmd
}

/// A decree: its button (one for each target when it has several: a war on one of their
/// lands), grey with the reason when it cannot start; its price, years and effects under it.
fn decree(ui: &mut Ui, g: &Game, open: &Open, a: &Action, targets: &[Target]) -> Option<Cmd> {
    let (w, d) = (&g.world, &g.data);
    let mut cmd = None;
    ui.add_space(4.0);
    let why: Vec<Option<String>> = match targets {
        [] => vec![why_not_on(g, open, a, None)],
        ts => ts.iter().map(|t| why_not_on(g, open, a, Some(t))).collect(),
    };
    let name = named(w, &a.name);
    if targets.len() < 2 {
        let tip = |ui: &mut Ui| decree_tip(ui, g, a, &why[0]);
        if button(ui, &name, why[0].is_some(), tip).clicked() && why[0].is_none() {
            cmd = Some(Cmd::Act(a.id.clone(), targets.first().cloned()));
        }
    } else {
        tip_label(ui, RichText::new(&name).strong(), |ui| action_tip(ui, g, a));
        ui.horizontal_wrapped(|ui| {
            for (t, why) in targets.iter().zip(&why) {
                let label = match t {
                    Target::Province(id) => w.provinces.get(id).map_or(id.0.clone(), |p| p.name.clone()),
                    t => target_name(w, t),
                };
                let tip = |ui: &mut Ui| decree_tip(ui, g, a, why);
                if button(ui, &label, why.is_some(), tip).clicked() && why.is_none() {
                    cmd = Some(Cmd::Act(a.id.clone(), Some(t.clone())));
                }
            }
        });
    }
    ui.small(RichText::new(terms(d, a)).color(FG2));
    if let ([Target::Neighbour(n)], [None]) = (targets, &why[..])
        && a.marries()
    {
        let chance = round(d.marriage.chance(w, d, n));
        ui.small(RichText::new(format!("Шанс согласия: {chance}%")).color(GOOD));
    }
    // The reason, when nothing here can start.
    if let Some(Some(first)) = why.first().filter(|_| why.iter().all(Option::is_some)) {
        ui.small(RichText::new(first).color(RUBRIC));
    }
    cmd
}

/// What a decree does, and why it cannot start.
fn decree_tip(ui: &mut Ui, g: &Game, a: &Action, why: &Option<String>) {
    action_tip(ui, g, a);
    if let Some(why) = why {
        ui.label(RichText::new(why).color(RUBRIC));
    }
}

/// A decree's button: grey when it cannot start (a click opens its tip then), its tip on hover.
fn button(ui: &mut Ui, text: &str, grey: bool, add: impl Fn(&mut Ui)) -> egui::Response {
    match grey {
        true => tip(ui.add(Button::new(RichText::new(text).color(FG2)).fill(BG2)), add),
        false => ui.button(text).on_hover_ui(add),
    }
}

/// «60 золота · 2 года · сила короны в провинции +5, доход провинции +1».
fn terms(d: &Data, a: &Action) -> String {
    let time = match a.duration_years.0 {
        0 => "сразу".to_string(),
        n => format!("{n} {}", years(n)),
    };
    let mut parts = vec![format!("{} золота", round(a.cost)), time];
    let does: Vec<String> = effects(d, &a.on_complete).into_iter().map(|(t, _)| t).collect();
    if !does.is_empty() {
        parts.push(does.join(", "));
    }
    parts.join(" · ")
}

/// Why action `a` cannot start on `t` now (None: an action without a target); None when it
/// can. Running here already first, the action's own condition and price next, then what
/// this target lacks.
fn why_not_on(g: &Game, open: &Open, a: &Action, t: Option<&Target>) -> Option<String> {
    let (w, d) = (&g.world, &g.data);
    let targets = open.iter().find(|(id, _)| *id == a.id).map(|x| &x.1);
    if targets.is_some_and(|ts| t.is_none_or(|t| ts.contains(t))) {
        return why_not(g, a, true);
    }
    let Some(t) = t else {
        return why_not(g, a, false);
    };
    let key = match t {
        Target::Province(id) => id.0.clone(),
        Target::Neighbour(n) => n.0.clone(),
        Target::Heir(id) => id.to_string(),
    };
    // Going on here already says more than the price of another.
    if (w.active_actions.iter()).any(|x| x.id == a.id && x.target.as_ref() == Some(&key)) {
        return Some(busy_text(a).into());
    }
    if let Some(why) = blocked(g, a) {
        return Some(why);
    }
    let power = round(a.min_crown_power);
    let capital = w.provinces.get(&w.capital.province).map_or(Fx(0), |p| p.crown_power);
    Some(match (t, &a.target) {
        (Target::Province(id), ActionTarget::Province(f)) => {
            let p = w.provinces.get(id)?;
            match &f.without_building {
                Some(b) if p.buildings.contains(b) => "Уже построено".into(),
                _ if f.capital == Some(false) && *id == w.capital.province => "Не для столицы".into(),
                _ if !f.matches(p, w) => "Не для этой земли".into(),
                _ => format!("Нужна сила короны в земле от {power}"),
            }
        }
        _ if capital < a.min_crown_power => format!("Нужна сила короны в столице от {power}"),
        (Target::Neighbour(n), _) if a.marries() => refusal(g, n),
        (Target::Heir(id), _) => {
            let h = w.heir_index(*id).map(|i| &w.heirs[i]);
            match h {
                Some(h) if h.married => "Уже в браке".into(),
                Some(h) if h.age < d.marriage.age => format!("Моложе {} лет", d.marriage.age),
                _ => "Недоступно".into(),
            }
        }
        _ => "Недоступно".into(),
    })
}

/// Why the court of `n` turns a suit away now.
fn refusal(g: &Game, n: &NeighbourId) -> String {
    let (w, d) = (&g.world, &g.data);
    let m = &d.marriage;
    let relation = w.neighbours.get(n).map_or(Fx(0), |x| x.relation);
    if w.war.as_ref().is_some_and(|x| x.enemy == *n) {
        "В войну сватов не принимают".into()
    } else if w.unions.contains_key(n) {
        "С этим двором уже породнились".into()
    } else if m.spouse(w, d).is_none() {
        format!("Сватать некого: государь в браке, неженатых наследников от {} лет нет", m.age)
    } else if relation < m.refuse_below {
        format!("Отношения ниже {}: сватов не примут", round(m.refuse_below))
    } else {
        "Сватов не примут".into()
    }
}
