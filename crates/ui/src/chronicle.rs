//! The dynasty after the founder: the chronicle with its map snapshots, then the score.

use crate::map::{BG, BG2, FG, FG2, GOOD, MapView, RUBRIC, round};
use crate::{Cmd, OMEN, axis_name, heading, lands, plural, realm, tone};
use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::Game;
use bd_core::score::Score;
use bd_core::sim::{Chronicle, ChronicleEntry, RulerRecord};
use bd_core::state::{Kin, World};
use bd_core::testament;
use bd_core::time::TimeUnit;
use eframe::egui::{self, Button, Grid, RichText, Ui, vec2};

/// Display names of `score::PARTS`, in the order shown.
pub const PART_NAMES: [(&str, &str); 5] = [
    ("years", "Годы династии"),
    ("territory_years", "Территория"),
    ("prestige", "Престиж"),
    ("stability", "Стабильность"),
    ("legacy", "Наследие"),
];

/// For each entry: the index in `c.rulers` of who reigned then, and whether the entry
/// crowns him (the `sim.texts.crowned` entry made at his start).
pub fn reigns(c: &Chronicle, d: &Data) -> Vec<(usize, bool)> {
    let mut r = 0;
    (c.entries.iter())
        .map(|e| {
            let crowns = c.rulers.get(r + 1).is_some_and(|n| {
                let title = d.sim.texts.crowned.0.replace("{ruler}", &n.name);
                e.event.is_none() && e.tick == n.start && e.title == title
            });
            r += crowns as usize;
            (r, crowns)
        })
        .collect()
}

pub fn chronicle(
    ui: &mut Ui,
    g: &Game,
    c: &Chronicle,
    map: &MapView,
    selected: usize,
) -> Option<Cmd> {
    let (d, w) = (&g.data, &g.world);
    let reigns = reigns(c, d);
    let mut cmd = None;
    egui::Panel::top("chronicle-top").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Хроника").size(22.0).strong());
            ui.label(RichText::new(house(c, w.start_year)).color(FG2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(primary("К итогу ▸")).clicked() {
                    cmd = Some(Cmd::Summary);
                }
                if ui.button("Родословная").clicked() {
                    cmd = Some(Cmd::Tree(true));
                }
            });
        })
    });
    egui::Panel::left("years").exact_size(230.0).show(ui, |ui| {
        ui.visuals_mut().selection.bg_fill = BG2;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, (e, &(r, crowns))) in c.entries.iter().zip(&reigns).enumerate() {
                let date = e.tick.date(w.time_unit, w.start_year);
                let text = match crowns {
                    true => RichText::new(format!(
                        "{date}  † {}. {}",
                        c.rulers[r - 1].full_name(),
                        c.rulers[r].full_name()
                    ))
                    .strong()
                    .color(FG),
                    false => RichText::new(format!("{date}  {}", title(d, e))).color(FG2),
                };
                let resp = ui.add(Button::selectable(i == selected, text).truncate());
                if i == selected {
                    let rect = resp.rect;
                    ui.painter()
                        .vline(rect.left(), rect.y_range(), (2.0, RUBRIC));
                }
                if resp.clicked() {
                    cmd = Some(Cmd::Entry(i));
                }
            }
        })
    });
    let Some(e) = c.entries.get(selected) else {
        egui::CentralPanel::default().show(ui, |ui| ui.label(&c.epilogue));
        return cmd;
    };
    egui::Panel::right("snapshot")
        .exact_size(320.0)
        .show(ui, |ui| {
            ui.allocate_ui(vec2(ui.available_width(), 260.0), |ui| {
                map.show(ui, &e.snapshot, d, &[]);
            });
            // The numbers of the year on hover (stage 22).
            let more = RichText::new(crate::NUMBERS).small().color(FG2);
            crate::tip_label(ui, more, |ui| snapshot_table(ui, e, d));
        });
    egui::CentralPanel::default().show(ui, |ui| {
        ui.set_max_width(680.0);
        let (r, crowns) = reigns[selected];
        let ruler = &c.rulers[r];
        let tpy = w.time_unit.ticks_per_year;
        let reign = e.tick.0.saturating_sub(ruler.start.0) / tpy + 1;
        let dynasty = e.tick.year(w.time_unit) + 1;
        ui.label(
            RichText::new(e.tick.date(w.time_unit, w.start_year))
                .size(30.0)
                .strong(),
        );
        ui.label(
            RichText::new(format!(
                "{} · {reign}-й год правления · {dynasty}-й год династии",
                ruler.full_name()
            ))
            .small()
            .color(FG2),
        );
        // The founder's testament as this ruler reads it (stage 24).
        if r > 0
            && let Some(line) = will_strength(d, &e.snapshot)
        {
            ui.label(RichText::new(line).small().color(FG2));
        }
        ui.add_space(10.0);
        ui.label(RichText::new(title(d, e)).size(18.0).strong());
        ui.label(&e.text);
        // What led to it through the graph (stage 20).
        if let Some(chain) = &e.chain {
            ui.label(RichText::new(&chain.text).color(FG2));
        }
        if crowns {
            let prev = &c.rulers[r - 1];
            let cause = prev.cause.as_deref().map_or("", |c| reign_end(d, c));
            let years = |t: bd_core::time::Tick| t.date(w.time_unit, w.start_year);
            ui.label(
                RichText::new(format!(
                    "{}, {}–{}, конец правления: {cause}",
                    prev.full_name(),
                    years(prev.start),
                    years(prev.end)
                ))
                .color(FG2),
            );
        }
        if let Some(h) = &e.hint {
            ui.add_space(6.0);
            let margin = egui::Margin {
                left: 10,
                ..Default::default()
            };
            let frame = egui::Frame::new().inner_margin(margin);
            let resp = frame.show(ui, |ui| ui.label(RichText::new(h).italics().color(FG2)));
            let rect = resp.response.rect;
            ui.painter()
                .vline(rect.left(), rect.y_range(), (2.0, RUBRIC));
        }
        // How the dynasty ended, after its last entry.
        if selected + 1 == c.entries.len() {
            ui.add_space(10.0);
            ui.label(RichText::new(&c.epilogue).italics());
        }
    });
    cmd
}

/// «Завет Ульриха: сила 0,92 — слава 1,31 × память 78% × рвение 0,9», the strength of the
/// testament in `w` and its parts (`testament::parts`); None before the founder's death.
fn will_strength(d: &Data, w: &World) -> Option<String> {
    let (legend, decay, zeal) = testament::parts(d, w)?;
    let t = w.testament.as_ref()?;
    let n = |v: Fx| format!("{:.2}", v.0 as f32 / 1000.0).replace('.', ",");
    let names = [("founder", t.by.0.as_str(), Some(t.by.1))];
    let whose = bd_core::text::fill("Завет {founder.род}", &d.names, &names);
    let broken = if t.broken.is_some() {
        " · наказ нарушен"
    } else {
        ""
    };
    Some(format!(
        "{whose}: сила {} — слава {} × память {}% × рвение {}{broken}",
        n(testament::strength(d, w)),
        n(legend),
        decay.0 / 10,
        n(zeal)
    ))
}

/// The testament of the founder, as read at his death, and its strength then; None
/// without one.
fn will_read(d: &Data, c: &Chronicle) -> Option<(String, String)> {
    let read = &d.testament.as_ref()?.texts.read.0;
    let e = c.entries.iter().find(|e| e.title == *read)?;
    Some((e.text.clone(), will_strength(d, &e.snapshot)?))
}

/// The entry's title, after `OMEN` for a symptom of the graph (`Event.omen`).
fn title(d: &Data, e: &ChronicleEntry) -> String {
    let mut all = d.events.iter().chain(&d.sim_events);
    let omen = e
        .event
        .as_ref()
        .is_some_and(|id| all.any(|x| x.id == *id && x.omen));
    format!("{}{}", if omen { OMEN } else { "" }, e.title)
}

/// Provinces, axes and heirs of the entry's year.
fn snapshot_table(ui: &mut Ui, e: &ChronicleEntry, d: &Data) {
    let w = &e.snapshot;
    Grid::new("snapshot-axes").striped(true).show(ui, |ui| {
        let mut row = |k: &str, v: String| {
            ui.small(RichText::new(k).color(FG2));
            ui.small(RichText::new(v).color(FG));
            ui.end_row();
        };
        row("Провинций", realm(w));
        for a in d.axes.iter().filter(|a| !a.hidden) {
            row(axis_name(d, &a.id), round(w.axes[&a.id]));
        }
        row("Наследников", w.heirs.len().to_string());
    });
}

pub fn summary(
    ui: &mut Ui,
    g: &Game,
    (c, s): (&Chronicle, &Score),
    seed: u64,
    selected: usize,
) -> Option<Cmd> {
    let (d, w) = (&g.data, &g.world);
    let mut cmd = None;
    let half = ui.available_width() / 2.0;
    egui::Panel::left("summary-left")
        .exact_size(half)
        .show(ui, |ui| {
            ui.add_space(16.0);
            heading(ui, &house(c, w.start_year));
            ui.label(RichText::new(c.years.to_string()).size(56.0).strong());
            let rulers = c.rulers.len() as u32;
            ui.label(
                RichText::new(format!(
                    "{} на троне · {rulers} {} · {}",
                    plural(c.years, ["год", "года", "лет"]),
                    plural(rulers, ["правитель", "правителя", "правителей"]),
                    fall(c, d)
                ))
                .color(FG2),
            );
            ui.label(RichText::new(&c.epilogue).italics());
            ui.add_space(20.0);
            heading(ui, "Итоговый счёт");
            ui.label(RichText::new(thousands(s.total)).size(40.0).strong());
            ui.add_space(20.0);
            ui.horizontal_wrapped(|ui| {
                if ui.add(primary("Тот же старт, заново")).clicked() {
                    cmd = Some(Cmd::Restart);
                }
                if ui.button("Новый seed").clicked() {
                    cmd = Some(Cmd::NewSeed);
                }
                if ui.button("Скопировать ссылку").clicked() {
                    cmd = Some(Cmd::CopyLink);
                }
            });
            let decisions = g.decisions.len() as u32;
            let line = format!(
                "seed {seed} · {decisions} {}",
                plural(decisions, ["решение", "решения", "решений"])
            );
            ui.small(RichText::new(line).color(FG2));
            ui.add_space(12.0);
            if ui.button("◂ К хронике").clicked() {
                cmd = Some(Cmd::Entry(selected));
            }
        });
    egui::CentralPanel::default().show(ui, |ui| {
        ui.add_space(16.0);
        let max = s.parts.values().copied().max().unwrap_or(0).max(1);
        Grid::new("parts").spacing(vec2(12.0, 10.0)).show(ui, |ui| {
            for (key, name) in PART_NAMES {
                let v = s.parts.get(key).copied().unwrap_or(0);
                ui.label(name);
                let (r, _) = ui.allocate_exact_size(vec2(220.0, 8.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 0.0, BG2);
                let share = (v.max(0) as f32 / max as f32).min(1.0);
                let bar = egui::Rect::from_min_size(r.min, vec2(r.width() * share, r.height()));
                ui.painter().rect_filled(bar, 0.0, FG);
                ui.label(RichText::new(thousands(v)).monospace());
                ui.end_row();
            }
        });
        ui.add_space(18.0);
        heading(ui, "Что решило судьбу");
        for dd in &s.decisive {
            let (sign, color) = match dd.weight {
                v if v > Fx(0) => ("+", GOOD),
                v if v < Fx(0) => ("−", RUBRIC),
                _ => ("·", FG2),
            };
            let tag = &dd.decision.cause_tag;
            let hint = d.hints.get(tag).map_or(tag.clone(), |h| sentence(h));
            let date = dd.decision.tick.date(w.time_unit, w.start_year);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(sign).strong().size(18.0).color(color));
                ui.label(RichText::new(date).color(FG2));
                ui.label(hint);
            });
        }
        // The testament and how the dynasty kept it (stage 24).
        if let Some((text, strength)) = will_read(d, c) {
            ui.add_space(6.0);
            ui.label(RichText::new("Завещание основателя").strong());
            ui.label(text);
            let breach = d.testament.as_ref().map(|r| &r.texts.breach);
            let broke = c.entries.iter().find(|e| Some(&e.title) == breach);
            let has_order = w.testament.as_ref().is_some_and(|t| t.order.is_some());
            let fate = match broke {
                Some(e) => format!(
                    "Наказ нарушен в {} году.",
                    e.tick.date(w.time_unit, w.start_year)
                ),
                None if has_order => "Наказ соблюдали до конца династии.".to_string(),
                None => String::new(),
            };
            ui.small(RichText::new(format!("{strength}. {fate}")).color(FG2));
        }
        let laws = founder_laws(g, c);
        if !laws.is_empty() {
            heading(ui, "Законы основателя");
        }
        for line in laws {
            ui.label(line);
        }
    });
    cmd
}

/// «1214  Крепостное право: основатель прикрепил крестьян к земле господ · до конца
/// династии»: every law the founder brought in and left in force (`World.laws` at the end of
/// his reign), with its hint and whether the dynasty kept it to the end or when it was
/// repealed (the first entry of the chronicle without it).
fn founder_laws(g: &Game, c: &Chronicle) -> Vec<String> {
    let (d, w) = (&g.data, &g.world);
    let date = |t: bd_core::time::Tick| t.date(w.time_unit, w.start_year);
    (d.laws_in_force(w)
        .filter_map(|l| w.laws.get(&l.id).map(|t| (l, t))))
    .map(|(l, since)| {
        let hint = d
            .hints
            .get(&l.id)
            .map_or(String::new(), |h| format!(": {h}"));
        let gone = c.entries.iter().find(|e| !e.snapshot.flags.contains(&l.id));
        let fate = gone.map_or("до конца династии".into(), |e| {
            format!("отменён в {}", date(e.tick))
        });
        format!("{}  {}{hint} · {fate}", date(*since), l.name)
    })
    .collect()
}

/// The card at the end of the founder's reign, over the last reign screen: its years and end,
/// the decisions that told, what became of the land and the axes, who took the throne.
pub fn reign_over(
    ctx: &egui::Context,
    g: &Game,
    start: &World,
    (c, s): &(Chronicle, Score),
) -> Option<Cmd> {
    let (d, w) = (&g.data, &g.world);
    let mut cmd = None;
    egui::Modal::new(egui::Id::new("reign-over")).show(ctx, |ui| {
        ui.set_width(600.0);
        let founder = &c.rulers[0];
        let date = |t: bd_core::time::Tick| t.date(w.time_unit, w.start_year);
        let years = (founder.end.0 - founder.start.0) / w.time_unit.ticks_per_year;
        let cause = founder.cause.as_deref().map_or("", |c| reign_end(d, c));
        ui.label(RichText::new("Итог правления").small().color(RUBRIC));
        ui.label(RichText::new(founder.full_name()).size(22.0).strong());
        ui.label(format!(
            "{}–{}, {years} {} на троне. Конец правления: {cause}.",
            date(founder.start),
            date(founder.end),
            plural(years, ["год", "года", "лет"]),
        ));
        heading(ui, "Наследник");
        // The successor as the chronicle crowned him.
        let crowned = (reigns(c, d).iter()).position(|&(r, crowns)| r == 1 && crowns);
        let heir = match (crowned, c.rulers.get(1)) {
            (Some(i), _) => c.entries[i].text.clone(),
            (None, Some(r)) => format!("На престол взошёл {}.", r.name),
            (None, None) => format!("Наследника не осталось: {}.", fall(c, d).to_lowercase()),
        };
        ui.label(heir);
        // What he left his heirs (stage 24).
        if let Some((text, strength)) = will_read(d, c) {
            heading(ui, "Завещание");
            ui.label(text);
            ui.small(RichText::new(strength).color(FG2));
        }
        let room = ctx.content_rect().height() - 380.0;
        egui::ScrollArea::vertical()
            .max_height(room.max(200.0))
            .show(ui, |ui| {
                heading(ui, "Жизнеописание");
                ui.label(RichText::new(&founder.biography).italics());
                heading(ui, "Ключевые решения");
                if s.decisive.is_empty() {
                    ui.label(
                        RichText::new("Ни одно решение не отозвалось в судьбе династии.")
                            .color(FG2),
                    );
                }
                for dd in s.decisive.iter().take(5) {
                    let up = (dd.weight != Fx(0)).then_some(dd.weight > Fx(0));
                    let tag = &dd.decision.cause_tag;
                    let hint = d.hints.get(tag).map_or(tag.clone(), |h| sentence(h));
                    let text = format!("{}  {hint}", date(dd.decision.tick));
                    ui.label(RichText::new(text).color(tone(up)));
                }
                heading(ui, "Земли");
                match lands(d, start, w) {
                    Some((text, up)) => ui.label(RichText::new(text).color(tone(up))),
                    None => ui.label("Земли без перемен"),
                };
                ui.small(RichText::new(format!("Теперь земель {}", realm(w))).color(FG2));
                heading(ui, "Состояние");
                Grid::new("reign-axes").show(ui, |ui| {
                    for a in d.axes.iter().filter(|a| !a.hidden) {
                        let (from, to) = (start.axes[&a.id], w.axes[&a.id]);
                        let up = (to != from).then_some(to > from);
                        ui.small(axis_name(d, &a.id));
                        ui.small(format!("{} → {}", round(from), round(to)));
                        ui.small(
                            RichText::new(if to > from {
                                "▲"
                            } else if to < from {
                                "▼"
                            } else {
                                ""
                            })
                            .color(tone(up)),
                        );
                        ui.end_row();
                    }
                });
            });
        ui.add_space(8.0);
        if ui.add(primary("К хронике ▸")).clicked() {
            cmd = Some(Cmd::Entry(0));
        }
    });
    cmd
}

/// The family tree card: the founder, his children under him and so on down, the dead in
/// grey, rulers with their reigns (`rulers`, in crowning order, once the dynasty is played).
/// A click on a ruler opens his life under him (kept in egui memory). True when closed.
pub fn tree(
    ctx: &egui::Context,
    kin: &[Kin],
    rulers: &[RulerRecord],
    start_year: u32,
    unit: TimeUnit,
) -> bool {
    let mut close = false;
    let open_id = egui::Id::new("tree-life");
    let open = ctx.data(|d| d.get_temp::<Option<usize>>(open_id)).flatten();
    let modal = egui::Modal::new(egui::Id::new("tree")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.label(RichText::new("Родословная").size(20.0).strong());
        egui::ScrollArea::vertical()
            .max_height(520.0)
            .show(ui, |ui| {
                let reign = |i: usize| {
                    let n = kin[..i].iter().filter(|k| k.crowned.is_some()).count();
                    rulers.get(n)
                };
                for (i, depth) in family(kin) {
                    let k = &kin[i];
                    // A reign over with no death: the ruler abdicated (sim::died).
                    let gave_up = (k.crowned.and(reign(i)))
                        .filter(|r| k.died.is_none() && r.cause.is_some())
                        .map(|r| r.end.date(unit, start_year));
                    let life = match (k.died, gave_up) {
                        (Some(d), _) => format!("{}–{d}", k.born),
                        (None, Some(y)) => format!("р. {}, отречение в {y}", k.born),
                        (None, None) => format!("р. {}", k.born),
                    };
                    let bastard = if k.bastard { ", бастард" } else { "" };
                    let name = (k.crowned.and(reign(i))).map_or(k.name.clone(), |r| r.full_name());
                    let mut text = format!("{name} ({life}{bastard})");
                    if let Some(y) = k.crowned {
                        let till = reign(i)
                            .filter(|r| r.cause.is_some())
                            .map(|r| r.end.date(unit, start_year));
                        text = format!(
                            "♔ {text}, на троне с {y}{}",
                            till.map_or(String::new(), |t| format!(" по {t}"))
                        );
                    }
                    let text = RichText::new(text);
                    let text = match (k.died.is_some(), k.crowned.is_some()) {
                        (true, _) => text.color(FG2.gamma_multiply(0.7)),
                        (false, true) => text.strong(),
                        _ => text,
                    };
                    let life = k.crowned.and(reign(i)).map(|r| &r.biography);
                    let life = life.filter(|b| !b.is_empty());
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 20.0);
                        let label = egui::Label::new(text);
                        match life {
                            Some(_) => {
                                let resp = ui.add(label.sense(egui::Sense::click()));
                                if resp.on_hover_text("Жизнеописание").clicked() {
                                    let next = (open != Some(i)).then_some(i);
                                    ctx.data_mut(|d| d.insert_temp(open_id, next));
                                }
                            }
                            None => {
                                ui.add(label);
                            }
                        }
                    });
                    if let Some(b) = life.filter(|_| open == Some(i)) {
                        let left = (depth * 20 + 20).min(120) as i8;
                        let margin = egui::Margin {
                            left,
                            ..Default::default()
                        };
                        egui::Frame::new().inner_margin(margin).show(ui, |ui| {
                            ui.label(RichText::new(b).italics().color(FG));
                        });
                    }
                }
            });
        ui.add_space(8.0);
        close = ui.button("Закрыть").clicked();
    });
    close || modal.should_close()
}

/// Kin in tree order, depth first, children in order of appearance, with their depth.
pub fn family(kin: &[Kin]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut stack: Vec<(usize, usize)> = (0..kin.len())
        .rev()
        .filter(|&i| kin[i].parent.is_none())
        .map(|i| (i, 0))
        .collect();
    while let Some((i, depth)) = stack.pop() {
        out.push((i, depth));
        let children = (0..kin.len()).rev().filter(|&j| kin[j].parent == Some(i));
        stack.extend(children.map(|j| (j, depth + 1)));
    }
    out
}

/// «Ульрих и потомки · 1187–1290».
fn house(c: &Chronicle, start_year: u32) -> String {
    let founder = c.rulers.first().map_or(String::new(), |r| r.full_name());
    format!(
        "{founder} и потомки · {start_year}–{}",
        start_year + c.years
    )
}

pub fn reign_end<'a>(d: &'a Data, cause: &'a str) -> &'a str {
    d.sim
        .texts
        .reign_ends
        .get(cause)
        .map_or(cause, String::as_str)
}

fn fall(c: &Chronicle, d: &Data) -> String {
    let text = d.sim.texts.falls.iter().find(|(f, _)| *f == c.fall);
    text.map_or(format!("{:?}", c.fall), |(_, t)| t.clone())
}

fn primary(text: &str) -> Button<'_> {
    Button::new(RichText::new(text).color(BG)).fill(FG)
}

/// A hint clause of `data/hints.ron` as a sentence, as the chronicle tells it.
fn sentence(h: &str) -> String {
    let mut chars = h.chars();
    let first = chars.next().into_iter().flat_map(char::to_uppercase);
    format!("{}.", first.chain(chars).collect::<String>())
}

/// `4 120`, `-35`: thousands apart by a narrow space.
fn thousands(v: i64) -> String {
    let digits = v.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{202f}');
        }
        out.push(ch);
    }
    match v < 0 {
        true => format!("-{out}"),
        false => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(thousands(4120), "4\u{202f}120");
        assert_eq!(thousands(-1234567), "-1\u{202f}234\u{202f}567");
        assert_eq!(thousands(35), "35");
        assert_eq!(sentence("знать помнила"), "Знать помнила.");
        let names: Vec<&str> = PART_NAMES.iter().map(|(k, _)| *k).collect();
        assert_eq!(names, bd_core::score::PARTS);
    }

    /// Stage 21: a symptom of the graph is «Знамение» in the chronicle; «Что решило судьбу»
    /// tells the founder's laws, kept to the end or repealed.
    #[test]
    fn omens_are_marked_and_the_founders_laws_told_with_their_fate() {
        use bd_core::sim::FallReason;
        use bd_core::time::Tick;
        let d = crate::load_data();
        let (_, p, m) = crate::PRESETS[0];
        let preset = bd_core::state::Preset::load_with_map(p, m, &d).unwrap();
        let mut g = Game::new(d, &preset, 1);
        // The founder brought in serfdom and the fairs in his 4th year; the preset's law of
        // succession is not his.
        for id in ["law_serfdom", "law_fairs"] {
            g.world.flags.insert(id.into());
            g.world.laws.insert(id.into(), Tick(3));
        }
        let kept = g.world.snapshot();
        let mut gone = kept.clone();
        gone.flags.remove("law_fairs");
        let entry = |tick, event: Option<&str>, w: &World| ChronicleEntry {
            tick: Tick(tick),
            event: event.map(Into::into),
            title: "Беглые".into(),
            text: String::new(),
            hint: None,
            importance: 0,
            causes: vec![],
            snapshot: w.clone(),
            chain: None,
        };
        let c = Chronicle {
            entries: vec![
                entry(40, Some("omen_runaways"), &kept),
                entry(60, Some("prov_crop_failure"), &gone),
            ],
            fall: FallReason::Alive,
            years: 300,
            rulers: vec![],
            kin: vec![],
            axes: Default::default(),
            deserted: 0,
            nodes: vec![],
            epilogue: String::new(),
            realms: Default::default(),
        };
        assert_eq!(title(&g.data, &c.entries[0]), "Знамение. Беглые");
        assert_eq!(title(&g.data, &c.entries[1]), "Беглые");
        let laws = founder_laws(&g, &c);
        assert_eq!(
            laws,
            [
                "1190  Крепостное право: основатель прикрепил крестьян к земле господ · до конца династии",
                "1190  Ярмарочное право: основатель освободил ярмарки от мыта баронов · отменён в 1247",
            ]
        );
    }
}
