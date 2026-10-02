//! egui front end: start, reign, event, chronicle, score. Every rule lives in `bd_core`; this
//! only shows and asks.

mod chronicle;
mod map;

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{EventView, Game, GameError, ReignEnd, Step};
use bd_core::link;
use bd_core::rng::Rng;
use bd_core::rules::{ActionTarget, Effect, Target};
use bd_core::score::{self, Score, ScoreRules};
use bd_core::sim::{self, Chronicle};
use bd_core::state::{AxisId, HeirStatus, Holder, NeighbourId, Preset, ProvinceId, World};
use eframe::egui::{self, Button, Grid, ProgressBar, RichText, Ui};
use map::{BG, BG2, FG, FG2, GOOD, MapView, RUBRIC, WARN, round};

// The web build has no file system, so the data ships inside the binary.
const RULES: &str = include_str!("../../../data/rules.ron");
const ACTIONS: &str = include_str!("../../../data/actions.ron");
const NAMES: &str = include_str!("../../../data/names.ron");
const HINTS: &str = include_str!("../../../data/hints.ron");
const SCORE: &str = include_str!("../../../data/score.ron");
/// Every top-level file of data/events, in file name order like the CLI.
const EVENTS: [&str; 5] = [
    include_str!("../../../data/events/death.ron"),
    include_str!("../../../data/events/heirs.ron"),
    include_str!("../../../data/events/neighbours.ron"),
    include_str!("../../../data/events/reign.ron"),
    include_str!("../../../data/events/war.ron"),
];
/// Every file of data/events/sim, in file name order.
const SIM_EVENTS: [&str; 1] = [include_str!("../../../data/events/sim/sim.ron")];
/// `(id, preset, map)`; the id is the file name and goes into game links.
const PRESETS: [(&str, &str, &str); 1] = [(
    "default",
    include_str!("../../../data/presets/default.ron"),
    include_str!("../../../data/maps/default.ron"),
)];
/// DejaVu Sans, Bitstream Vera license: assets/DejaVuSans-LICENSE.txt.
const FONT: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

enum Screen {
    Start,
    Reign,
    Event(EventView),
    /// The entry `App.entry` of the chronicle.
    Chronicle,
    Summary,
}

/// What a click asks for; `App::apply` carries it out.
#[derive(Debug, PartialEq)]
enum Cmd {
    Start(u64),
    Wait,
    /// An action that needs a target: show its targets.
    Pick(String, Vec<Target>),
    Cancel,
    Act(String, Option<Target>),
    Choose(usize),
    Abdicate,
    /// Show this entry of the chronicle.
    Entry(usize),
    Summary,
    /// The same seed and preset again.
    Restart,
    /// The link to this game into the clipboard; native also prints it.
    CopyLink,
    /// A new seed, drawn from the last one, and the same preset.
    NewSeed,
}

struct App {
    game: Option<Game>,
    screen: Screen,
    data: Data,
    presets: Vec<Preset>,
    preset: usize,
    seed: String,
    /// The seed of the game in play.
    played: u64,
    score_rules: ScoreRules,
    /// After the reign: the simulated dynasty and its score.
    dynasty: Option<(Chronicle, Score)>,
    /// The chronicle entry on screen.
    entry: usize,
    map: MapView,
    /// The action whose target is being chosen, with its targets.
    picking: Option<(String, Vec<Target>)>,
    /// The last refusal from the core.
    note: String,
    frame_ms: Option<f32>,
    /// The address bar holds a `#p=...` link; a new start clears it, so a reload does not
    /// bring the old game back.
    linked: bool,
}

fn load_data() -> Data {
    let mut d = bd_core::data::load(RULES).expect("rules.ron");
    EVENTS
        .iter()
        .for_each(|e| d.add_events(e).expect("data/events"));
    (SIM_EVENTS.iter()).for_each(|e| d.add_sim_events(e).expect("data/events/sim"));
    d.add_hints(HINTS).expect("hints.ron");
    d.add_actions(ACTIONS).expect("actions.ron");
    d.add_names(NAMES).expect("names.ron");
    d
}

impl App {
    fn new(ctx: &egui::Context) -> App {
        let mut fonts = egui::FontDefinitions::default();
        let font = egui::FontData::from_static(FONT);
        fonts
            .font_data
            .insert("dejavu".into(), std::sync::Arc::new(font));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "dejavu".into());
        }
        ctx.set_fonts(fonts);
        // The web build would follow the system theme; the palette is light only.
        ctx.set_theme(egui::Theme::Light);
        ctx.set_visuals(egui::Visuals {
            panel_fill: BG,
            window_fill: BG,
            ..egui::Visuals::light()
        });

        let data = load_data();
        let presets: Vec<Preset> = (PRESETS.iter())
            .map(|(_, p, m)| Preset::load_with_map(p, m, &data).expect("data/presets"))
            .collect();
        App {
            game: None,
            screen: Screen::Start,
            map: MapView::new(&presets[0].map.polygons),
            score_rules: score::load(SCORE, &data).expect("score.ron"),
            data,
            presets,
            preset: 0,
            seed: "1".into(),
            played: 1,
            dynasty: None,
            entry: 0,
            picking: None,
            note: String::new(),
            frame_ms: None,
            linked: false,
        }
    }

    fn apply(&mut self, cmd: Cmd) {
        self.note.clear();
        if matches!(cmd, Cmd::Start(_) | Cmd::Restart | Cmd::NewSeed) && self.linked {
            self.linked = false;
            #[cfg(target_arch = "wasm32")]
            if let Some(w) = eframe::web_sys::window() {
                let _ = w.location().set_hash("");
            }
        }
        match cmd {
            Cmd::Start(seed) => return self.start(seed),
            Cmd::Restart => return self.start(self.played),
            Cmd::NewSeed => {
                let seed = Rng::from_seed(self.played).next_u64() % 1_000_000;
                return self.start(seed);
            }
            Cmd::Entry(i) => {
                (self.entry, self.screen) = (i, Screen::Chronicle);
                return;
            }
            Cmd::Summary => {
                self.screen = Screen::Summary;
                return;
            }
            _ => {}
        }
        let g = self.game.as_mut().expect("only Start runs without a game");
        let res = match cmd {
            Cmd::Wait => {
                // A year of ticks, up to the first event.
                for _ in 1..g.data.time_unit.ticks_per_year {
                    match g.wait() {
                        Ok(Step::Idle) => {}
                        other => return self.step(other),
                    }
                }
                g.wait()
            }
            Cmd::Pick(id, targets) => {
                self.picking = Some((id, targets));
                return;
            }
            Cmd::Cancel => {
                self.picking = None;
                return;
            }
            Cmd::Act(id, target) => {
                self.picking = None;
                g.start_action(&id, target).map(|_| Step::Idle)
            }
            Cmd::Choose(idx) => match g.choose(idx) {
                // The choice may have ended the reign; `wait` reports it.
                Ok(()) if g.ended.is_some() => g.wait(),
                res => res.map(|_| Step::Idle),
            },
            Cmd::Abdicate => g.abdicate().and_then(|_| g.wait()),
            Cmd::Start(_)
            | Cmd::Restart
            | Cmd::NewSeed
            | Cmd::Entry(_)
            | Cmd::Summary
            | Cmd::CopyLink => {
                unreachable!("handled above")
            }
        };
        self.step(res);
    }

    /// Opens the game of a `#p=...` link, if `url` has one: at the reign, or at the score
    /// if the reign is over. A broken link leaves the start screen with a note.
    fn open(&mut self, url: &str) {
        let Some((_, text)) = url.split_once("#p=") else {
            return;
        };
        self.linked = true;
        if let Err(e) = self.replay(text) {
            (self.game, self.screen) = (None, Screen::Start);
            self.note = format!("Ссылка не открылась: {e}");
        }
    }

    fn replay(&mut self, text: &str) -> Result<(), String> {
        let l = link::decode(text)?;
        let preset = PRESETS.iter().position(|p| p.0 == l.preset_id);
        self.preset = preset.ok_or(format!("нет пресета {}", l.preset_id))?;
        self.start(l.seed);
        let g = self.game.as_mut().expect("just started");
        l.play(g)?;
        if let Some(cause) = g.ended.clone() {
            let end = ReignEnd {
                cause,
                tick: g.world.tick,
                world: g.world.snapshot(),
            };
            self.step(Ok(Step::ReignEnded(end)));
            self.screen = Screen::Summary;
        } else if g.pending_event.is_some() {
            let res = g.wait();
            self.step(res);
        }
        Ok(())
    }

    /// `#p=...` after the page address on the web, alone natively.
    fn link(&self) -> String {
        let g = self.game.as_ref().expect("in a game");
        let text = link::encode(PRESETS[self.preset].0, self.played, g);
        #[cfg(target_arch = "wasm32")]
        let page = (eframe::web_sys::window().and_then(|w| w.location().href().ok()))
            .map(|h| h.split('#').next().unwrap_or_default().to_string())
            .unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        let page = "";
        format!("{page}#p={text}")
    }

    fn start(&mut self, seed: u64) {
        let p = &self.presets[self.preset];
        self.map = MapView::new(&p.map.polygons);
        self.game = Some(Game::new(self.data.clone(), p, seed));
        (self.screen, self.picking, self.dynasty) = (Screen::Reign, None, None);
        (self.played, self.seed, self.entry) = (seed, seed.to_string(), 0);
    }

    fn step(&mut self, res: Result<Step, GameError>) {
        match res {
            Ok(Step::Idle) => self.screen = Screen::Reign,
            Ok(Step::Event(v)) => self.screen = Screen::Event(v),
            Ok(Step::ReignEnded(end)) => {
                // The dynasty goes on with the game's rng, as in the CLI.
                let g = self.game.as_ref().expect("a reign ended");
                let c = sim::run(end, &g.data, g.rng.clone());
                let s = score::compute(&c, &g.decisions, &self.score_rules);
                (self.dynasty, self.entry) = (Some((c, s)), 0);
                self.screen = Screen::Chronicle;
            }
            Err(e) => {
                self.note = match e {
                    GameError::NoSlot => "Все слоты действий заняты".into(),
                    GameError::Unavailable => "Действие сейчас недоступно".into(),
                    e => format!("Ошибка: {e:?}"),
                }
            }
        }
    }

    fn show(&mut self, ui: &mut Ui) {
        let cmd = match &self.screen {
            Screen::Start => self.start_screen(ui),
            Screen::Reign => self.reign(ui),
            Screen::Event(v) => {
                // The reign screen stays visible under the card; the modal blocks its clicks.
                self.reign(ui);
                event(ui.ctx(), self.game.as_ref().expect("in a game"), v)
            }
            Screen::Chronicle | Screen::Summary => {
                let g = self.game.as_ref().expect("in a game");
                let (c, s) = self.dynasty.as_ref().expect("after the reign");
                match self.screen {
                    Screen::Chronicle => chronicle::chronicle(ui, g, c, &self.map, self.entry),
                    _ => chronicle::summary(ui, g, (c, s), self.played, self.entry),
                }
            }
        };
        match cmd {
            Some(Cmd::CopyLink) => {
                let url = self.link();
                #[cfg(not(target_arch = "wasm32"))]
                println!("{url}");
                // eframe writes it with navigator.clipboard on the web.
                ui.ctx().copy_text(url);
                self.note = "Ссылка скопирована".into();
            }
            Some(cmd) => self.apply(cmd),
            None => {}
        }
    }

    fn start_screen(&mut self, ui: &mut Ui) -> Option<Cmd> {
        let mut cmd = None;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(RichText::new("Blessed Dynasty").size(32.0));
            ui.add_space(12.0);
            let label = |p: &Preset| format!("{}, {} год", p.ruler.name, p.start_year);
            egui::ComboBox::from_label("Пресет")
                .selected_text(label(&self.presets[self.preset]))
                .show_ui(ui, |ui| {
                    for (i, p) in self.presets.iter().enumerate() {
                        ui.selectable_value(&mut self.preset, i, label(p));
                    }
                });
            ui.horizontal(|ui| {
                ui.label("Seed");
                ui.text_edit_singleline(&mut self.seed);
            });
            let seed = self.seed.trim().parse().ok();
            if ui
                .add_enabled(seed.is_some(), Button::new("Начать"))
                .clicked()
            {
                cmd = seed.map(Cmd::Start);
            }
            if !self.note.is_empty() {
                ui.label(RichText::new(&self.note).color(RUBRIC));
            }
        });
        cmd
    }

    fn reign(&self, ui: &mut Ui) -> Option<Cmd> {
        let g = self.game.as_ref().expect("in a game");
        let mut cmd = None;
        egui::Panel::top("top").show(ui, |ui| ui.horizontal(|ui| self.top_bar(ui, g)));
        egui::Panel::bottom("actions").show(ui, |ui| cmd = self.actions(ui, g));
        egui::Panel::right("side").exact_size(300.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| side(ui, g))
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let targets = self.picking.as_ref().map_or(&[][..], |(_, t)| t);
            let marked: Vec<ProvinceId> = (targets.iter())
                .filter_map(|t| match t {
                    Target::Province(id) => Some(id.clone()),
                    _ => None,
                })
                .collect();
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                map::legend(ui);
                let clicked = self.map.show(ui, &g.world, &g.data, &marked);
                if let (Some(id), Some((action, _))) = (clicked, &self.picking)
                    && marked.contains(&id)
                {
                    cmd = Some(Cmd::Act(action.clone(), Some(Target::Province(id))));
                }
            });
        });
        cmd
    }

    fn top_bar(&self, ui: &mut Ui, g: &Game) {
        let (w, d) = (&g.world, &g.data);
        let unit = w.time_unit;
        ui.label(
            RichText::new(w.tick.date(unit, w.start_year))
                .size(22.0)
                .strong(),
        );
        let age = w.ruler.age;
        key(
            ui,
            "Правитель",
            &format!("{}, {age} {}", w.ruler.name, years(age)),
        );
        let reign = w.tick.0.saturating_sub(w.ruler.reign_start.0) / unit.ticks_per_year + 1;
        key(ui, "Правление", &format!("{reign}-й год"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(ms) = self.frame_ms {
                ui.small(RichText::new(format!("кадр {ms:.1} мс")).color(FG2));
            }
            // Right to left: value first, then its key.
            key_rtl(ui, "Провинций", &realm(w));
            if let Some(army) = w.axes.get(&d.war.army) {
                key_rtl(ui, axis_name(d, &d.war.army), &round(*army));
            }
            let income = d.economy.yearly_income(w);
            let arrow = if income >= Fx(0) { "▲" } else { "▼" };
            let treasury = round(w.axes[&d.economy.treasury]);
            key_rtl(
                ui,
                "Казна",
                &format!("{treasury} {arrow} {}/г", round(income)),
            );
        });
    }

    fn actions(&self, ui: &mut Ui, g: &Game) -> Option<Cmd> {
        let (w, d) = (&g.world, &g.data);
        let mut cmd = None;
        ui.add_space(4.0);
        match &self.picking {
            Some((id, targets)) => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "Цель для «{}», на карте или из списка:",
                        action_name(d, id)
                    ));
                    for t in targets {
                        if ui.button(target_name(w, t)).clicked() {
                            cmd = Some(Cmd::Act(id.clone(), Some(t.clone())));
                        }
                    }
                    if ui.button("Отмена").clicked() {
                        cmd = Some(Cmd::Cancel);
                    }
                });
            }
            None => {
                ui.horizontal_wrapped(|ui| {
                    for (id, targets) in g.available_actions() {
                        if ui.button(action_name(d, &id)).clicked() {
                            cmd = Some(match targets.is_empty() {
                                true => Cmd::Act(id, None),
                                false => Cmd::Pick(id, targets),
                            });
                        }
                    }
                    let abdicate = Button::new(RichText::new("Отречься").color(RUBRIC));
                    if ui.add(abdicate.stroke((1.0, RUBRIC))).clicked() {
                        cmd = Some(Cmd::Abdicate);
                    }
                });
            }
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(running(g)).color(FG2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let wait = Button::new(RichText::new("Подождать год ▸").color(BG)).fill(FG);
                if ui.add(wait).clicked() {
                    cmd = Some(Cmd::Wait);
                }
                if ui.button("Скопировать ссылку").clicked() {
                    cmd = Some(Cmd::CopyLink);
                }
            });
        });
        if !self.note.is_empty() {
            ui.label(RichText::new(&self.note).color(RUBRIC));
        }
        ui.add_space(4.0);
        cmd
    }
}

/// «Действия k из n: идёт X, t из T лет».
fn running(g: &Game) -> String {
    let (w, d) = (&g.world, &g.data);
    let tpy = w.time_unit.ticks_per_year as f32;
    let running: Vec<String> = (w.active_actions.iter())
        .map(|a| {
            let def = d
                .actions
                .iter()
                .find(|x| x.id == a.id)
                .expect("only known actions run");
            let target = (a.target.clone()).map(|key| match def.target {
                ActionTarget::Province(_) => Target::Province(ProvinceId(key)),
                ActionTarget::Neighbour => Target::Neighbour(NeighbourId(key)),
                ActionTarget::Heir | ActionTarget::None => {
                    Target::Heir(key.parse().unwrap_or(u32::MAX))
                }
            });
            let name = match target {
                Some(t) => format!("{} ({})", def.name, target_name(w, &t)),
                None => def.name.clone(),
            };
            let total = def.duration_years.0 as f32;
            let done = total - a.ends_at.0.saturating_sub(w.tick.0) as f32 / tpy;
            format!("{name}, {} из {} лет", num(done), num(total))
        })
        .collect();
    let line = format!("Действия {} из {}", running.len(), d.action_slots.slots(w));
    match running.is_empty() {
        true => line,
        false => format!("{line}: идёт {}", running.join("; ")),
    }
}

/// The modal card over the dimmed reign screen.
fn event(ctx: &egui::Context, g: &Game, v: &EventView) -> Option<Cmd> {
    let w = &g.world;
    let mut cmd = None;
    egui::Modal::new(egui::Id::new("event")).show(ctx, |ui| {
        ui.set_width(540.0);
        let mut eyebrow = vec!["Событие".to_string()];
        eyebrow.extend(v.target.as_ref().map(|t| target_name(w, t)));
        eyebrow.push(w.tick.date(w.time_unit, w.start_year));
        ui.label(RichText::new(eyebrow.join(" · ")).small().color(RUBRIC));
        ui.label(RichText::new(&v.title).size(20.0).strong());
        ui.label(&v.text);
        ui.add_space(6.0);
        let mut hint = None;
        for (i, c) in v.choices.iter().enumerate() {
            ui.horizontal(|ui| {
                let r = ui.add(
                    Button::new(&c.text)
                        .wrap()
                        .min_size(egui::vec2(360.0, 28.0)),
                );
                ui.vertical(|ui| {
                    for (text, up) in effects(&g.data, &c.effects) {
                        let color = match up {
                            Some(true) => GOOD,
                            Some(false) => RUBRIC,
                            None => FG2,
                        };
                        ui.small(RichText::new(text).color(color));
                    }
                });
                if r.clicked() {
                    cmd = Some(Cmd::Choose(i));
                }
                if r.hovered() {
                    hint = c.hint.as_deref();
                }
            });
        }
        ui.add_space(6.0);
        let hint = hint.unwrap_or("Наведите на вариант, чтобы увидеть намёк.");
        ui.label(RichText::new(hint).small().color(FG2));
    });
    cmd
}

/// Ruler, axes, heirs, neighbours.
fn side(ui: &mut Ui, g: &Game) {
    let (w, d) = (&g.world, &g.data);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(44.0, 52.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 0.0, BG2);
        let initial = w.ruler.name.chars().next().unwrap_or('?').to_string();
        let font = egui::FontId::proportional(22.0);
        ui.painter()
            .text(r.center(), egui::Align2::CENTER_CENTER, initial, font, FG2);
        ui.vertical(|ui| {
            ui.strong(&w.ruler.name);
            let health = round(w.ruler.health);
            Grid::new("health").show(ui, |ui| {
                bar(
                    ui,
                    "Здоровье",
                    w.ruler.health,
                    Fx(0),
                    Fx::from_int(100),
                    &health,
                )
            });
            ui.small(format!("{} {}", w.ruler.age, years(w.ruler.age)));
        });
    });
    heading(ui, "Состояние");
    Grid::new("axes").show(ui, |ui| {
        for a in &d.axes {
            let v = w.axes[&a.id];
            bar(ui, axis_name(d, &a.id), v, a.min, a.max, &round(v));
        }
    });
    heading(ui, "Наследники");
    if w.heirs.is_empty() {
        ui.label("нет");
    }
    Grid::new("heirs").show(ui, |ui| {
        for (i, h) in w.heirs.iter().enumerate() {
            let status = match &h.status {
                HeirStatus::Home if i == 0 => RichText::new("наследник").color(FG2),
                HeirStatus::Home => RichText::new(""),
                HeirStatus::Studying(place) => RichText::new(format!("учится: {place}")).color(FG2),
                HeirStatus::Hostage(n) => {
                    let n = w.neighbours.get(n).map_or(&n.0, |n| &n.name);
                    RichText::new(format!("заложник: {n}")).color(RUBRIC)
                }
            };
            ui.label(format!("{}, {}", h.name, h.age));
            ui.small(status);
            ui.small(format!(
                "спос. {} · прет. {}",
                round(h.ability),
                round(h.claim)
            ));
            ui.end_row();
        }
    });
    heading(ui, "Соседи");
    Grid::new("neighbours").show(ui, |ui| {
        for n in w.neighbours.values() {
            let sign = if n.relation > Fx(0) { "+" } else { "" };
            let value = format!("{sign}{}", round(n.relation));
            let (lo, hi) = (Fx::from_int(-100), Fx::from_int(100));
            bar(ui, &n.name, n.relation, lo, hi, &value);
        }
    });
}

pub(crate) fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text.to_uppercase()).small().color(FG2));
}

fn key(ui: &mut Ui, k: &str, v: &str) {
    ui.label(RichText::new(k).color(FG2));
    ui.label(RichText::new(v).color(FG));
}

fn key_rtl(ui: &mut Ui, k: &str, v: &str) {
    ui.label(RichText::new(v).color(FG));
    ui.label(RichText::new(k).color(FG2));
}

/// A grid row with a labelled bar; the colour goes from bad to good with the share of the range.
fn bar(ui: &mut Ui, label: &str, v: Fx, min: Fx, max: Fx, value: &str) {
    let share = ((v - min).0 as f32 / (max - min).0.max(1) as f32).clamp(0.0, 1.0);
    let color = match share {
        s if s < 0.35 => RUBRIC,
        s if s < 0.55 => WARN,
        _ => GOOD,
    };
    ui.small(label);
    let bar = ProgressBar::new(share).fill(color).desired_width(110.0);
    ui.add(bar.desired_height(6.0));
    ui.small(value);
    ui.end_row();
}

/// The axis name from rules.ron, or its id.
pub(crate) fn axis_name<'a>(d: &'a Data, id: &'a AxisId) -> &'a str {
    let def = d.axes.iter().find(|a| a.id == *id);
    def.filter(|a| !a.name.is_empty())
        .map_or(&id.0, |a| &a.name)
}

/// «10 · короне 6»: provinces of the crown and its vassals, then of the crown alone.
pub(crate) fn realm(w: &World) -> String {
    let own = (w.provinces.values()).filter(|p| !matches!(p.holder, Holder::Foreign(_)));
    let crown = own.clone().filter(|p| p.holder == Holder::Crown).count();
    format!("{} · короне {crown}", own.count())
}

fn action_name<'a>(d: &'a Data, id: &'a str) -> &'a str {
    d.actions
        .iter()
        .find(|a| a.id == id)
        .map_or(id, |a| &a.name)
}

fn target_name(w: &World, t: &Target) -> String {
    let name = match t {
        Target::Province(id) => w.provinces.get(id).map(|p| &p.name),
        Target::Neighbour(id) => w.neighbours.get(id).map(|n| &n.name),
        Target::Heir(i) => w.heirs.get(*i as usize).map(|h| &h.name),
    };
    name.cloned().unwrap_or_else(|| format!("{t:?}"))
}

/// Axis effects of a choice with their sign, `("Знать +15", Some(true))`, then without
/// numbers «риск» for a chance and «провинция» for a province effect (`None`: no sign).
/// Other effects stay hidden.
fn effects(d: &Data, effects: &[Effect]) -> Vec<(String, Option<bool>)> {
    let axes = effects.iter().filter_map(|e| match e {
        Effect::Axis(a, v) => Some((a, *v)),
        _ => None,
    });
    let mut out: Vec<_> = axes
        .map(|(a, v)| {
            let sign = if v > Fx(0) { "+" } else { "" };
            (format!("{} {sign}{v}", axis_name(d, a)), Some(v > Fx(0)))
        })
        .collect();
    if effects.iter().any(|e| matches!(e, Effect::Chance(_))) {
        out.push(("риск".into(), None));
    }
    let province = |e: &Effect| {
        matches!(
            e,
            Effect::Province(..)
                | Effect::CrownPower(..)
                | Effect::Build(..)
                | Effect::Grant(_)
                | Effect::Revoke(_)
                | Effect::TransferProvince(..)
                | Effect::Secede(_)
        )
    };
    if effects.iter().any(province) {
        out.push(("провинция".into(), None));
    }
    out
}

/// `1 год`, `3 года`, `5 лет`, `21 год`, `11 лет`.
fn years(n: u32) -> &'static str {
    plural(n, ["год", "года", "лет"])
}

/// The Russian form of a word by `n`: `[1, 2..4, 5..]`, «21 правитель», «3 правителя».
pub(crate) fn plural(n: u32, [one, few, many]: [&str; 3]) -> &str {
    match (n % 10, n % 100) {
        (_, 11..=14) => many,
        (1, _) => one,
        (2..=4, _) => few,
        _ => many,
    }
}

/// `2`, `2,5`: years with one decimal, Russian style.
fn num(v: f32) -> String {
    let s = format!("{v:.1}");
    s.trim_end_matches(".0").replace('.', ",")
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        self.frame_ms = frame.info().cpu_usage.map(|s| s * 1000.0);
        self.show(ui);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    // A link as the first argument, the whole URL or its `#p=...`.
    let url = std::env::args().nth(1).unwrap_or_default();
    eframe::run_native(
        "Blessed Dynasty",
        eframe::NativeOptions::default(),
        Box::new(move |cc| {
            let mut app = App::new(&cc.egui_ctx);
            app.open(&url);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast;
    let document = eframe::web_sys::window()
        .and_then(|w| w.document())
        .expect("a document");
    let canvas = document
        .get_element_by_id("canvas")
        .expect("index.html has #canvas");
    let canvas = canvas
        .dyn_into::<eframe::web_sys::HtmlCanvasElement>()
        .expect("a canvas");
    let url = eframe::web_sys::window()
        .and_then(|w| w.location().hash().ok())
        .unwrap_or_default();
    wasm_bindgen_futures::spawn_local(async move {
        let app: eframe::AppCreator = Box::new(move |cc| {
            let mut app = App::new(&cc.egui_ctx);
            app.open(&url);
            Ok(Box::new(app))
        });
        let options = eframe::WebOptions::default();
        let runner = eframe::WebRunner::new();
        runner
            .start(canvas, options, app)
            .await
            .expect("eframe starts");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, RawInput, Rect, vec2};

    /// The app in a headless egui context: frames go through the same `show` as on screen.
    struct Harness {
        ctx: egui::Context,
        app: App,
    }

    impl Harness {
        fn new() -> Harness {
            let ctx = egui::Context::default();
            let app = App::new(&ctx);
            let mut h = Harness { ctx, app };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                events,
                ..Default::default()
            };
            let app = &mut self.app;
            let mut out = self.ctx.run_ui(input, |ui| app.show(ui));
            // No GPU here to upload textures to.
            out.textures_delta.clear();
            out
        }

        /// Press and release of the left button at `pos`, as the mouse does it; the output of
        /// the release.
        fn click(&mut self, pos: Pos2) -> egui::FullOutput {
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)])
        }

        fn game(&self) -> &Game {
            self.app.game.as_ref().unwrap()
        }

        /// Clicks the widget with this label, found by its accessibility node.
        fn click_label(&mut self, label: &str) -> egui::FullOutput {
            self.ctx.enable_accesskit();
            let out = self.frame(vec![]);
            let tree = out
                .platform_output
                .accesskit_update
                .expect("accesskit is on");
            let node = tree.nodes.iter().find(|(_, n)| n.label() == Some(label));
            let b = node.and_then(|(_, n)| n.bounds());
            let b = b.unwrap_or_else(|| panic!("no «{label}» on screen"));
            let centre = Pos2::new((b.x0 + b.x1) as f32 / 2.0, (b.y0 + b.y1) as f32 / 2.0);
            self.click(centre)
        }

        fn province_on_screen(&self, id: &str) -> Pos2 {
            let map = &self.app.map;
            map.to_screen(map.centre(&ProvinceId(id.into())).unwrap())
        }
    }

    /// Plays a reign from `seed` to its end: an action now and then (by the map for province
    /// targets), choices by the year. Returns the number of events.
    fn play(h: &mut Harness, seed: u64) -> usize {
        h.app.apply(Cmd::Start(seed));
        let mut events = 0;
        for year in 0..200 {
            h.frame(vec![]);
            match &h.app.screen {
                Screen::Reign => {
                    let first = h.game().available_actions().into_iter().next();
                    if year % 5 == 0
                        && let Some((id, targets)) = first
                    {
                        let province = match targets.first() {
                            Some(Target::Province(p)) => Some(p.0.clone()),
                            _ => None,
                        };
                        match province {
                            Some(p) => {
                                h.app.apply(Cmd::Pick(id, targets));
                                h.frame(vec![]);
                                h.click(h.province_on_screen(&p));
                            }
                            None => h.app.apply(Cmd::Act(id, targets.first().cloned())),
                        }
                    }
                    h.app.apply(Cmd::Wait);
                }
                Screen::Event(v) => {
                    events += 1;
                    let last = v.choices.len() - 1;
                    h.app.apply(Cmd::Choose(year % (last + 1)));
                }
                Screen::Chronicle => return events,
                Screen::Start | Screen::Summary => unreachable!(),
            }
            assert!(
                h.app.note.is_empty() || h.app.note.contains("слоты"),
                "{}",
                h.app.note
            );
        }
        panic!("the ruler outlives 200 years")
    }

    /// Start, reign, chronicle, score and the same start again, by the buttons on screen.
    #[test]
    fn a_game_runs_from_start_to_the_score_and_again() {
        let mut h = Harness::new();
        assert!(play(&mut h, 7) > 0);
        assert!(!h.game().decisions.is_empty());
        let (c, s) = h.app.dynasty.clone().expect("simulated at the reign end");
        assert!(!c.entries.is_empty() && h.app.entry == 0);
        // The CLI's way: the game's rng goes on into the simulation.
        let g = h.game();
        let end = bd_core::game::ReignEnd {
            cause: g.ended.clone().unwrap(),
            tick: g.world.tick,
            world: g.world.snapshot(),
        };
        let want = sim::run(end, &g.data, g.rng.clone());
        assert_eq!(c, want);
        assert_eq!(s, score::compute(&c, &g.decisions, &h.app.score_rules));

        // A click on the list selects the entry.
        let e = &c.entries[1];
        let date = e
            .tick
            .date(h.game().world.time_unit, h.game().world.start_year);
        h.click_label(&format!("{date}  {}", e.title));
        assert!(matches!(h.app.screen, Screen::Chronicle) && h.app.entry == 1);
        let last = c.entries.len() - 1;
        h.app.apply(Cmd::Entry(last));
        h.click_label("К итогу ▸");
        assert!(matches!(h.app.screen, Screen::Summary));
        h.click_label("◂ К хронике");
        assert!(matches!(h.app.screen, Screen::Chronicle) && h.app.entry == last);
        h.click_label("К итогу ▸");
        h.click_label("Тот же старт, заново");
        assert!(matches!(h.app.screen, Screen::Reign));
        assert!(h.app.dynasty.is_none() && h.app.played == 7);
        assert_eq!(h.game().world.tick.0, 0);
        assert!(h.game().decisions.is_empty());

        play(&mut h, 7);
        h.click_label("К итогу ▸");
        h.click_label("Новый seed");
        let seed = Rng::from_seed(7).next_u64() % 1_000_000;
        assert_eq!((h.app.played, h.app.seed.clone()), (seed, seed.to_string()));
        assert!(matches!(h.app.screen, Screen::Reign) && h.game().world.tick.0 == 0);
    }

    /// The link the button copies.
    fn copy_link(h: &mut Harness) -> String {
        let out = h.click_label("Скопировать ссылку");
        let copied = out.platform_output.commands.iter().find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        });
        assert_eq!(h.app.note, "Ссылка скопирована");
        copied.expect("the button copies the link")
    }

    /// A link opened by another app, from the embedded data alone, gives the same chronicle
    /// and score, on the score screen.
    #[test]
    fn a_link_shows_the_same_chronicle_and_score_elsewhere() {
        let mut h = Harness::new();
        play(&mut h, 7);
        h.click_label("К итогу ▸");
        let url = copy_link(&mut h);
        assert!(url.starts_with("#p="), "{url}");
        let mut other = Harness::new();
        other.app.open(&format!("https://example.org/bd/{url}"));
        assert!(
            matches!(other.app.screen, Screen::Summary),
            "{}",
            other.app.note
        );
        assert_eq!(other.app.dynasty, h.app.dynasty);
        assert_eq!(other.game().decisions, h.game().decisions);
        assert_eq!(other.app.played, 7);
        other.frame(vec![]);
        // A new start drops the link from the address bar.
        assert!(other.app.linked);
        other.click_label("Тот же старт, заново");
        assert!(!other.app.linked && !h.app.linked);
    }

    /// From the reign screen the link reopens the same reign, years waited included.
    #[test]
    fn a_link_to_a_reign_in_progress_opens_the_reign() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(2));
        for _ in 0..6 {
            if let Screen::Event(_) = h.app.screen {
                h.app.apply(Cmd::Choose(0));
            }
            h.app.apply(Cmd::Wait);
        }
        if let Screen::Event(_) = h.app.screen {
            h.app.apply(Cmd::Choose(1));
        }
        h.frame(vec![]);
        let url = copy_link(&mut h);
        let mut other = Harness::new();
        other.app.open(&url);
        assert!(matches!(other.app.screen, Screen::Reign));
        assert_eq!(other.game().world, h.game().world);
        assert!(other.game().world.tick.0 >= 6);
        assert!(other.app.dynasty.is_none());

        // A broken link stays on the start screen and says so; no link, nothing happens.
        let mut broken = Harness::new();
        broken.app.open("#p=AAAA");
        assert!(matches!(broken.app.screen, Screen::Start) && broken.app.game.is_none());
        assert!(broken.app.note.starts_with("Ссылка не открылась"));
        broken.frame(vec![]);
        broken.app.open("https://example.org/");
        assert!(matches!(broken.app.screen, Screen::Start));
        assert!(broken.app.linked);
        broken.app.apply(Cmd::Start(3));
        assert!(!broken.app.linked);
    }

    #[test]
    fn the_same_seed_and_choices_give_the_same_chronicle() {
        let mut h = Harness::new();
        play(&mut h, 3);
        let first = h.app.dynasty.clone().unwrap();
        h.app.apply(Cmd::Restart);
        play(&mut h, 3);
        assert_eq!(h.app.dynasty.clone().unwrap(), first);
        play(&mut h, 4);
        assert_ne!(h.app.dynasty.clone().unwrap().0, first.0);
    }

    /// Vertex colours of the meshes painted: the map is the only mesh on the screen.
    fn mesh_colors(out: &egui::FullOutput) -> Vec<egui::Color32> {
        (out.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Mesh(m) => Some(m.vertices.iter().map(|v| v.color)),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn another_entry_repaints_the_map_from_its_snapshot() {
        let mut h = Harness::new();
        let holders = |c: &Chronicle, i: usize| {
            let w = &c.entries[i].snapshot;
            w.provinces
                .values()
                .map(|p| p.holder.clone())
                .collect::<Vec<_>>()
        };
        // The first seed whose chronicle moves a province.
        let other = (1..20)
            .find_map(|seed| {
                play(&mut h, seed);
                let c = &h.app.dynasty.as_ref().unwrap().0;
                (1..c.entries.len()).find(|&i| holders(c, i) != holders(c, 0))
            })
            .expect("some realm changes over its chronicle");
        h.app.apply(Cmd::Entry(0));
        let before = mesh_colors(&h.frame(vec![]));
        h.app.apply(Cmd::Entry(other));
        let after = mesh_colors(&h.frame(vec![]));
        assert!(!before.is_empty());
        assert_ne!(before, after);
        h.app.apply(Cmd::Entry(0));
        assert_eq!(mesh_colors(&h.frame(vec![])), before);
    }

    #[test]
    fn crownings_split_the_chronicle_into_reigns() {
        let mut h = Harness::new();
        play(&mut h, 7);
        let c = h.app.dynasty.clone().unwrap().0;
        let reigns = chronicle::reigns(&c, &h.app.data);
        let crowned: Vec<usize> = (reigns.iter())
            .filter(|(_, c)| *c)
            .map(|(r, _)| *r)
            .collect();
        // Every ruler after the founder is crowned once, in order.
        assert_eq!(crowned, (1..c.rulers.len()).collect::<Vec<_>>());
        assert!(c.rulers.len() > 1);
        for ((r, _), e) in reigns.iter().zip(&c.entries) {
            assert!(c.rulers[*r].start <= e.tick, "{e:?}");
        }
        let founder = c.rulers[0].cause.as_deref().unwrap();
        assert_ne!(chronicle::reign_end(&h.app.data, founder), founder);
    }

    #[test]
    fn abdication_ends_the_reign_through_its_event() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.app.apply(Cmd::Abdicate);
        let Screen::Event(v) = &h.app.screen else {
            panic!("no abdication card")
        };
        assert_eq!(v.event_id, h.app.data.abdication.event);
        let id = v.event_id.clone();
        h.frame(vec![]);
        // The confirming choice is the one with the Abdicate effect.
        let ev = h.app.data.events.iter().find(|e| e.id == id).unwrap();
        let confirm = ev
            .choices
            .iter()
            .position(|c| c.effects.contains(&Effect::Abdicate))
            .unwrap();
        h.app.apply(Cmd::Choose(confirm));
        assert!(matches!(h.app.screen, Screen::Chronicle));
        let c = &h.app.dynasty.as_ref().unwrap().0;
        assert_eq!(c.rulers[0].cause.as_deref(), Some(id.as_str()));
        assert_eq!(chronicle::reign_end(&h.app.data, &id), "отречение");
        h.frame(vec![]);
    }

    #[test]
    fn map_click_picks_the_target() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        let (id, targets) = (h.game().available_actions().into_iter())
            .find(|(id, _)| id == "build_fort")
            .unwrap();
        assert!(targets.contains(&Target::Province(ProvinceId("berg".into()))));
        h.app.apply(Cmd::Pick(id, targets));
        h.frame(vec![]);
        // A foreign province is not a target: nothing starts.
        h.click(h.province_on_screen("nordheim"));
        assert!(h.game().world.active_actions.is_empty() && h.app.picking.is_some());
        h.click(h.province_on_screen("berg"));
        let running = &h.game().world.active_actions;
        assert_eq!(running.len(), 1);
        assert_eq!(
            (running[0].id.as_str(), running[0].target.as_deref()),
            ("build_fort", Some("berg"))
        );
        assert!(h.app.picking.is_none());
        assert!(
            running_line(&h).starts_with("Действия 1 из 1: идёт Построить крепость (Берг), 0 из")
        );
    }

    fn running_line(h: &Harness) -> String {
        running(h.game())
    }

    #[test]
    fn refusals_from_the_core_are_shown() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.app.apply(Cmd::Act("royal_progress".into(), None));
        // One slot at the start bureaucracy: the second action is refused by the core.
        h.app.apply(Cmd::Act(
            "build_road".into(),
            Some(Target::Province(ProvinceId("berg".into()))),
        ));
        assert_eq!(h.app.note, "Все слоты действий заняты");
        assert_eq!(h.game().world.active_actions.len(), 1);
        h.app.apply(Cmd::Wait);
        assert!(h.app.note.is_empty());
    }

    #[test]
    fn frame_with_the_map_fits_16_ms() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.app.apply(Cmd::Act(
            "build_road".into(),
            Some(Target::Province(ProvinceId("berg".into()))),
        ));
        let (id, targets) = h
            .game()
            .available_actions()
            .into_iter()
            .find(|(_, t)| t.len() > 2)
            .unwrap();
        h.app.apply(Cmd::Pick(id, targets));
        h.frame(vec![]);
        let n = 30;
        let t = std::time::Instant::now();
        for i in 0..n {
            // Hover moves over the map, so the tooltip and the hit test run too.
            let out = h.frame(vec![Event::PointerMoved(Pos2::new(
                200.0 + i as f32 * 10.0,
                300.0,
            ))]);
            let _ = h.ctx.tessellate(out.shapes, out.pixels_per_point);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        eprintln!("reign frame: {ms:.2} ms");
        assert!(ms < 16.0, "{ms} ms");
    }

    #[test]
    fn choice_effects_show_axes_with_sign_risk_and_province() {
        let d = load_data();
        let ax = |s: &str| AxisId(s.into());
        let here = || bd_core::rules::ProvinceTarget::EventTarget;
        let chance = bd_core::rules::Chance {
            percent: Fx::from_int(50),
            axes: vec![],
            bonus: vec![],
            then: vec![Effect::RulerDies("illness".into())],
            otherwise: vec![],
        };
        let shown = effects(
            &d,
            &[
                Effect::Axis(ax("loyalty_nobles"), Fx::from_int(15)),
                Effect::RulerHealth(Fx::from_int(5)),
                Effect::Axis(ax("treasury"), Fx::from_int(-360)),
                Effect::Axis(ax("no_name"), Fx(1500)),
                Effect::Chance(chance.clone()),
                Effect::Chance(chance),
                Effect::Grant(here()),
                Effect::Province(here(), bd_core::rules::ProvinceField::Loyalty, Fx(1)),
            ],
        );
        let want = [
            ("Знать +15", Some(true)),
            ("Казна -360", Some(false)),
            ("no_name +1.5", Some(true)),
            ("риск", None),
            ("провинция", None),
        ];
        assert_eq!(shown, want.map(|(s, up)| (s.to_string(), up)));
        assert!(effects(&d, &[Effect::RulerHealth(Fx(1))]).is_empty());
    }

    /// At the start no own province is in unrest; the threshold is its own, below the crown
    /// power one.
    #[test]
    fn no_unrest_at_the_start() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        let (w, d) = (&h.game().world, &h.game().data);
        let unrest = (w.provinces.values())
            .filter(|p| !matches!(p.holder, Holder::Foreign(_)))
            .filter(|p| p.loyalty < d.unrest_below)
            .count();
        assert_eq!(unrest, 0);
        assert!(d.unrest_below > Fx(0) && d.unrest_below < d.crown_power.loyalty_threshold);
        assert_eq!(axis_name(d, &d.war.army), "Армия");
        assert_eq!(axis_name(d, &AxisId("no_name".into())), "no_name");
    }

    #[test]
    fn every_event_file_is_embedded() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/events");
        let texts = |dir: &str| {
            let mut files: Vec<_> = (std::fs::read_dir(dir).unwrap())
                .map(|e| e.unwrap().path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "ron"))
                .collect();
            files.sort();
            let texts: Vec<String> = (files.iter())
                .map(|p| std::fs::read_to_string(p).unwrap())
                .collect();
            (texts, files)
        };
        let (events, files) = texts(dir);
        assert_eq!(events, EVENTS, "EVENTS must list {files:?} in this order");
        let (sim, files) = texts(&format!("{dir}/sim"));
        assert_eq!(
            sim, SIM_EVENTS,
            "SIM_EVENTS must list {files:?} in this order"
        );
    }

    #[test]
    fn russian_numbers() {
        let y = |n| format!("{n} {}", years(n));
        let got: Vec<String> = [1, 3, 5, 11, 12, 21, 41, 43, 112]
            .into_iter()
            .map(y)
            .collect();
        let want = [
            "1 год",
            "3 года",
            "5 лет",
            "11 лет",
            "12 лет",
            "21 год",
            "41 год",
            "43 года",
            "112 лет",
        ];
        assert_eq!(got, want);
        let rulers = |n| plural(n, ["правитель", "правителя", "правителей"]);
        assert_eq!(
            [1, 3, 12, 21].map(rulers),
            ["правитель", "правителя", "правителей", "правитель"]
        );
        assert_eq!(
            (num(2.0), num(2.5), num(0.0)),
            ("2".into(), "2,5".into(), "0".into())
        );
    }

    /// Slow: builds the release wasm. `cargo test -p ui -- --ignored`.
    /// Counts what the browser loads: custom sections (names, wasm-bindgen's own) are
    /// stripped by wasm-bindgen in `trunk build --release`.
    #[test]
    #[ignore]
    fn wasm_release_fits_8_mb() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let dir = format!("{root}/target/wasm-size");
        let cargo = std::env::var("CARGO").unwrap_or("cargo".into());
        let args = [
            "build",
            "-p",
            "ui",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--target-dir",
            &dir,
        ];
        let status = std::process::Command::new(cargo)
            .args(args)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success());
        let wasm = format!("{dir}/wasm32-unknown-unknown/release/ui.wasm");
        let bytes = std::fs::read(wasm).unwrap();
        // Sections after the 8-byte header: id, LEB128 length, payload; id 0 is custom.
        let (mut i, mut size) = (8, 0);
        while i < bytes.len() {
            let id = bytes[i];
            let (mut len, mut shift) = (0usize, 0);
            loop {
                i += 1;
                len |= ((bytes[i] & 0x7f) as usize) << shift;
                shift += 7;
                if bytes[i] < 0x80 {
                    break;
                }
            }
            i += 1 + len;
            size += if id == 0 { 0 } else { len };
        }
        eprintln!(
            "ui.wasm: {} bytes, {size} without custom sections",
            bytes.len()
        );
        assert!(size <= 8 * 1024 * 1024, "{size} bytes");
    }
}
