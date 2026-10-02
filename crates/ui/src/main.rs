//! egui front end: start, reign, event, chronicle, score. Every rule lives in `bd_core`; this
//! only shows and asks.

mod chronicle;
mod map;

use bd_core::data::{Data, ENACT, Law, REPEAL};
use bd_core::fx::Fx;
use bd_core::game::{EventView, Game, GameError, ReignEnd, Step};
use bd_core::link;
use bd_core::rng::Rng;
use bd_core::rules::{Action, ActionTarget, Effect, HeirOp, ProvinceField, Target};
use bd_core::score::{self, Score, ScoreRules};
use bd_core::sim::{self, Chronicle};
use bd_core::state::{
    AxisId, Change, HeirStatus, Holder, NeighbourId, Preset, ProvinceId, Sex, Stance, World,
};
use bd_core::war::War;
use eframe::egui::{self, Button, Grid, ProgressBar, RichText, Ui};
use map::{BG, BG2, FG, FG2, GOOD, MapView, RUBRIC, WARN, holder_name, round};

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
    /// The reign is over: its summary over the last reign screen, the dynasty simulated.
    ReignOver,
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
    /// Close the backstory and rule.
    Begin,
    /// Open or close the family tree.
    Tree(bool),
    /// Open or close the list of succession laws to bring in.
    Laws(bool),
}

/// A line of the journal or of an effect list; `Some(true)` good, `Some(false)` bad.
type Line = (String, Option<bool>);

/// The start of the treasury line every year of the journal has.
const MONEY: &str = "Казна:";

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
    /// The backstory card is up: a new game before its first move.
    intro: bool,
    /// The family tree card is up.
    tree: bool,
    /// The list of laws to bring in is up, in place of the actions.
    laws: bool,
    /// What happened, year by year: the date and its lines, oldest first.
    journal: Vec<(String, Vec<Line>)>,
    /// The world when «Подождать год» was pressed, and the choices made since: the year's
    /// record in the making.
    year_start: Option<World>,
    chosen: Vec<Line>,
    /// The treasury before the latest year's tick, that year's income and upkeep
    /// (`war::income_parts`): its «Казна:» line.
    money: Option<(Fx, Fx, Fx)>,
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
            intro: false,
            tree: false,
            laws: false,
            journal: Vec::new(),
            year_start: None,
            chosen: Vec::new(),
            money: None,
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
            Cmd::Begin => {
                self.intro = false;
                return;
            }
            Cmd::Tree(open) => {
                self.tree = open;
                return;
            }
            Cmd::Laws(open) => {
                (self.laws, self.picking) = (open, None);
                return;
            }
            Cmd::Choose(i) => {
                if let Screen::Event(v) = &self.screen
                    && let Some(c) = v.choices.get(i)
                {
                    self.chosen
                        .push((format!("«{}»: {}", v.title, c.text), None));
                }
            }
            _ => {}
        }
        let closes_year = matches!(cmd, Cmd::Wait | Cmd::Choose(_));
        let spends = matches!(cmd, Cmd::Act(..));
        let g = self.game.as_mut().expect("only Start runs without a game");
        let res = match cmd {
            Cmd::Wait => {
                self.year_start.get_or_insert_with(|| g.world.clone());
                let (income, upkeep) = bd_core::war::income_parts(&g.world, &g.data);
                let treasury = g.world.axes[&g.data.economy.treasury];
                self.money = Some((treasury, income, upkeep));
                // A year of ticks, up to the first event.
                let mut res = g.wait();
                for _ in 1..g.data.time_unit.ticks_per_year {
                    if res != Ok(Step::Idle) {
                        break;
                    }
                    res = g.wait();
                }
                res
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
                (self.picking, self.laws) = (None, false);
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
            | Cmd::CopyLink
            | Cmd::Begin
            | Cmd::Tree(_)
            | Cmd::Laws(_) => {
                unreachable!("handled above")
            }
        };
        self.step(res);
        // A year waiting on its event is not recorded yet: its money line comes with it.
        let closed = closes_year && matches!(self.screen, Screen::Reign);
        if closed {
            self.close_year();
        }
        if closed || spends {
            self.money_line();
        }
    }

    /// «Казна: +N (доход +X, содержание -Y, траты -Z)» first in the latest year's record: N
    /// from before its tick to now, траты what the actions and choices since took.
    fn money_line(&mut self) {
        let (Some(g), Some((start, income, upkeep))) = (&self.game, self.money) else {
            return;
        };
        let Some((_, lines)) = self.journal.last_mut() else {
            return;
        };
        let now = g.world.axes[&g.data.economy.treasury];
        let spent = now - start - income + upkeep;
        let sign = |v: Fx| {
            if v > Fx(0) {
                format!("+{}", round(v))
            } else {
                round(v)
            }
        };
        let text = format!(
            "{MONEY} {} (доход {}, содержание {}, траты {})",
            sign(now - start),
            sign(income),
            sign(Fx(0) - upkeep),
            sign(spent),
        );
        lines.retain(|(t, _)| !t.starts_with(MONEY));
        lines.insert(0, (text, Some(now >= start)));
    }

    /// Puts what happened since «Подождать год» into the journal: the choices made, then
    /// what changed. A second record of the same date joins the first.
    fn close_year(&mut self) {
        let g = self.game.as_ref().expect("in a game");
        let mut lines = std::mem::take(&mut self.chosen);
        if let Some(before) = self.year_start.take() {
            lines.extend(change_lines(g, &before, &g.world));
        }
        let date = g.world.tick.date(g.world.time_unit, g.world.start_year);
        match self.journal.last_mut() {
            Some((d, l)) if *d == date => l.extend(lines),
            _ => self.journal.push((date, lines)),
        }
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
        self.intro = false;
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
        (self.intro, self.tree, self.year_start) = (true, false, None);
        self.laws = false;
        (self.journal, self.chosen, self.money) = (Vec::new(), Vec::new(), None);
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
                self.screen = Screen::ReignOver;
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
        let ctx = ui.ctx().clone();
        let cmd = match &self.screen {
            Screen::Start => self.start_screen(ui),
            // The reign screen stays visible under a card; the modal blocks its clicks.
            Screen::Reign if self.intro => {
                self.reign(ui);
                intro(&ctx, &self.presets[self.preset])
            }
            Screen::Reign if self.laws => {
                self.reign(ui);
                laws(&ctx, self.game.as_ref().expect("in a game"))
            }
            Screen::Reign => self.reign(ui),
            Screen::Event(v) => {
                self.reign(ui);
                event(&ctx, self.game.as_ref().expect("in a game"), v)
            }
            Screen::ReignOver => {
                self.reign(ui);
                let g = self.game.as_ref().expect("in a game");
                let start = World::from_preset(&self.data, &self.presets[self.preset]);
                let dynasty = self.dynasty.as_ref().expect("after the reign");
                chronicle::reign_over(&ctx, g, &start, dynasty)
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
        if self.tree {
            let g = self.game.as_ref().expect("the tree opens in a game");
            let (kin, rulers) = match (&self.screen, &self.dynasty) {
                (Screen::Chronicle | Screen::Summary, Some((c, _))) => (&c.kin, &c.rulers[..]),
                _ => (&g.world.kin, &[][..]),
            };
            if chronicle::tree(&ctx, kin, rulers, g.world.start_year, g.world.time_unit) {
                self.tree = false;
            }
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
        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Родословная").clicked() {
                    cmd = Some(Cmd::Tree(true));
                }
                self.top_bar(ui, g)
            })
        });
        egui::Panel::bottom("actions").show(ui, |ui| cmd = cmd.take().or(self.actions(ui, g)));
        egui::Panel::right("side").exact_size(300.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| side(ui, g))
        });
        egui::Panel::left("journal")
            .exact_size(260.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| journal(ui, &self.journal))
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
                map::legend(ui, &g.world);
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
        if let Some(war) = &w.war {
            let enemy = target_name(w, &Target::Neighbour(war.enemy.clone()));
            let flag = RichText::new(format!("⚔ Война: {enemy}"))
                .color(BG)
                .strong();
            ui.add(Button::new(flag).fill(RUBRIC).sense(egui::Sense::hover()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(ms) = self.frame_ms {
                ui.small(RichText::new(format!("кадр {ms:.1} мс")).color(FG2));
            }
            // Right to left: value first, then its key.
            key_rtl(ui, "Провинций", &realm(w));
            if let Some(army) = w.axes.get(&d.war.army) {
                key_rtl(ui, axis_name(d, &d.war.army), &round(*army));
            }
            let income = bd_core::war::yearly_income(w, d);
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
                        action_name(w, d, id)
                    ));
                    let def = d.actions.iter().find(|a| a.id == *id);
                    let suit = def.filter(|a| a.marries());
                    let rightful = rightful_heir(g).filter(|_| def.is_some_and(designates));
                    for t in targets {
                        let label = match (suit, t) {
                            (Some(_), Target::Neighbour(n)) => {
                                let chance = d.marriage.chance(w, d, n);
                                format!("{} · {}%", target_name(w, t), round(chance))
                            }
                            (_, Target::Heir(id)) if rightful.is_some() => {
                                let lawful = rightful.is_some_and(|r| r.id == *id);
                                let how = if lawful {
                                    "по закону"
                                } else {
                                    "в обход закона"
                                };
                                format!("{} · {how}", target_name(w, t))
                            }
                            _ => target_name(w, t),
                        };
                        if ui.button(label).clicked() {
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
                    // The laws to bring in go to a list of their own.
                    let others = (d.actions.iter())
                        .filter(|a| law_of(d, a).is_some_and(|l| d.heirs.law(w) != Some(l)));
                    if others.count() > 0 && ui.button("Сменить закон").clicked() {
                        cmd = Some(Cmd::Laws(true));
                    }
                    for (id, targets) in g.available_actions() {
                        let def = d.actions.iter().find(|a| a.id == id);
                        if id.starts_with(ENACT) || id.starts_with(REPEAL) {
                            continue;
                        }
                        let button = ui.button(action_name(w, d, &id));
                        let button = match def {
                            Some(a) => button.on_hover_ui(|ui| action_tip(ui, g, a)),
                            None => button,
                        };
                        // A war action has one target, the enemy: no choice to make.
                        let enemy = def.is_some_and(|a| a.target == ActionTarget::Enemy);
                        if button.clicked() {
                            cmd = Some(match (targets.is_empty(), enemy) {
                                (true, _) => Cmd::Act(id, None),
                                (false, true) => Cmd::Act(id, targets.first().cloned()),
                                (false, false) => Cmd::Pick(id, targets),
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

/// The law an action brings in: the one its `on_complete` enacts.
fn law_of<'a>(d: &'a Data, a: &Action) -> Option<&'a Law> {
    let sets = |f: &String| a.on_complete.contains(&Effect::EnactLaw(f.clone()));
    d.heirs.laws.iter().find(|l| sets(&l.flag))
}

/// «Сменить закон»: a card with every law but the one in force, its text, price and
/// resistance; those not to be had now are greyed out.
fn laws(ctx: &egui::Context, g: &Game) -> Option<Cmd> {
    let (w, d) = (&g.world, &g.data);
    let mut cmd = None;
    let open: Vec<String> = (g.available_actions().into_iter())
        .map(|(id, _)| id)
        .collect();
    egui::Modal::new(egui::Id::new("laws")).show(ctx, |ui| {
        ui.set_width(640.0);
        ui.label(RichText::new("Сменить закон").size(20.0).strong());
        ui.label("Новый закон о престоле вводят годами, за деньги и против воли знати или церкви.");
        if w.flags.contains(&d.abdication.contested_flag) {
            let busy = "Пока идёт спор о престоле, закон не сменить.";
            ui.label(RichText::new(busy).color(RUBRIC));
        }
        for a in &d.actions {
            let Some(l) = law_of(d, a).filter(|l| d.heirs.law(w) != Some(*l)) else {
                continue;
            };
            ui.separator();
            let button = Button::new(RichText::new(&l.name).strong());
            if ui.add_enabled(open.contains(&a.id), button).clicked() {
                cmd = Some(Cmd::Act(a.id.clone(), None));
            }
            ui.label(l.text());
            let n = a.duration_years.0;
            let mut price = format!("Стоимость {} · {n} {}", round(a.cost), years(n));
            if a.min_crown_power > Fx(0) {
                price += &format!(" · сила короны от {}", round(a.min_crown_power));
            }
            ui.label(RichText::new(price).color(FG2));
            // «пока вводят, к цели: Церковь -10 · по введении: Знать +3, Церковь -2».
            let list = |es: &[Effect]| {
                let lines: Vec<String> = effects(d, es).into_iter().map(|(t, _)| t).collect();
                lines.join(", ")
            };
            let against = d.law(&l.flag).map_or(&[][..], |x| &x.resistance);
            let against: Vec<Effect> = (against.iter())
                .map(|(a, v)| Effect::Axis(a.clone(), *v))
                .collect();
            let parts = [
                ("пока вводят, к цели", list(&against)),
                ("по введении", list(&a.on_complete)),
            ];
            let parts: Vec<String> = (parts.iter())
                .filter(|(_, l)| !l.is_empty())
                .map(|(when, l)| format!("{when}: {l}"))
                .collect();
            ui.small(RichText::new(parts.join(" · ")).color(FG2));
        }
        ui.separator();
        if ui.button("Отмена").clicked() {
            cmd = Some(Cmd::Laws(false));
        }
    });
    cmd
}

/// «Действия k из n, военные j из m: идёт X, t из T лет»; the war slots only at war.
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
            let name = acted(w, def, a.target.as_deref());
            let total = def.duration_years.0 as f32;
            let done = total - a.ends_at.0.saturating_sub(w.tick.0) as f32 / tpy;
            format!("{name}, {} из {} лет", num(done), num(total))
        })
        .collect();
    let war = |a: &Action| a.target == ActionTarget::Enemy;
    let def = |id: &str| d.actions.iter().find(|a| a.id == id);
    let wars = (w.active_actions.iter()).filter(|a| def(&a.id).is_some_and(war));
    let wars = wars.count();
    let mut line = format!(
        "Действия {} из {}",
        running.len() - wars,
        d.action_slots.slots(w)
    );
    if w.war.is_some() || wars > 0 {
        line += &format!(", военные {wars} из {}", d.action_slots.war_slots);
    }
    match running.is_empty() {
        true => line,
        false => format!("{line}: идёт {}", running.join("; ")),
    }
}

/// «Построить крепость (Берг)»: an action with the target of its `ActiveAction` key.
fn acted(w: &World, def: &Action, key: Option<&str>) -> String {
    let target = key.map(|key| match def.target {
        ActionTarget::Province(_) => Target::Province(ProvinceId(key.into())),
        ActionTarget::Neighbour | ActionTarget::Enemy => Target::Neighbour(NeighbourId(key.into())),
        ActionTarget::Heir | ActionTarget::None => Target::Heir(key.parse().unwrap_or(u32::MAX)),
    });
    match target {
        Some(t) => format!("{} ({})", named(w, &def.name), target_name(w, &t)),
        None => named(w, &def.name),
    }
}

/// What changed from `before` to `w`, as journal lines.
fn change_lines(g: &Game, before: &World, w: &World) -> Vec<Line> {
    let d = &g.data;
    let province = |id: &ProvinceId| w.provinces.get(id).map_or(id.0.clone(), |p| p.name.clone());
    (w.changes(before, d).into_iter())
        .map(|c| match c {
            Change::Axis(a, v) => signed(axis_name(d, &a), v),
            Change::Born(name) => (format!("Рождение: {name}"), Some(true)),
            Change::HeirGone(name) => (format!("Смерть наследника: {name}"), Some(false)),
            Change::Holder(id, from, to) => {
                let p = province(&id);
                match (&from, &to) {
                    (_, Holder::Foreign(_)) => {
                        let to = holder_name(w, &to);
                        (format!("Потеряна земля {p}: теперь {to}"), Some(false))
                    }
                    (Holder::Foreign(_), _) => {
                        let from = holder_name(before, &from);
                        (format!("Присоединена земля {p}, прежде {from}"), Some(true))
                    }
                    (_, Holder::Crown) => (format!("{p} снова под короной"), None),
                    _ => (format!("{p} отошла: {}", holder_name(w, &to)), None),
                }
            }
            Change::Done(id, key) => {
                let def = d.actions.iter().find(|a| a.id == id);
                let what = def.map_or(id.clone(), |a| acted(w, a, key.as_deref()));
                (format!("Завершено: {what}"), None)
            }
        })
        .collect()
}

fn tone(up: Option<bool>) -> egui::Color32 {
    match up {
        Some(true) => GOOD,
        Some(false) => RUBRIC,
        None => FG2,
    }
}

/// «Знать +15», good when up.
fn signed(name: &str, v: Fx) -> Line {
    let sign = if v > Fx(0) { "+" } else { "" };
    (format!("{name} {sign}{v}"), Some(v > Fx(0)))
}

/// The journal, the latest year on top and stressed.
fn journal(ui: &mut Ui, journal: &[(String, Vec<Line>)]) {
    heading(ui, "Итоги года");
    if journal.is_empty() {
        let hint = "Здесь появится, что случилось за год, после «Подождать год».";
        ui.small(RichText::new(hint).color(FG2));
    }
    for (i, (date, lines)) in journal.iter().rev().enumerate() {
        if i == 1 {
            heading(ui, "Прежние годы");
        }
        let date = RichText::new(date).strong();
        ui.label(if i == 0 {
            date.size(16.0)
        } else {
            date.color(FG2)
        });
        for (text, up) in lines {
            ui.small(RichText::new(text).color(tone(*up)));
        }
        if lines.iter().all(|(t, _)| t.starts_with(MONEY)) {
            ui.small(RichText::new("Тихий год").color(FG2));
        }
        ui.add_space(4.0);
    }
}

/// The backstory of the preset and how to play, before the first move.
fn intro(ctx: &egui::Context, p: &Preset) -> Option<Cmd> {
    let mut cmd = None;
    egui::Modal::new(egui::Id::new("intro")).show(ctx, |ui| {
        ui.set_width(560.0);
        let eyebrow = format!("{} · {} год", p.ruler.name, p.start_year);
        ui.label(RichText::new(eyebrow).small().color(RUBRIC));
        ui.label(RichText::new("Предыстория").size(20.0).strong());
        ui.label(&p.intro);
        heading(ui, "Как играть");
        for line in HOW_TO_PLAY {
            ui.label(format!("· {line}"));
        }
        ui.add_space(8.0);
        let rule = Button::new(RichText::new("Править").color(BG)).fill(FG);
        if ui.add(rule).clicked() {
            cmd = Some(Cmd::Begin);
        }
    });
    cmd
}

const HOW_TO_PLAY: [&str; 4] = [
    "Цель: оставить потомкам крепкое государство. Вы правите только первым государем, \
     счёт считается по тому, сколько проживёт династия и чего она достигнет.",
    "Ход: начните действие внизу экрана и нажмите «Подождать год». За год случаются \
     события: выберите вариант в окне. Что произошло, видно в итогах года слева.",
    "Наведите мышь на действие, соседа, закон или провинцию, чтобы узнать подробности.",
    "Правление кончается смертью государя или отречением. Дальше симуляция разыграет \
     судьбу династии, а хроника покажет, к чему привели ваши решения.",
];

/// An action that names the heir (`HeirOp::TargetDesignate`).
fn designates(a: &Action) -> bool {
    (a.on_complete).contains(&Effect::HeirOp(HeirOp::TargetDesignate))
}

/// The heir the law puts first (`sim::rightful`).
fn rightful_heir(g: &Game) -> Option<&bd_core::state::Heir> {
    sim::rightful(&g.world, &g.data).map(|i| &g.world.heirs[i])
}

/// What an action gives, costs and takes.
fn action_tip(ui: &mut Ui, g: &Game, a: &Action) {
    let (w, d) = (&g.world, &g.data);
    ui.set_max_width(320.0);
    ui.strong(named(w, &a.name));
    if !a.description.is_empty() {
        ui.label(&a.description);
    }
    let years = a.duration_years.0;
    let time = match years {
        0 => "сразу".to_string(),
        n => format!("{n} {}", plural(n, ["год", "года", "лет"])),
    };
    ui.label(RichText::new(format!("Стоимость {} · {time}", round(a.cost))).color(FG2));
    if a.min_crown_power > Fx(0) {
        let need = format!("Нужна сила короны от {}", round(a.min_crown_power));
        ui.label(RichText::new(need).color(FG2));
    }
    let yearly = effects(d, &a.yearly).into_iter();
    let yearly = yearly.map(|(t, up)| (format!("пока идёт, в год: {t}"), up));
    for (text, up) in yearly.chain(effects(d, &a.on_complete)) {
        ui.small(RichText::new(text).color(tone(up)));
    }
    if designates(a) {
        let name = rightful_heir(g).map_or("никто", |h| h.name.as_str());
        ui.label(format!("По закону престол наследует: {name}"));
        let penalty: Vec<Effect> = (d.heirs.designate_penalty.iter())
            .map(|(a, v)| Effect::Axis(a.clone(), *v))
            .collect();
        let lines = effects(d, &penalty).into_iter().map(|(t, _)| t);
        let lines: Vec<String> = lines.collect();
        let dispute = round(d.heirs.designate_dispute);
        ui.small(
            RichText::new(format!(
                "Назначить другого: {}, спор при воцарении вероятнее на {dispute}%",
                lines.join(", ")
            ))
            .color(RUBRIC),
        );
    }
    if a.marries() {
        // The chance of every court before the suit; 0: it turns any suit away.
        let (yes, no): (Vec<_>, Vec<_>) = (w.neighbours.values())
            .map(|n| (n, d.marriage.chance(w, d, &n.id)))
            .partition(|(_, c)| *c > Fx(0));
        let yes: Vec<String> = yes
            .iter()
            .map(|(n, c)| format!("{} {}%", n.name, round(*c)))
            .collect();
        if !yes.is_empty() {
            ui.label(format!("Шанс согласия: {}", yes.join(", ")));
        }
        let no: Vec<&str> = no.iter().map(|(n, _)| n.name.as_str()).collect();
        if !no.is_empty() {
            ui.label(RichText::new(format!("Сватов не примут: {}", no.join(", "))).color(FG2));
        }
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
                        ui.small(RichText::new(text).color(tone(up)));
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
    if let Some(war) = &w.war {
        war_panel(ui, g, war);
    }
    heading(ui, "Состояние");
    Grid::new("axes").show(ui, |ui| {
        for a in d.axes.iter().filter(|a| !a.hidden) {
            let v = w.axes[&a.id];
            bar(ui, axis_name(d, &a.id), v, a.min, a.max, &round(v));
        }
    });
    if let Some((line, hint)) = overreach(g) {
        ui.label(RichText::new(line).color(RUBRIC));
        ui.label(RichText::new(hint).small().color(FG2));
    }
    heading(ui, "Наследники");
    let first = bd_core::sim::successor(w, d);
    let rightful = bd_core::sim::rightful(w, d);
    match d.heirs.law(w) {
        Some(l) => {
            let law = ui.label(format!("Закон: {} (?)", l.name));
            law.on_hover_ui(|ui| {
                ui.set_max_width(320.0);
                ui.strong(&l.name);
                ui.label(l.text());
            });
        }
        None => {
            ui.label("Закона наследования нет");
        }
    }
    let line = first.map_or("никого".into(), |i| w.heirs[i].name.clone());
    ui.label(format!("Первый в очереди: {line}"));
    if w.heirs.is_empty() {
        ui.label("нет");
    }
    Grid::new("heirs").show(ui, |ui| {
        for (i, h) in w.heirs.iter().enumerate() {
            let status = match &h.status {
                HeirStatus::Home if Some(i) == first && first != rightful => {
                    RichText::new("назначен").color(RUBRIC)
                }
                HeirStatus::Home if Some(i) == first => RichText::new("первый").color(FG2),
                HeirStatus::Home if Some(i) == rightful => RichText::new("по закону").color(FG2),
                HeirStatus::Home => RichText::new(""),
                HeirStatus::Studying(place) => RichText::new(format!("учится: {place}")).color(FG2),
                HeirStatus::Hostage(n) => {
                    let n = w.neighbours.get(n).map_or(&n.0, |n| &n.name);
                    RichText::new(format!("заложник: {n}")).color(RUBRIC)
                }
            };
            let sex = match h.sex {
                Sex::Male => "♂",
                Sex::Female => "♀",
            };
            let bastard = if h.bastard { " (бастард)" } else { "" };
            ui.label(format!("{sex} {}{bastard}, {}", h.name, h.age));
            ui.small(status);
            ui.small(format!(
                "спос. {} · прет. {}",
                round(h.ability),
                round(h.claim)
            ));
            ui.end_row();
        }
    });
    if !w.bastards.is_empty() {
        let names: Vec<String> = (w.bastards.iter())
            .map(|h| format!("{}, {}", h.name, h.age))
            .collect();
        let line = ui.label(format!("Бастарды: {}", names.join("; ")));
        line.on_hover_text("Рождены вне брака и не наследуют, пока их не признают.");
    }
    heading(ui, "Соседи");
    let bonds = g.bonds();
    Grid::new("neighbours").show(ui, |ui| {
        for n in w.neighbours.values() {
            let sign = if n.relation > Fx(0) { "+" } else { "" };
            let value = format!("{sign}{}", round(n.relation));
            let (lo, hi) = (Fx::from_int(-100), Fx::from_int(100));
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
            let ties: Vec<String> = (bonds.iter())
                .filter(|(id, ..)| *id == n.id)
                .map(|(_, a, t)| format!("{} с {}", a.bond, t.date(w.time_unit, w.start_year)))
                .collect();
            let at_war = w.war.as_ref().is_some_and(|x| x.enemy == n.id);
            let label = match (ties.is_empty(), at_war) {
                (_, true) => format!("{} ⚔", n.name),
                (false, _) => format!("{} ♥", n.name),
                _ => n.name.clone(),
            };
            let row = bar(ui, &label, n.relation, lo, hi, &value);
            row.on_hover_ui(|ui| {
                ui.strong(&n.name);
                ui.label(format!("Отношение {value}: {mood}"));
                ui.label(format!("Сила {}, {stance}", round(n.strength)));
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
                if at_war {
                    ui.label(RichText::new("Идёт война").color(RUBRIC));
                }
            });
        }
    });
}

/// «Сверх предела: k земель (…), штраф в год: …» and what to do about it; None within the
/// limit (`crown_capacity`).
fn overreach(g: &Game) -> Option<(String, String)> {
    let (w, d) = (&g.world, &g.data);
    let c = &d.crown_capacity;
    let over = c.over(w);
    if over.is_empty() {
        return None;
    }
    let k = over.len() as u32;
    let names: Vec<&str> = over.iter().map(|p| p.name.as_str()).collect();
    let times = Fx::from_int(k as i64);
    let mut fines: Vec<String> = (c.penalty.iter())
        .map(|(a, v)| format!("{} {}", axis_name(d, a).to_lowercase(), round(*v * times)))
        .collect();
    if c.income != Fx(0) {
        fines.push(format!("доход -{}", round(c.income * times)));
    }
    if c.loyalty != Fx(0) {
        let these = plural(k, ["этой земли", "этих земель", "этих земель"]);
        fines.push(format!("лояльность {these} -{}", round(c.loyalty)));
    }
    let lands = plural(k, ["земля", "земли", "земель"]);
    let line = format!(
        "Сверх предела: {k} {lands} ({}), штраф в год: {}",
        names.join(", "),
        fines.join(", ")
    );
    let room = c.room(w);
    let terms =
        (c.per_axis.iter()).map(|(a, k)| format!(" + {} × {k}", axis_name(d, a).to_lowercase()));
    let hint = format!(
        "Корона сама держит не больше {room} {} (сила короны в столице × {}{}). Пожалуйте \
         лишние земли вассалам или укрепите власть короны в столице.",
        plural(room as u32, ["земли", "земель", "земель"]),
        c.per_power,
        terms.collect::<String>(),
    );
    Some((line, hint))
}

/// The war going on: enemy, target, the score between defeat and victory, forces, years,
/// the last battles.
fn war_panel(ui: &mut Ui, g: &Game, war: &War) {
    let (w, d) = (&g.world, &g.data);
    heading(ui, "Война");
    let enemy = target_name(w, &Target::Neighbour(war.enemy.clone()));
    let tpy = w.time_unit.ticks_per_year;
    let year = w.tick.0.saturating_sub(war.started.0) / tpy + 1;
    ui.label(
        RichText::new(format!("⚔ {enemy}, {year}-й год войны"))
            .color(RUBRIC)
            .strong(),
    );
    let target = war
        .target
        .as_ref()
        .map(|p| target_name(w, &Target::Province(p.clone())));
    ui.label(format!("Цель: {}", target.unwrap_or("нет".into())));
    // The score from -max (defeat) to +max (victory), filled from the middle.
    let (r, _) = ui.allocate_exact_size(egui::vec2(260.0, 10.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 0.0, BG2);
    let share = (war.war_score.0 as f32 / d.war.max_score.0.max(1) as f32).clamp(-1.0, 1.0);
    let x = r.center().x + share * r.width() / 2.0;
    let (lo, hi) = (x.min(r.center().x), x.max(r.center().x));
    let fill = if share >= 0.0 { GOOD } else { RUBRIC };
    p.rect_filled(egui::Rect::from_x_y_ranges(lo..=hi, r.y_range()), 0.0, fill);
    p.line_segment([r.center_top(), r.center_bottom()], (1.0, FG));
    let score = war.war_score;
    let lead = match score {
        s if s > Fx(0) => "перевес наш",
        s if s < Fx(0) => "перевес врага",
        _ => "равенство",
    };
    ui.horizontal(|ui| {
        ui.small(RichText::new("поражение").color(RUBRIC));
        let sign = if score > Fx(0) { "+" } else { "" };
        ui.small(format!("счёт {sign}{}: {lead}", round(score)));
        ui.small(RichText::new("победа").color(GOOD));
    });
    let (ours, theirs) = bd_core::war::strengths(w, d, &war.enemy);
    ui.label(format!(
        "Силы: наши {} · враг {}",
        round(ours),
        round(theirs)
    ));
    match war.battles.len() {
        0 => {
            ui.small(RichText::new("Битв ещё не было").color(FG2));
        }
        n => {
            let last = war.battles.iter().rev().take(3);
            let told: Vec<String> = last
                .map(|(t, delta)| {
                    let sign = if *delta > Fx(0) { "+" } else { "" };
                    format!(
                        "{} {sign}{}",
                        t.date(w.time_unit, w.start_year),
                        round(*delta)
                    )
                })
                .collect();
            let n = format!("{n} {}", plural(n as u32, ["битва", "битвы", "битв"]));
            ui.small(format!("{n}, последние: {}", told.join(", ")));
        }
    }
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
/// Returns the label's response, for a tooltip.
fn bar(ui: &mut Ui, label: &str, v: Fx, min: Fx, max: Fx, value: &str) -> egui::Response {
    let share = ((v - min).0 as f32 / (max - min).0.max(1) as f32).clamp(0.0, 1.0);
    let color = match share {
        s if s < 0.35 => RUBRIC,
        s if s < 0.55 => WARN,
        _ => GOOD,
    };
    let label = ui.small(label);
    let bar = ProgressBar::new(share).fill(color).desired_width(110.0);
    ui.add(bar.desired_height(6.0));
    ui.small(value);
    ui.end_row();
    label
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

fn action_name(w: &World, d: &Data, id: &str) -> String {
    let def = d.actions.iter().find(|a| a.id == id);
    def.map_or(id.into(), |a| named(w, &a.name))
}

/// An action name with `{war_target}`, the province the war going on is fought for, filled in.
fn named(w: &World, name: &str) -> String {
    let target = (w.war.as_ref().and_then(|x| x.target.as_ref()))
        .and_then(|id| w.provinces.get(id))
        .map_or("цель войны", |p| &p.name);
    name.replace("{war_target}", target)
}

/// A province of a foreign state also names the state: «Фростад, Нордмарк».
fn target_name(w: &World, t: &Target) -> String {
    if let Some(p) = match t {
        Target::Province(id) => w.provinces.get(id),
        _ => None,
    } && let Holder::Foreign(_) = p.holder
    {
        return format!("{}, {}", p.name, holder_name(w, &p.holder));
    }
    let name = match t {
        Target::Province(id) => w.provinces.get(id).map(|p| &p.name),
        Target::Neighbour(id) => w.neighbours.get(id).map(|n| &n.name),
        Target::Heir(id) => w.heir_index(*id).map(|i| &w.heirs[i].name),
    };
    name.cloned().unwrap_or_else(|| format!("{t:?}"))
}

/// What effects do, with their sign where they have one: axes, province fields, crown
/// power, relations, holders, war; «риск» once for any chance. Flags and the rest stay hidden.
fn effects(d: &Data, list: &[Effect]) -> Vec<Line> {
    let mut out = Vec::new();
    for e in list {
        let line = match e {
            Effect::Axis(a, v) => signed(axis_name(d, a), *v),
            Effect::Province(_, f, v) => signed(
                match f {
                    ProvinceField::Income => "доход провинции",
                    ProvinceField::Loyalty => "лояльность провинции",
                    ProvinceField::Population => "население провинции",
                },
                *v,
            ),
            Effect::Build(_, b) => {
                let bonus = d.crown_power.buildings.get(b).copied().unwrap_or_default();
                signed("сила короны в провинции", bonus)
            }
            Effect::CrownPower(_, v) => signed("сила короны в провинции", *v),
            Effect::Relation(_, v) => signed("отношения", *v),
            Effect::OtherRelations(v) => signed("отношения с другими соседями", *v),
            Effect::Grant(_) => ("провинция уходит вассалу".into(), None),
            Effect::Revoke(_) => ("провинция возвращается короне".into(), None),
            Effect::TransferProvince(..) | Effect::Secede(_) => {
                ("провинция меняет хозяина".into(), None)
            }
            Effect::StartWar(_) => ("война".into(), Some(false)),
            Effect::Tribute(v) => signed(axis_name(d, &d.economy.treasury), *v),
            Effect::Clash if !out.iter().any(|(t, _)| t == "битва") => ("битва".into(), None),
            Effect::Abdicate | Effect::RulerDies(_) => ("конец правления".into(), Some(false)),
            Effect::IfFriendly(es) => {
                let friendly = effects(d, es).into_iter();
                out.extend(friendly.map(|(t, up)| (format!("если сосед — друг: {t}"), up)));
                continue;
            }
            Effect::Marry { then, otherwise } => {
                let yes = effects(d, then)
                    .into_iter()
                    .map(|(t, up)| (format!("согласие: {t}"), up));
                let no = effects(d, otherwise)
                    .into_iter()
                    .map(|(t, up)| (format!("отказ: {t}"), up));
                out.extend(yes.chain(no));
                continue;
            }
            Effect::Chance(_) if !out.iter().any(|(t, _)| t == "риск") => ("риск".into(), None),
            _ => continue,
        };
        out.push(line);
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
            // A card sizes itself in its first frames and settles in the middle after.
            for _ in 0..4 {
                self.frame(vec![]);
            }
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
        h.click_label("Править");
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
                Screen::ReignOver => {
                    h.click_label("К хронике ▸");
                    assert!(matches!(h.app.screen, Screen::Chronicle));
                    return events;
                }
                Screen::Start | Screen::Summary | Screen::Chronicle => unreachable!(),
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
        h.app.apply(Cmd::Begin);
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

    /// Seed 1: a road to Берг, the first choice of every event, six years.
    fn six_years(h: &mut Harness) {
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let berg = Some(Target::Province(ProvinceId("berg".into())));
        h.app.apply(Cmd::Act("build_road".into(), berg));
        for _ in 0..6 {
            h.app.apply(Cmd::Wait);
            while let Screen::Event(_) = h.app.screen {
                h.app.apply(Cmd::Choose(0));
            }
        }
    }

    /// After a road and six years of seed 1 the journal holds, year by year, the choices
    /// made and what changed, and shows the latest year on top.
    #[test]
    fn the_year_summary_lists_what_happened() {
        let mut h = Harness::new();
        six_years(&mut h);
        let line = |s: &str, up| (s.to_string(), up);
        // Every year opens with the treasury, notable or not.
        let money = line("Казна: +28 (доход +33, содержание -5, траты 0)", Some(true));
        let want = vec![
            (
                "1188",
                vec![
                    line(
                        "Казна: -13 (доход +32, содержание -5, траты -40)",
                        Some(false),
                    ),
                    line("«Беда с наследником»: Позвать лучших лекарей", None),
                    line("Смерть наследника: Конрад", Some(false)),
                ],
            ),
            (
                "1189",
                vec![
                    line(
                        "Казна: -8 (доход +32, содержание -5, траты -35)",
                        Some(false),
                    ),
                    line("«Неурожай»: Раздать зерно из казны", None),
                    line("Рождение: Генрих", Some(true)),
                    line("Завершено: Проложить дорогу (Берг)", None),
                ],
            ),
            (
                "1190",
                vec![money.clone(), line("Рождение: Освальд", Some(true))],
            ),
            (
                "1191",
                vec![
                    money.clone(),
                    line("«Знать требует»: Подтвердить вольности", None),
                    line("Бюрократия -5", Some(false)),
                    line("Знать +11", Some(true)),
                ],
            ),
            (
                "1192",
                vec![
                    line(
                        "Казна: +43 (доход +33, содержание -5, траты +15)",
                        Some(true),
                    ),
                    line("«Пограничная стычка»: Потребовать виру", None),
                    line("Рождение: Рейнхольд", Some(true)),
                ],
            ),
            (
                "1193",
                vec![
                    money,
                    line("«Церковь требует»: Платить десятину", None),
                    line("Доход -2", Some(false)),
                    line("Церковь +9", Some(true)),
                    line("Смерть наследника: Рейнхольд", Some(false)),
                ],
            ),
        ];
        let got: Vec<_> = (h.app.journal.iter())
            .map(|(d, l)| (d.as_str(), l.clone()))
            .collect();
        assert_eq!(got, want);
        let texts = texts(&h.frame(vec![]));
        let (latest, older) = (pos(&texts, "1193"), pos(&texts, "1192"));
        assert!(latest < older, "the latest year comes first");
        assert!(texts.iter().any(|t| t == "Смерть наследника: Конрад"));
        // A quiet year says so; a reign over leaves the journal to the reign's card.
        let d = &mut h.app.game.as_mut().unwrap().data;
        (d.quiet_weight, d.heirs.birth) = (1_000_000, vec![]);
        h.app.apply(Cmd::Wait);
        let quiet = vec![line(
            "Казна: +26 (доход +31, содержание -5, траты 0)",
            Some(true),
        )];
        assert_eq!(h.app.journal.last().unwrap(), &("1194".to_string(), quiet));
        assert!(texts_of(&mut h).contains(&"Тихий год".to_string()));
    }

    /// Stage 15: land over the crown's limit shows its yearly penalty and what to do.
    #[test]
    fn the_side_panel_tells_the_penalty_over_the_limit() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let over = |t: &[String]| t.iter().any(|t| t.starts_with("Сверх предела"));
        assert!(!over(&texts_of(&mut h)), "room 8 for 6 at the start");
        let c = &mut h.app.game.as_mut().unwrap().data.crown_capacity;
        (c.per_power, c.per_axis) = (Fx(50), vec![]); // room 4 for 6
        let texts = texts_of(&mut h);
        let line = texts
            .iter()
            .find(|t| t.starts_with("Сверх предела"))
            .unwrap();
        assert!(line.starts_with("Сверх предела: 2 земли ("), "{line}");
        assert!(
            line.ends_with("штраф в год: стабильность -2, доход -4, лояльность этих земель -3"),
            "{line}"
        );
        let hint = texts
            .iter()
            .find(|t| t.starts_with("Корона сама держит"))
            .unwrap();
        assert!(
            hint.contains("не больше 4 земель") && hint.contains("Пожалуйте"),
            "{hint}"
        );
    }

    /// Every text painted in the frame, tooltips and cards included.
    fn texts(out: &egui::FullOutput) -> Vec<String> {
        fn walk(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut all = Vec::new();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut all));
        all
    }

    fn texts_of(h: &mut Harness) -> Vec<String> {
        texts(&h.frame(vec![]))
    }

    fn pos(texts: &[String], t: &str) -> usize {
        texts
            .iter()
            .position(|x| x == t)
            .unwrap_or_else(|| panic!("no «{t}»"))
    }

    /// The texts on screen with the pointer resting on the widget labelled `label`.
    fn hover(h: &mut Harness, label: &str) -> Vec<String> {
        h.ctx
            .global_style_mut(|s| s.interaction.tooltip_delay = 0.0);
        h.ctx.enable_accesskit();
        // Away first: the last tooltip, which takes the pointer, may cover the widget.
        for _ in 0..3 {
            h.frame(vec![Event::PointerMoved(Pos2::new(1.0, 1.0))]);
        }
        let out = h.frame(vec![]);
        let tree = out
            .platform_output
            .accesskit_update
            .expect("accesskit is on");
        let node = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label) || n.value() == Some(label));
        let b = node
            .and_then(|(_, n)| n.bounds())
            .unwrap_or_else(|| panic!("no «{label}»"));
        let centre = Pos2::new((b.x0 + b.x1) as f32 / 2.0, (b.y0 + b.y1) as f32 / 2.0);
        h.frame(vec![Event::PointerMoved(centre)]);
        for _ in 0..5 {
            h.frame(vec![]);
        }
        texts_of(h)
    }

    #[test]
    fn the_backstory_comes_before_the_first_move() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        assert!(h.app.intro);
        let intro = h.app.presets[0].intro.clone();
        let sentences = intro.matches(". ").count() + 1;
        assert!((3..=5).contains(&sentences), "{intro}");
        let shown = texts_of(&mut h);
        assert!(shown.contains(&intro) && shown.contains(&"Предыстория".to_string()));
        assert!(shown.iter().any(|t| t.contains("Подождать год")));
        // The card holds the screen: a click on «Подождать год» does nothing.
        h.click_label("Подождать год ▸");
        assert_eq!(h.game().world.tick.0, 0);
        h.click_label("Править");
        assert!(!h.app.intro);
        h.click_label("Подождать год ▸");
        assert_eq!(h.game().world.tick.0, 1);
        // A link opens straight into the game.
        let mut other = Harness::new();
        other.app.open(&h.app.link());
        assert!(!other.app.intro && other.game().world.tick.0 == 1);
    }

    #[test]
    fn hovering_tells_what_actions_neighbours_and_the_law_do() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let road = hover(&mut h, "Проложить дорогу");
        for t in [
            "Стоимость 60 · 2 года",
            "сила короны в провинции +5",
            "доход провинции +1",
            "Нужна сила короны от 20",
        ] {
            assert!(road.contains(&t.to_string()), "{t}: {road:?}");
        }
        // A widowed king may wed; Веструм agrees for sure.
        let g = h.app.game.as_mut().unwrap();
        g.world.flags.remove("married");
        g.data.marriage.percent = Fx::from_int(100);
        let marriage = hover(&mut h, "Заключить брачный союз");
        assert!(
            marriage.contains(&"согласие: отношения +30".to_string())
                && marriage.contains(&"отказ: Престиж -5".to_string()),
            "{marriage:?}"
        );
        assert!(
            marriage
                .iter()
                .any(|t| t.starts_with("Посвататься к соседнему двору"))
        );
        // The chance of every court before the suit (here a flat 100), Нордмарк at -40 none.
        for t in [
            "Шанс согласия: Пурпуляндия 99%, Веструм 100%",
            "Сватов не примут: Нордмарк",
        ] {
            assert!(marriage.contains(&t.to_string()), "{t}: {marriage:?}");
        }

        let law = hover(&mut h, "Закон: Абсолютное первородство (?)");
        let text = h.game().data.heirs.laws[0].text();
        assert!(law.contains(&text) && text.contains("ниже 70"), "{law:?}");
        assert!(texts_of(&mut h).contains(&"Первый в очереди: Конрад".to_string()));

        let n = hover(&mut h, "Веструм");
        for t in [
            "Отношение +40: друг",
            "Сила 45, торгует",
            "Союзов и браков нет",
        ] {
            assert!(n.contains(&t.to_string()), "{t}: {n:?}");
        }
        let vestrum = Target::Neighbour(NeighbourId("vestrum".into()));
        h.app
            .apply(Cmd::Act("marry_neighbour".into(), Some(vestrum)));
        h.app.apply(Cmd::Wait);
        while let Screen::Event(_) = h.app.screen {
            h.app.apply(Cmd::Choose(0));
        }
        let n = hover(&mut h, "Веструм ♥");
        // Dated by the wedding, a year after the suit.
        assert!(n.contains(&"брачный союз с 1188".to_string()), "{n:?}");
    }

    /// Stage 17: «Назначить наследника» tells who the law names and what naming another
    /// costs; the heirs to pick from are named by id (not by index) with the law's verdict.
    #[test]
    fn designating_an_heir_tells_the_law() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let g = h.app.game.as_mut().unwrap();
        g.world.heirs.clear();
        for name in ["Ада", "Бруно"] {
            let mut x = g.data.new_heir.clone();
            (x.name, x.age) = (name.into(), 10);
            g.world.add_heir(x);
        }
        let tip = hover(&mut h, "Назначить наследника");
        assert!(
            tip.contains(&"По закону престол наследует: Ада".to_string()),
            "{tip:?}"
        );
        let penalty = "Назначить другого: Легитимность -10, Знать -5, Церковь -5";
        assert!(tip.iter().any(|t| t.starts_with(penalty)), "{tip:?}");
        h.click_label("Назначить наследника");
        let shown = texts_of(&mut h);
        for t in ["Ада · по закону", "Бруно · в обход закона"] {
            assert!(shown.contains(&t.to_string()), "{t}: {shown:?}");
        }
        h.click_label("Бруно · в обход закона");
        h.app.apply(Cmd::Wait);
        let shown = texts_of(&mut h);
        assert!(
            shown.contains(&"Первый в очереди: Бруно".to_string()),
            "{shown:?}"
        );
        assert!(
            shown.contains(&"назначен".to_string()) && shown.contains(&"по закону".to_string())
        );
    }

    /// Acceptance (stage 16): a law changes by mouse: «Сменить закон», the list with every
    /// other law's text and price, a pick; the law is in force once the years pass.
    #[test]
    fn the_law_changes_from_its_list() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let row = texts_of(&mut h);
        assert!(
            !row.iter().any(|t| t.starts_with("Ввести закон")),
            "{row:?}"
        );
        h.click_label("Сменить закон");
        assert!(h.app.laws);
        // A card sizes itself in its first frame.
        h.frame(vec![]);
        let list = texts_of(&mut h);
        let d = h.game().data.clone();
        for l in d.heirs.laws.iter().skip(1) {
            assert!(
                list.contains(&l.name) && list.contains(&l.text()),
                "{}: {list:?}",
                l.name
            );
        }
        let current = &d.heirs.laws[0].name;
        assert!(!list.contains(current), "the law in force is not offered");
        for t in [
            "Стоимость 45 · 2 года · сила короны от 40",
            "пока вводят, к цели: Церковь -10 · по введении: Знать +3, Церковь -2",
        ] {
            assert!(list.iter().any(|x| x.starts_with(t)), "{t}: {list:?}");
        }
        h.click_label("Салический закон");
        assert!(!h.app.laws);
        let running = &h.game().world.active_actions;
        assert_eq!(running[0].id, "enact_law_salic");
        for _ in 0..2 {
            h.app.apply(Cmd::Wait);
            while let Screen::Event(_) = h.app.screen {
                h.app.apply(Cmd::Choose(0));
            }
        }
        assert!(texts_of(&mut h).contains(&"Закон: Салический закон (?)".to_string()));
        // While the throne is disputed, nothing to pick and the reason said.
        h.app
            .game
            .as_mut()
            .unwrap()
            .world
            .flags
            .insert("succession_contested".into());
        h.click_label("Сменить закон");
        h.frame(vec![]);
        let list = texts_of(&mut h);
        assert!(list.contains(&"Пока идёт спор о престоле, закон не сменить.".to_string()));
        h.click_label("Мужское первородство");
        assert!(h.app.laws && h.game().world.active_actions.is_empty());
        h.click_label("Отмена");
        assert!(!h.app.laws);
    }

    #[test]
    fn the_reign_ends_with_its_summary_then_the_chronicle() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(7));
        h.app.apply(Cmd::Begin);
        h.app.apply(Cmd::Act(
            "grant_province".into(),
            Some(Target::Province(ProvinceId("gart".into()))),
        ));
        while !matches!(h.app.screen, Screen::ReignOver) {
            match h.app.screen {
                Screen::Event(_) => h.app.apply(Cmd::Choose(0)),
                _ => h.app.apply(Cmd::Wait),
            }
        }
        let shown = texts_of(&mut h);
        let (c, _) = h.app.dynasty.as_ref().unwrap();
        for t in [
            "Итог правления",
            "Ключевые решения",
            "Земли",
            "Состояние",
            "Наследник",
        ] {
            assert!(
                shown.contains(&t.to_uppercase()) || shown.contains(&t.to_string()),
                "{t}"
            );
        }
        assert!(
            shown.contains(&"Гарт отошла: вассал Вейр".to_string()),
            "{shown:?}"
        );
        let heir = format!("На престол взошёл {}", c.rulers[1].name);
        assert!(shown.iter().any(|t| t.starts_with(&heir)), "{heir}");
        h.click_label("К хронике ▸");
        assert!(matches!(h.app.screen, Screen::Chronicle) && h.app.entry == 0);
    }

    #[test]
    fn the_family_tree_shows_the_dynasty() {
        let mut h = Harness::new();
        six_years(&mut h);
        h.click_label("Родословная");
        let shown = texts_of(&mut h);
        for t in [
            "♔ Ульрих (р. 1155), правил с 1187",
            "Конрад (1181–1188)",
            "Генрих (р. 1189)",
        ] {
            assert!(shown.contains(&t.to_string()), "{t}: {shown:?}");
        }
        // Children under their parent, deeper.
        let kin = &h.game().world.kin;
        assert_eq!(
            chronicle::family(kin),
            [(0, 0), (1, 1), (2, 1), (3, 1), (4, 1)]
        );
        h.click_label("Закрыть");
        assert!(!h.app.tree);

        // After the dynasty: the rulers from the chronicle, the first ones on screen (a long
        // dynasty scrolls).
        play(&mut h, 7);
        h.click_label("Родословная");
        let shown = texts_of(&mut h);
        let c = &h.app.dynasty.as_ref().unwrap().0;
        assert!(c.rulers.len() > 1);
        for r in &c.rulers[..2] {
            let crowned = format!("♔ {} (", r.name);
            assert!(shown.iter().any(|t| t.starts_with(&crowned)), "{crowned}");
        }
        assert_eq!(chronicle::family(&c.kin).len(), c.kin.len());
    }

    /// Stage 17: bastards show beside the heirs and in the family tree, out of the line.
    #[test]
    fn bastards_show_out_of_the_line() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let g = h.app.game.as_mut().unwrap();
        let mut b = g.data.new_heir.clone();
        (b.name, b.bastard) = ("Ольга".into(), true);
        g.world.add_heir(b);
        let shown = texts_of(&mut h);
        assert!(
            shown.contains(&"Бастарды: Ольга, 0".to_string()),
            "{shown:?}"
        );
        assert!(shown.contains(&"Первый в очереди: Конрад".to_string()));
        h.click_label("Родословная");
        let shown = texts_of(&mut h);
        assert!(
            shown.contains(&"Ольга (р. 1187, бастард)".to_string()),
            "{shown:?}"
        );
    }

    /// Stage 17 bugs: an heir target is named by its id, not its index; a ruler who gave up
    /// the crown has the year of it in the tree; a state breaking away does not repaint
    /// the others.
    #[test]
    fn heir_names_abdication_years_and_state_colours_hold() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        let g = h.app.game.as_mut().unwrap();
        let mut x = g.data.new_heir.clone();
        x.name = "Ада".into();
        g.world.add_heir(x);
        g.world.heirs.remove(0);
        assert_eq!(target_name(&g.world, &Target::Heir(1)), "Ада");

        let colour =
            |w: &World, n: &str| map::holder_color(w, &Holder::Foreign(NeighbourId(n.into())));
        let states = ["nordmark", "purpur", "vestrum"];
        let before = states.map(|n| colour(&g.world, n));
        // Арден, whose id sorts before every state, breaks away.
        let mut queue = vec![];
        let mut ctx = bd_core::rules::Ctx {
            data: &g.data,
            queue: &mut queue,
            target: None,
            neighbour: None,
        };
        let land = (g.world.provinces.values())
            .find(|p| p.holder == Holder::Vassal(bd_core::state::VassalId("arden".into())))
            .map(|p| p.id.clone())
            .unwrap();
        let secede = Effect::Secede(bd_core::rules::ProvinceTarget::ById(land));
        secede.apply(&mut g.world, &mut ctx);
        assert!(
            g.world
                .neighbours
                .contains_key(&NeighbourId("arden".into()))
        );
        assert_eq!(states.map(|n| colour(&g.world, n)), before);
        assert!(!before.contains(&colour(&g.world, "arden")));

        h.app.apply(Cmd::Abdicate);
        let id = h.app.data.abdication.event.clone();
        let ev = h.app.data.events.iter().find(|e| e.id == id).unwrap();
        let confirm = (ev.choices.iter())
            .position(|c| c.effects.contains(&Effect::Abdicate))
            .unwrap();
        h.app.apply(Cmd::Choose(confirm));
        (h.app.screen, h.app.tree) = (Screen::Chronicle, true);
        let shown = texts_of(&mut h);
        let founder = "♔ Ульрих (р. 1155, отрёкся в 1187)";
        assert!(shown.iter().any(|t| t.starts_with(founder)), "{shown:?}");
    }

    /// Line segments painted in the holder border colour, in map coordinates.
    fn borders(h: &Harness, out: &egui::FullOutput) -> Vec<[(i32, i32); 2]> {
        let map = &h.app.map;
        (out.shapes.iter())
            .filter_map(|c| match &c.shape {
                egui::Shape::LineSegment { points, stroke } if stroke.color == map::BORDER => {
                    let p = points.map(|p| map.to_map(p));
                    let mut e = p.map(|p| (p.x.round() as i32, p.y.round() as i32));
                    e.sort();
                    Some(e)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_map_outlines_every_realm_and_names_the_states() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.app.apply(Cmd::Begin);
        let out = h.frame(vec![]);
        let lines = borders(&h, &out);
        // capital–holm: crown against vassal; capital–berg: crown on both sides.
        assert!(lines.contains(&[(160, 110), (210, 100)]));
        assert!(!lines.contains(&[(150, 150), (160, 110)]));
        let shown = texts(&out);
        for t in [
            "НОРДМАРК",
            "ПУРПУЛЯНДИЯ",
            "ВЕСТРУМ",
            "вассал Вейр",
            "вассал Арден",
            "Нордмарк",
            "граница владений",
        ] {
            assert!(shown.contains(&t.to_string()), "{t}");
        }
        // Granted away, capital–berg becomes a border.
        let g = h.app.game.as_mut().unwrap();
        g.world
            .provinces
            .get_mut(&ProvinceId("berg".into()))
            .unwrap()
            .holder = Holder::Vassal(bd_core::state::VassalId("weir".into()));
        let out = h.frame(vec![]);
        assert!(borders(&h, &out).contains(&[(150, 150), (160, 110)]));
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
        assert!(matches!(h.app.screen, Screen::ReignOver));
        let c = &h.app.dynasty.as_ref().unwrap().0;
        assert_eq!(c.rulers[0].cause.as_deref(), Some(id.as_str()));
        assert_eq!(chronicle::reign_end(&h.app.data, &id), "отречение");
        h.frame(vec![]);
    }

    #[test]
    fn map_click_picks_the_target() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.app.apply(Cmd::Begin);
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

    /// Outlines painted in this colour and width: the war target's on the map.
    fn outlines(out: &egui::FullOutput, color: egui::Color32, width: f32) -> usize {
        (out.shapes.iter())
            .filter(|c| match &c.shape {
                egui::Shape::Path(p) => match &p.stroke.color {
                    egui::epaint::ColorMode::Solid(c) => *c == color && p.stroke.width == width,
                    _ => false,
                },
                _ => false,
            })
            .count()
    }

    /// Stage 14: a war by the mouse. The target picked on the map, the war's flag on the top
    /// bar, its summary on the side, the target marked on the map, a war action from its
    /// button; the war ends and its marks go.
    #[test]
    fn a_war_is_fought_with_the_mouse() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        assert!(!texts_of(&mut h).iter().any(|t| t.starts_with("⚔")));
        assert!(!texts_of(&mut h).contains(&"Набрать войско".to_string()));
        h.click_label("Объявить войну");
        let Some((_, targets)) = &h.app.picking else {
            panic!("a target to pick")
        };
        assert!(targets.contains(&Target::Province(ProvinceId("skala".into()))));
        assert!(texts_of(&mut h).contains(&"Скала, Нордмарк".to_string()));
        h.click(h.province_on_screen("skala"));
        assert_eq!(h.game().world.active_actions[0].id, "declare_war");
        h.click_label("Подождать год ▸");
        let Screen::Event(v) = &h.app.screen else {
            panic!("the war's first event")
        };
        assert_eq!(v.event_id, "war_declared");
        h.click_label("Созвать вассалов");
        assert!(matches!(h.app.screen, Screen::Reign));
        let war = h.game().world.war.clone().unwrap();
        assert_eq!(war.target, Some(ProvinceId("skala".into())));

        let out = h.frame(vec![]);
        let shown = texts(&out);
        for t in [
            "⚔ Война: Нордмарк",
            "ВОЙНА",
            "⚔ Нордмарк, 1-й год войны",
            "Цель: Скала, Нордмарк",
            "счёт 0: равенство",
            "Битв ещё не было",
            "⚔ Скала",
            "⚔ цель войны",
        ] {
            assert!(shown.contains(&t.to_string()), "{t}: {shown:?}");
        }
        assert!(shown.iter().any(|t| t.starts_with("Силы: наши ")));
        assert_eq!(outlines(&out, map::RUBRIC, 4.0), 1);
        // War actions act at the enemy at once, no target to pick.
        h.click_label("Набрать войско");
        assert!(h.app.picking.is_none());
        let running = &h.game().world.active_actions;
        assert_eq!(running[0].id, "war_recruit");
        assert_eq!(running[0].target.as_deref(), Some("nordmark"));
        // On to the peace by the year button and the first choice of every card.
        for _ in 0..10 {
            match h.app.screen {
                Screen::Event(_) => h.app.apply(Cmd::Choose(0)),
                _ => {
                    h.click_label("Подождать год ▸");
                }
            }
            if h.game().world.war.is_none() {
                break;
            }
            let shown = texts_of(&mut h);
            if !h.game().world.war.as_ref().unwrap().battles.is_empty() {
                assert!(
                    shown.iter().any(|t| t.contains(", последние: ")),
                    "{shown:?}"
                );
            }
        }
        assert!(h.game().world.war.is_none());
        let out = h.frame(vec![]);
        assert!(!texts(&out).iter().any(|t| t.starts_with("⚔")));
        assert_eq!(outlines(&out, map::RUBRIC, 4.0), 0);
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
        h.app.apply(Cmd::Begin);
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
                Effect::Tribute(Fx::from_int(-30)),
                Effect::Clash,
                Effect::Clash,
            ],
        );
        let want = [
            ("Знать +15", Some(true)),
            ("Казна -360", Some(false)),
            ("no_name +1.5", Some(true)),
            ("риск", None),
            ("провинция уходит вассалу", None),
            ("лояльность провинции +0.001", Some(true)),
            ("Казна -30", Some(false)),
            ("битва", None),
        ];
        assert_eq!(shown, want.map(|(s, up)| (s.to_string(), up)));
        assert!(effects(&d, &[Effect::RulerHealth(Fx(1))]).is_empty());
        let end = effects(&d, &[Effect::Abdicate]);
        assert_eq!(end, [("конец правления".to_string(), Some(false))]);
        // What a road gives: its crown power bonus from rules.ron and its income.
        let road = d.actions.iter().find(|a| a.id == "build_road").unwrap();
        let shown = effects(&d, &road.on_complete);
        let want = [
            ("сила короны в провинции +5", Some(true)),
            ("доход провинции +1", Some(true)),
        ];
        assert_eq!(shown, want.map(|(s, up)| (s.to_string(), up)));
        let war = d.actions.iter().find(|a| a.id == "declare_war").unwrap();
        let shown: Vec<String> = effects(&d, &war.on_complete)
            .into_iter()
            .map(|l| l.0)
            .collect();
        assert_eq!(
            shown[..3],
            [
                "если сосед — друг: Престиж -15",
                "если сосед — друг: отношения с другими соседями -10",
                "война"
            ]
        );
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

    /// Slow: builds the release wasm and tells its size. `cargo test -p ui -- --ignored`.
    /// Counts what the browser loads: custom sections (names, wasm-bindgen's own) are
    /// stripped by wasm-bindgen in `trunk build --release`. The 8 MiB limit is soft: over
    /// it, a warning, not a failure.
    #[test]
    #[ignore]
    fn wasm_release_size() {
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
        if size > 8 * 1024 * 1024 {
            eprintln!("warning: ui.wasm over 8 MiB: {size} bytes");
        }
    }
}
