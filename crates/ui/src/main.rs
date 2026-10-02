//! egui front end: start, reign, event. Every rule lives in `bd_core`; this only shows and asks.

mod map;

use bd_core::data::Data;
use bd_core::fx::Fx;
use bd_core::game::{EventView, Game, GameError, Step};
use bd_core::rules::{ActionTarget, Effect, Target};
use bd_core::state::{AxisId, HeirStatus, Holder, NeighbourId, Preset, ProvinceId, World};
use eframe::egui::{self, Button, Grid, ProgressBar, RichText, Ui};
use map::{BG, BG2, FG, FG2, GOOD, MapView, RUBRIC, WARN, round};

// The web build has no file system, so the data ships inside the binary.
const RULES: &str = include_str!("../../../data/rules.ron");
const ACTIONS: &str = include_str!("../../../data/actions.ron");
const NAMES: &str = include_str!("../../../data/names.ron");
/// Every top-level file of data/events, in file name order like the CLI.
const EVENTS: [&str; 5] = [
    include_str!("../../../data/events/death.ron"),
    include_str!("../../../data/events/heirs.ron"),
    include_str!("../../../data/events/neighbours.ron"),
    include_str!("../../../data/events/reign.ron"),
    include_str!("../../../data/events/war.ron"),
];
/// `(preset, map)`.
const PRESETS: [(&str, &str); 1] = [(
    include_str!("../../../data/presets/default.ron"),
    include_str!("../../../data/maps/default.ron"),
)];
/// DejaVu Sans, Bitstream Vera license: assets/DejaVuSans-LICENSE.txt.
const FONT: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

/// Display names of the axes of rules.ron; an unlisted axis shows its id.
const AXIS_NAMES: [(&str, &str); 11] = [
    ("treasury", "Казна"),
    ("income", "Доход"),
    ("army", "Армия"),
    ("legitimacy", "Легитимность"),
    ("stability", "Стабильность"),
    ("bureaucracy", "Бюрократия"),
    ("loyalty_nobles", "Знать"),
    ("loyalty_church", "Церковь"),
    ("loyalty_people", "Народ"),
    ("prestige", "Престиж"),
    ("loyalty", "Лояльность"),
];
/// The axis shown in the top bar next to the treasury.
const ARMY: &str = "army";

enum Screen {
    Start,
    Reign,
    Event(EventView),
    ReignEnded,
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
    Again,
}

struct App {
    game: Option<Game>,
    screen: Screen,
    data: Data,
    presets: Vec<Preset>,
    preset: usize,
    seed: String,
    map: MapView,
    /// The action whose target is being chosen, with its targets.
    picking: Option<(String, Vec<Target>)>,
    /// The last refusal from the core.
    note: String,
    frame_ms: Option<f32>,
}

fn load_data() -> Data {
    let mut d = bd_core::data::load(RULES).expect("rules.ron");
    EVENTS
        .iter()
        .for_each(|e| d.add_events(e).expect("data/events"));
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
            .map(|(p, m)| Preset::load_with_map(p, m, &data).expect("data/presets"))
            .collect();
        App {
            game: None,
            screen: Screen::Start,
            map: MapView::new(&presets[0].map.polygons),
            data,
            presets,
            preset: 0,
            seed: "1".into(),
            picking: None,
            note: String::new(),
            frame_ms: None,
        }
    }

    fn apply(&mut self, cmd: Cmd) {
        self.note.clear();
        if let Cmd::Start(seed) = cmd {
            let p = &self.presets[self.preset];
            self.map = MapView::new(&p.map.polygons);
            self.game = Some(Game::new(self.data.clone(), p, seed));
            self.screen = Screen::Reign;
            return;
        }
        if cmd == Cmd::Again {
            (self.game, self.screen, self.picking) = (None, Screen::Start, None);
            return;
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
            Cmd::Start(_) | Cmd::Again => unreachable!("handled above"),
        };
        self.step(res);
    }

    fn step(&mut self, res: Result<Step, GameError>) {
        match res {
            Ok(Step::Idle) => self.screen = Screen::Reign,
            Ok(Step::Event(v)) => self.screen = Screen::Event(v),
            Ok(Step::ReignEnded(_)) => self.screen = Screen::ReignEnded,
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
            Screen::ReignEnded => self.ended(ui),
        };
        if let Some(cmd) = cmd {
            self.apply(cmd);
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
            let own = w
                .provinces
                .values()
                .filter(|p| !matches!(p.holder, Holder::Foreign(_)));
            let crown = own.clone().filter(|p| p.holder == Holder::Crown).count();
            // Right to left: value first, then its key.
            key_rtl(
                ui,
                "Провинций",
                &format!("{} · короне {crown}", own.count()),
            );
            if let Some(army) = w.axes.get(&AxisId(ARMY.into())) {
                key_rtl(ui, "Армия", &round(*army));
            }
            let income = yearly_income(g);
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
            });
        });
        if !self.note.is_empty() {
            ui.label(RichText::new(&self.note).color(RUBRIC));
        }
        ui.add_space(4.0);
        cmd
    }

    fn ended(&self, ui: &mut Ui) -> Option<Cmd> {
        let g = self.game.as_ref().expect("in a game");
        let cause = g.ended.as_deref().unwrap_or_default();
        // Causes are event ids (illness, abdication...); show the title when there is one.
        let title = (g.data.events.iter())
            .find(|e| e.id == cause)
            .map_or(cause, |e| &e.title);
        let mut cmd = None;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(format!("Правление окончено: {title}"));
            let w = &g.world;
            ui.label(w.tick.date(w.time_unit, w.start_year));
            if ui.button("Снова").clicked() {
                cmd = Some(Cmd::Again);
            }
        });
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
                    for (text, up) in effects(&c.effects) {
                        ui.small(RichText::new(text).color(if up { GOOD } else { RUBRIC }));
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
            bar(ui, axis_name(&a.id), v, a.min, a.max, &round(v));
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

fn heading(ui: &mut Ui, text: &str) {
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

fn axis_name(id: &AxisId) -> &str {
    AXIS_NAMES
        .iter()
        .find(|(a, _)| *a == id.0)
        .map_or(&id.0, |(_, n)| n)
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

/// Axis effects of a choice with their sign: `("Знать +15", true)`. Other effects stay hidden.
fn effects(effects: &[Effect]) -> Vec<(String, bool)> {
    let axes = effects.iter().filter_map(|e| match e {
        Effect::Axis(a, v) => Some((a, *v)),
        _ => None,
    });
    axes.map(|(a, v)| {
        let sign = if v > Fx(0) { "+" } else { "" };
        (format!("{} {sign}{v}", axis_name(a)), v > Fx(0))
    })
    .collect()
}

/// What the treasury gains in a year by the formula of `Data.economy`.
// ponytail: mirrors the yearly sum in `Game::passive`; a getter in core would keep them in step.
fn yearly_income(g: &Game) -> Fx {
    let (w, e) = (&g.world, &g.data.economy);
    let crown = w.provinces.values().filter(|p| p.holder == Holder::Crown);
    let income = crown.fold(Fx(0), |s, p| s + p.income);
    e.flows.iter().fold(income, |s, (a, k)| s + w.axes[a] * *k)
}

/// `1 год`, `3 года`, `5 лет`, `21 год`, `11 лет`.
fn years(n: u32) -> &'static str {
    match (n % 10, n % 100) {
        (_, 11..=14) => "лет",
        (1, _) => "год",
        (2..=4, _) => "года",
        _ => "лет",
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
    eframe::run_native(
        "Blessed Dynasty",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
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
    wasm_bindgen_futures::spawn_local(async {
        let app: eframe::AppCreator = Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx))));
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

        /// Press and release of the left button at `pos`, as the mouse does it.
        fn click(&mut self, pos: Pos2) {
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
        }

        fn game(&self) -> &Game {
            self.app.game.as_ref().unwrap()
        }

        fn province_on_screen(&self, id: &str) -> Pos2 {
            let map = &self.app.map;
            map.to_screen(map.centre(&ProvinceId(id.into())).unwrap())
        }
    }

    #[test]
    fn reign_plays_from_start_to_the_end() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(7));
        let mut events = 0;
        for year in 0..200 {
            h.frame(vec![]);
            match &h.app.screen {
                Screen::Reign => {
                    // Now and then an action: by the map for province targets.
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
                Screen::ReignEnded => break,
                Screen::Start => unreachable!(),
            }
            assert!(
                h.app.note.is_empty() || h.app.note.contains("слоты"),
                "{}",
                h.app.note
            );
        }
        assert!(
            matches!(h.app.screen, Screen::ReignEnded),
            "the ruler outlives 200 years"
        );
        assert!(events > 0);
        assert!(!h.game().decisions.is_empty());
        h.frame(vec![]);
        h.app.apply(Cmd::Again);
        h.frame(vec![]);
        assert!(matches!(h.app.screen, Screen::Start) && h.app.game.is_none());
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
        assert!(matches!(h.app.screen, Screen::ReignEnded));
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
    fn choice_effects_show_axes_with_sign() {
        let ax = |s: &str| AxisId(s.into());
        let shown = effects(&[
            Effect::Axis(ax("loyalty_nobles"), Fx::from_int(15)),
            Effect::RulerHealth(Fx::from_int(5)),
            Effect::Axis(ax("treasury"), Fx::from_int(-360)),
            Effect::Axis(ax("no_name"), Fx(1500)),
        ]);
        let want = [
            ("Знать +15", true),
            ("Казна -360", false),
            ("no_name +1.5", true),
        ];
        assert_eq!(shown, want.map(|(s, up)| (s.to_string(), up)));
    }

    #[test]
    fn every_event_file_is_embedded() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/events");
        let mut files: Vec<_> = (std::fs::read_dir(dir).unwrap())
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "ron"))
            .collect();
        files.sort();
        let texts: Vec<String> = files
            .iter()
            .map(|p| std::fs::read_to_string(p).unwrap())
            .collect();
        assert_eq!(texts, EVENTS, "EVENTS must list {files:?} in this order");
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
