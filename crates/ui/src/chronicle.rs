//! The dynasty after the founder: the chronicle with its map snapshots, then the score.

use crate::map::{BG, BG2, FG, FG2, GOOD, MapView, RUBRIC, round};
use crate::{Cmd, axis_name, heading, plural, realm};
use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::Game;
use bd_core::score::Score;
use bd_core::sim::{Chronicle, ChronicleEntry};
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
                        c.rulers[r - 1].name,
                        c.rulers[r].name
                    ))
                    .strong()
                    .color(FG),
                    false => RichText::new(format!("{date}  {}", e.title)).color(FG2),
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
        egui::CentralPanel::default().show(ui, |ui| ui.label(fall(c, d)));
        return cmd;
    };
    egui::Panel::right("snapshot")
        .exact_size(320.0)
        .show(ui, |ui| {
            ui.allocate_ui(vec2(ui.available_width(), 260.0), |ui| {
                map.show(ui, &e.snapshot, d, &[]);
            });
            snapshot_table(ui, e, d);
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
                ruler.name
            ))
            .small()
            .color(FG2),
        );
        ui.add_space(10.0);
        ui.label(RichText::new(&e.title).size(18.0).strong());
        ui.label(&e.text);
        if crowns {
            let prev = &c.rulers[r - 1];
            let cause = prev.cause.as_deref().map_or("", |c| reign_end(d, c));
            let years = |t: bd_core::time::Tick| t.date(w.time_unit, w.start_year);
            ui.label(
                RichText::new(format!(
                    "{}, {}–{}, конец правления: {cause}",
                    prev.name,
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
    });
    cmd
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
        for a in &d.axes {
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
    });
    cmd
}

/// «Ульрих и потомки · 1187–1290».
fn house(c: &Chronicle, start_year: u32) -> String {
    let founder = c.rulers.first().map_or("", |r| &r.name);
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
}
