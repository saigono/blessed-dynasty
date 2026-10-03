//! egui front end: start, reign, event, chronicle, score. Every rule lives in `bd_core`; this
//! only shows and asks.

mod chronicle;
mod map;

use bd_core::data::{AxisDef, Data, ENACT, LawDef, REPEAL};
use bd_core::fx::Fx;
use bd_core::game::{EventView, Game, GameError, Step};
use bd_core::graph::{self, Push, Sight};
use bd_core::link;
use bd_core::rng::Rng;
use bd_core::rules::{Action, ActionTarget, Effect, HeirOp, Predicate, ProvinceField, Target};
use bd_core::score::{self, Score, ScoreRules};
use bd_core::sim::{self, Chronicle};
use bd_core::state::{
    AxisId, Change, HeirStatus, Holder, Neighbour, NeighbourId, Preset, ProvinceId, Sex, Stance,
    World,
};
use bd_core::testament::{Order, Testament};
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
const EVENTS: [&str; 7] = [
    include_str!("../../../data/events/death.ron"),
    include_str!("../../../data/events/heirs.ron"),
    include_str!("../../../data/events/neighbours.ron"),
    include_str!("../../../data/events/omens.ron"),
    include_str!("../../../data/events/reign.ron"),
    include_str!("../../../data/events/stories.ron"),
    include_str!("../../../data/events/war.ron"),
];
/// Every file of data/events/sim, in file name order.
const SIM_EVENTS: [&str; 2] = [
    include_str!("../../../data/events/sim/sim.ron"),
    include_str!("../../../data/events/sim/testament.ron"),
];
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
    /// Open the testament card with this draft, keep its edits, or close it (stage 24).
    Will(Option<Testament>),
    /// Seal the testament: `Game::write_testament`.
    WriteWill(Testament),
    /// The game saved (`App.saved`) again (stage 25).
    Continue,
}

/// A line of the journal or of an effect list; `Some(true)` good, `Some(false)` bad.
type Line = (String, Option<bool>);

/// Before the title of a symptom of the graph (`Event.omen`), in the year's summary and the
/// chronicle.
const OMEN: &str = "Знамение. ";

/// The start of the treasury line every year of the journal has.
const MONEY: &str = "Казна за год";

/// Over the numbers of an entry of the chronicle, shown on hover or a click.
const NUMBERS: &str = "в цифрах ℹ";

/// The header of the numbers of a year of the journal, open by a click (stage 25).
const YEAR_NUMBERS: &str = "Изменения за год";

/// Over the messages of a year in the journal (stage 26b).
const NEWS: &str = "Вести";

/// After a word or a number with a tip, the same mark all over the game (stage 25).
const INFO: &str = "ℹ";

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
    /// The testament card is up with its draft (stage 24).
    will: Option<Testament>,
    /// What happened, year by year, oldest first: the date, the choices made, the rest (the
    /// news in words and the numbers).
    journal: Vec<(String, Vec<Line>, Vec<Line>)>,
    /// The world when «Подождать год» was pressed, and the choices made since: the year's
    /// record in the making.
    year_start: Option<World>,
    chosen: Vec<Line>,
    /// The messages of the year in the making (`Event::is_message`), not asked (stage 26b).
    news: Vec<Line>,
    /// The event told last this year: in the news or not, its id and target. A linked event
    /// after it joins its line (`sim::fuse`, stage 26c).
    told: Option<(bool, String, Option<Target>)>,
    /// The treasury before the latest year's tick, that year's income and upkeep
    /// (`war::income_parts`): its «Казна:» line.
    money: Option<(Fx, Fx, Fx)>,
    /// The link (`link::encode`) of the unfinished game, saved once a year (stage 25).
    saved: Option<String>,
    /// `saved` goes to the browser's localStorage or a file natively; off in tests.
    persist: bool,
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
        // Tips at once (stage 25): a half-second wait read as no tip at all.
        ctx.global_style_mut(|s| s.interaction.tooltip_delay = 0.1);

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
            will: None,
            journal: Vec::new(),
            year_start: None,
            chosen: Vec::new(),
            news: Vec::new(),
            told: None,
            money: None,
            saved: None,
            persist: false,
        }
    }

    /// `App::new` with the saved game kept where the platform keeps it.
    fn persistent(ctx: &egui::Context) -> App {
        let mut app = App::new(ctx);
        (app.persist, app.saved) = (true, load_save());
        app
    }

    /// Keeps `text` as the saved game, or none.
    fn save(&mut self, text: Option<String>) {
        if self.persist {
            write_save(text.as_deref());
        }
        self.saved = text;
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
            Cmd::Continue => {
                let text = self.saved.clone().unwrap_or_default();
                if let Err(e) = self.replay(&text) {
                    (self.game, self.screen) = (None, Screen::Start);
                    self.note = format!("Сохранение не открылось: {e}");
                    self.save(None);
                }
                return;
            }
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
            Cmd::Will(draft) => {
                (self.will, self.picking) = (draft, None);
                return;
            }
            Cmd::WriteWill(_) => self.will = None,
            Cmd::Choose(i) => {
                if let Screen::Event(v) = &self.screen
                    && let Some(c) = v.choices.get(i)
                {
                    // A symptom of a loop of the graph (stage 20) is marked so. The choice
                    // as the chronicle tells it (stage 22), before it is made.
                    let events = self.game.as_ref().map_or(&[][..], |g| &g.data.events);
                    let omen = events.iter().any(|e| e.id == v.event_id && e.omen);
                    let mark = if omen { OMEN } else { "" };
                    let told = (self.game.as_ref().and_then(|g| g.told(i)))
                        .unwrap_or_else(|| format!("«{}»: {}", v.title, c.text));
                    let (id, target) = (v.event_id.clone(), v.target.clone());
                    self.tell(false, id, target, format!("{mark}{told}"));
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
            Cmd::WriteWill(t) => {
                // The rumours it stirs, told in the year's record.
                let rumour = !bd_core::testament::cost(&g.data, &g.world).is_empty();
                let res = g.write_testament(t).map(|_| Step::Idle);
                if res.is_ok() {
                    let tx = g.data.testament.as_ref().map(|r| &r.texts);
                    let said = tx.map_or(String::new(), |tx| {
                        bd_core::testament::fill(&g.data, &g.world, &tx.sealed)
                    });
                    self.chosen.push((said, None));
                    if let Some(tx) = tx.filter(|_| rumour) {
                        self.chosen.push((tx.rumour.clone(), Some(false)));
                    }
                }
                res
            }
            Cmd::Will(_)
            | Cmd::Start(_)
            | Cmd::Restart
            | Cmd::NewSeed
            | Cmd::Entry(_)
            | Cmd::Summary
            | Cmd::CopyLink
            | Cmd::Begin
            | Cmd::Tree(_)
            | Cmd::Laws(_)
            | Cmd::Continue => {
                unreachable!("handled above")
            }
        };
        // A message is not asked: it goes into the year's news (stage 26b).
        let mut res = res;
        while let Ok(Step::Event(v)) = &res
            && let Some(g) = self.game.as_ref()
            && (g.data.events.iter()).any(|e| e.id == v.event_id && e.is_message())
        {
            let told = g.told(0).unwrap_or_else(|| v.title.clone());
            let (id, target) = (v.event_id.clone(), v.target.clone());
            self.tell(true, id, target, told);
            let g = self.game.as_mut().expect("in a game");
            res = match g.choose(0) {
                Ok(()) if g.ended.is_some() => g.wait(),
                r => r.map(|_| Step::Idle),
            };
        }
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

    /// «Казна за год +N: доход +X, расходы -Y, действия и события -Z» first in the latest
    /// year's record: N from before its tick to now, the last what the actions and choices
    /// since took, left out when nothing.
    fn money_line(&mut self) {
        let (Some(g), Some((start, income, upkeep))) = (&self.game, self.money) else {
            return;
        };
        let Some((_, _, lines)) = self.journal.last_mut() else {
            return;
        };
        let now = g.world.axes[&g.data.economy.treasury];
        let spent = now - start - income + upkeep;
        let mut text = format!(
            "{MONEY} {}: доход {}, расходы {}",
            plus(now - start),
            plus(income),
            plus(Fx(0) - upkeep),
        );
        // Nothing spent or got by the actions and the choices: no «0» to puzzle over.
        if round(spent) != "0" && round(spent) != "-0" {
            text += &format!(", действия и события {}", plus(spent));
        }
        lines.retain(|(t, _)| !t.starts_with(MONEY));
        lines.insert(0, (text, Some(now >= start)));
    }

    /// Puts what happened since «Подождать год» into the journal: the choices made, then
    /// the messages and what changed. A second record of the same date joins the first.
    /// `told` of event `id` into the year's news or choices, or joined to the line of the
    /// event told before it this year when the two are linked (`sim::fuse`).
    fn tell(&mut self, news: bool, id: String, target: Option<Target>, told: String) {
        let g = self.game.as_ref().expect("in a game");
        let tick = g.world.tick;
        let salt = g.rng.clone().next_u64();
        let fused = self.told.as_ref().and_then(|(in_news, e, t)| {
            let lines = if *in_news { &self.news } else { &self.chosen };
            let told_as = |event, target, text| sim::Told {
                event,
                target,
                tick,
                root: None,
                text,
            };
            let a = told_as(e.as_str(), t.as_ref(), &lines.last()?.0);
            let b = told_as(&id, target.as_ref(), &told);
            let (_, text) = sim::fuse(&g.data, &g.world, &a, &b, salt, tick.0 as u64)?;
            Some((*in_news, text))
        });
        match fused {
            Some((in_news, text)) => {
                let lines = if in_news { &mut self.news } else { &mut self.chosen };
                lines.last_mut().expect("joined to it").0 = text;
                self.told = None;
            }
            None => {
                let lines = if news { &mut self.news } else { &mut self.chosen };
                lines.push((told, None));
                self.told = Some((news, id, target));
            }
        }
    }

    fn close_year(&mut self) {
        self.told = None;
        let g = self.game.as_ref().expect("in a game");
        let chosen = std::mem::take(&mut self.chosen);
        let mut lines = std::mem::take(&mut self.news);
        if let Some(before) = self.year_start.take() {
            lines.extend(change_lines(g, &before, &g.world));
        }
        let date = g.world.tick.date(g.world.time_unit, g.world.start_year);
        match self.journal.last_mut() {
            Some((d, c, l)) if *d == date => {
                c.extend(chosen);
                l.extend(lines);
            }
            _ => self.journal.push((date, chosen, lines)),
        }
        // Once a year, the game so far (stage 25); a reign over is no game to go on with.
        if g.ended.is_none() {
            let text = link::encode(PRESETS[self.preset].0, self.played, g);
            self.save(Some(text));
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
            let end = g.reign_end(cause);
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
        (self.laws, self.will) = (false, None);
        (self.journal, self.chosen, self.money) = (Vec::new(), Vec::new(), None);
        (self.news, self.told) = (Vec::new(), None);
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
                self.save(None);
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
            Screen::Reign if self.will.is_some() => {
                self.reign(ui);
                let draft = self.will.as_ref().expect("checked");
                testament(&ctx, self.game.as_ref().expect("in a game"), draft)
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
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(seed.is_some(), Button::new("Новая партия"))
                    .clicked()
                {
                    cmd = seed.map(Cmd::Start);
                }
                // The game left unfinished, saved once a year (stage 25).
                if self.saved.is_some() && ui.button("Продолжить партию").clicked()
                {
                    cmd = Some(Cmd::Continue);
                }
            });
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
            // A panel, not a bottom-up layout: the legend wraps into rows that must stay visible.
            egui::Panel::bottom("legend")
                .resizable(false)
                .frame(egui::Frame::NONE)
                .show(ui, |ui| map::legend(ui, &g.world, &g.data));
            let clicked = self.map.show(ui, &g.world, &g.data, &marked);
            if let (Some(id), Some((action, _))) = (clicked, &self.picking)
                && marked.contains(&id)
            {
                cmd = Some(Cmd::Act(action.clone(), Some(Target::Province(id))));
            }
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
            // What a year brings, net, apart from the treasury itself (stage 25).
            let income = bd_core::war::yearly_income(w, d);
            let year = RichText::new(format!("за год {} {INFO}", plus(income))).color(FG);
            tip_label(ui, year, |ui| money_tip(ui, g));
            key_rtl(ui, "Казна", &round(w.axes[&d.economy.treasury]));
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
                    // The laws to bring in or repeal go to a card of their own.
                    if !d.laws.list.is_empty()
                        && ui.button("Ввести закон").on_hover_text(LAWS_TIP).clicked()
                    {
                        cmd = Some(Cmd::Laws(true));
                    }
                    // Every action, those not to be had now grey with the reason in their
                    // tip (stage 25); the war's own only at war.
                    let open = g.available_actions();
                    let shown = (d.actions.iter())
                        .filter(|a| !a.id.starts_with(ENACT) && !a.id.starts_with(REPEAL))
                        .filter(|a| a.target != ActionTarget::Enemy || w.war.is_some());
                    for a in shown {
                        let targets = open.iter().find(|(id, _)| *id == a.id).map(|x| &x.1);
                        let name = action_name(w, d, &a.id);
                        if let Some(why) = why_not(g, a, targets.is_some()) {
                            let grey = Button::new(RichText::new(name).color(FG2)).fill(BG2);
                            tip(ui.add(grey), |ui| {
                                action_tip(ui, g, a);
                                ui.label(RichText::new(&why).color(RUBRIC));
                            });
                            continue;
                        }
                        let targets = targets.cloned().unwrap_or_default();
                        let button = ui.button(name).on_hover_ui(|ui| action_tip(ui, g, a));
                        // A war action has one target, the enemy: no choice to make.
                        let enemy = a.target == ActionTarget::Enemy;
                        if button.clicked() {
                            let id = a.id.clone();
                            cmd = Some(match (targets.is_empty(), enemy) {
                                (true, _) => Cmd::Act(id, None),
                                (false, true) => Cmd::Act(id, targets.first().cloned()),
                                (false, false) => Cmd::Pick(id, targets),
                            });
                        }
                    }
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
                    let abdicate = Button::new(RichText::new("Отречься").color(RUBRIC));
                    let abdicate = ui.add(abdicate.stroke((1.0, RUBRIC)));
                    if abdicate.on_hover_text(ABDICATE_TIP).clicked() {
                        cmd = Some(Cmd::Abdicate);
                    }
                });
            }
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(running(g)).color(FG2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let wait = Button::new(RichText::new("Подождать год ▸").color(BG)).fill(FG);
                if ui.add(wait).on_hover_text(WAIT_TIP).clicked() {
                    cmd = Some(Cmd::Wait);
                }
                let link = ui.button("Скопировать ссылку").on_hover_text(LINK_TIP);
                if link.clicked() {
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

/// «Ввести закон»: every law of `Data.laws` by its group, with what it holds and what it
/// feeds in words, its price, years and resistance; a law in force says so and offers its
/// repeal; those not to be had now are greyed out.
fn laws(ctx: &egui::Context, g: &Game) -> Option<Cmd> {
    let (w, d) = (&g.world, &g.data);
    let mut cmd = None;
    let open: Vec<String> = (g.available_actions().into_iter())
        .map(|(id, _)| id)
        .collect();
    let mut groups: Vec<&str> = vec![];
    for l in &d.laws.list {
        if !groups.contains(&l.group.as_str()) {
            groups.push(&l.group);
        }
    }
    egui::Modal::new(egui::Id::new("laws")).show(ctx, |ui| {
        ui.set_width(640.0);
        ui.label(RichText::new("Ввести закон").size(20.0).strong());
        ui.label("Закон вводят годами, за деньги и против воли недовольной фракции. Он переживает правителя; отмена стоит половину цены.");
        if w.flags.contains(&d.abdication.contested_flag) {
            let busy = "Пока идёт спор о престоле, закон о престоле не сменить.";
            ui.label(RichText::new(busy).color(RUBRIC));
        }
        let height = ctx.content_rect().height() * 0.7;
        let scroll = egui::ScrollArea::vertical().max_height(height);
        // A card grows from its last size; a long list takes its height at once.
        scroll.min_scrolled_height(height).show(ui, |ui| {
            // A group a header, the first one open.
            for (k, group) in groups.iter().enumerate() {
                let name = if group.is_empty() { "Прочие законы" } else { group };
                let header = egui::CollapsingHeader::new(RichText::new(name).strong());
                header.default_open(k == 0).show(ui, |ui| {
                for l in d.laws.list.iter().filter(|l| l.group == *group) {
                    ui.separator();
                    let enact = format!("{ENACT}{}", l.id);
                    let repeal = format!("{REPEAL}{}", l.id);
                    // Why it cannot be brought in now (stage 25); a law in force says so.
                    let def = d.actions.iter().find(|a| a.id == enact);
                    let why = def.and_then(|a| why_not(g, a, open.contains(&enact)));
                    let why = why.filter(|_| !w.flags.contains(&l.id));
                    ui.horizontal(|ui| {
                        let button = Button::new(RichText::new(&l.name).strong());
                        if ui.add_enabled(why.is_none(), button).clicked() {
                            cmd = Some(Cmd::Act(enact.clone(), None));
                        }
                        if let Some(why) = &why {
                            ui.label(RichText::new(why).color(RUBRIC));
                        }
                        if w.flags.contains(&l.id) {
                            ui.label(RichText::new("действует").color(FG2));
                        }
                        let price = round(l.cost * d.laws.repeal_share);
                        if open.contains(&repeal)
                            && ui.button(format!("Отменить · {price}")).clicked()
                        {
                            cmd = Some(Cmd::Act(repeal.clone(), None));
                        }
                    });
                    ui.label(&l.description);
                    let n = l.years.0;
                    let mut price = format!("Стоимость {} · {n} {}", round(l.cost), years(n));
                    if d.laws.min_crown_power > Fx(0) {
                        price += &format!(" · сила короны от {}", round(d.laws.min_crown_power));
                    }
                    ui.label(RichText::new(price).color(FG2));
                    // «пока вводят, к цели: Церковь -10 · по введении: Знать +3, Церковь -2».
                    let list = |es: &[Effect]| {
                        let lines: Vec<String> = effects(d, es).into_iter().map(|(t, _)| t).collect();
                        lines.join(", ")
                    };
                    let against: Vec<Effect> = (l.resistance.iter())
                        .map(|(a, v)| Effect::Axis(a.clone(), *v))
                        .collect();
                    let yearly = [Effect::Axis(d.economy.treasury.clone(), l.treasury)];
                    let yearly = if l.treasury == Fx(0) { &[][..] } else { &yearly[..] };
                    let parts = [
                        ("пока вводят, к цели", list(&against)),
                        ("по введении", list(&l.on_complete)),
                        ("в год", list(yearly)),
                    ];
                    let parts: Vec<String> = (parts.iter())
                        .filter(|(_, l)| !l.is_empty())
                        .map(|(when, l)| format!("{when}: {l}"))
                        .collect();
                    ui.small(RichText::new(parts.join(" · ")).color(FG2));
                }
                });
            }
        });
        ui.separator();
        if ui.button("Отмена").clicked() {
            cmd = Some(Cmd::Laws(false));
        }
    });
    cmd
}

/// «Завещание» (stage 24): the precept, the order and its object, the heir, what writing it
/// costs now; sealed by «Скрепить печатью» once it names something known.
fn testament(ctx: &egui::Context, g: &Game, draft: &Testament) -> Option<Cmd> {
    let (w, d) = (&g.world, &g.data);
    let r = d.testament.as_ref()?;
    let (mut t, mut cmd) = (draft.clone(), None);
    egui::Modal::new(egui::Id::new("testament")).show(ctx, |ui| {
        ui.set_width(600.0);
        ui.label(RichText::new("Завещание").size(20.0).strong());
        ui.label("Завещание вскроют над гробом государя. Чем громче его слава, тем крепче потомки держатся наказа; с годами он слабеет, а государь с шаткими правами чтит его ревностнее. Переписать можно в любой год.");
        heading(ui, "Заповедь");
        ui.radio_value(&mut t.precept, None, "Без заповеди");
        for p in &r.precepts {
            ui.radio_value(&mut t.precept, Some(p.id.clone()), &p.name);
            ui.small(RichText::new(&p.description).color(FG2));
        }
        heading(ui, "Наказ");
        let kinds = [
            ("Без наказа", None),
            (r.orders.keep_law.name.as_str(), Some(0)),
            (r.orders.keep_province.name.as_str(), Some(1)),
            (r.orders.peace.name.as_str(), Some(2)),
        ];
        let kind = |o: &Option<Order>| match o {
            None => None,
            Some(Order::KeepLaw(_)) => Some(0),
            Some(Order::KeepProvince(_)) => Some(1),
            Some(Order::Peace(_)) => Some(2),
        };
        let crown = || (w.provinces.iter()).filter(|(_, p)| !matches!(p.holder, Holder::Foreign(_)));
        ui.horizontal_wrapped(|ui| {
            for (name, k) in kinds {
                if ui.radio(kind(&t.order) == k, name).clicked() && kind(&t.order) != k {
                    t.order = match k {
                        Some(0) => d.laws.list.first().map(|l| Order::KeepLaw(l.id.clone())),
                        Some(1) => Some(Order::KeepProvince(w.capital.province.clone())),
                        Some(_) => w.neighbours.keys().next().cloned().map(Order::Peace),
                        None => None,
                    };
                }
            }
        });
        let pick = |ui: &mut Ui, order: &mut Option<Order>, now: String, all: Vec<(String, Order)>| {
            egui::ComboBox::from_id_salt("order").selected_text(now).show_ui(ui, |ui| {
                for (name, o) in all {
                    ui.selectable_value(order, Some(o), name);
                }
            });
        };
        match t.order.clone() {
            Some(Order::KeepLaw(l)) => {
                let name = |id: &str| d.law(id).map_or(id.to_string(), |l| {
                    let on = if w.flags.contains(&l.id) { " · действует" } else { "" };
                    format!("{}{on}", l.name)
                });
                let all = (d.laws.list.iter()).map(|l| (name(&l.id), Order::KeepLaw(l.id.clone())));
                pick(ui, &mut t.order, name(&l), all.collect());
            }
            Some(Order::KeepProvince(p)) => {
                let name = |id: &ProvinceId| w.provinces.get(id).map_or(id.0.clone(), |p| p.name.clone());
                let all = crown().map(|(id, _)| (name(id), Order::KeepProvince(id.clone())));
                pick(ui, &mut t.order, name(&p), all.collect());
            }
            Some(Order::Peace(n)) => {
                let name = |id: &NeighbourId| w.neighbours.get(id).map_or(id.0.clone(), |n| n.name.clone());
                let all = w.neighbours.keys().map(|id| (name(id), Order::Peace(id.clone())));
                pick(ui, &mut t.order, name(&n), all.collect());
            }
            None => {}
        }
        heading(ui, "Наследник");
        let rightful = rightful_heir(g).map(|h| h.id);
        let heir = |id: Option<u32>| match id.and_then(|id| w.heir_index(id)) {
            None => "По закону".to_string(),
            Some(i) => {
                let h = &w.heirs[i];
                let how = if rightful == Some(h.id) { "по закону" } else { "в обход закона, тайно" };
                format!("{}, {} {} · {how}", h.name, h.age, years(h.age))
            }
        };
        egui::ComboBox::from_id_salt("heir").selected_text(heir(t.heir)).show_ui(ui, |ui| {
            ui.selectable_value(&mut t.heir, None, heir(None));
            for h in &w.heirs {
                ui.selectable_value(&mut t.heir, Some(h.id), heir(Some(h.id)));
            }
        });
        ui.small(RichText::new("Наследник в завещании запечатан: пока государь жив, закон о престоле не нарушен. Коронация по завещанию оспаривается чаще, чем открытое назначение.").color(FG2));
        ui.separator();
        // What sealing it costs now (testament::cost).
        let cost: Vec<Effect> = (bd_core::testament::cost(d, w).into_iter())
            .map(|(a, v)| Effect::Axis(a, v))
            .collect();
        match cost.is_empty() {
            true => ui.label(RichText::new("Сейчас завещание не встревожит двор: государь стар или болен.").color(FG2)),
            false => {
                let lines: Vec<String> = effects(d, &cost).into_iter().map(|(l, _)| l).collect();
                let text = format!("{} Сейчас: {}. Каждое переписывание — снова.", r.texts.rumour, lines.join(", "));
                ui.label(RichText::new(text).color(RUBRIC))
            }
        };
        ui.horizontal(|ui| {
            let valid = bd_core::testament::valid(d, w, &t);
            let seal = Button::new(RichText::new("Скрепить печатью").color(BG)).fill(FG);
            if ui.add_enabled(valid, seal).clicked() {
                cmd = Some(Cmd::WriteWill(t.clone()));
            }
            if ui.button("Отмена").clicked() {
                cmd = Some(Cmd::Will(None));
            }
        });
    });
    if cmd.is_none() && t != *draft {
        cmd = Some(Cmd::Will(Some(t)));
    }
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
    // How to get more (stage 25); nothing on the last step.
    if let Some(more) = more_slots(d, w) {
        line += &format!(" ({more})");
    }
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

/// What changed from `before` to `w`, as journal lines; the land moved in one line (`lands`).
fn change_lines(g: &Game, before: &World, w: &World) -> Vec<Line> {
    let d = &g.data;
    let mut lines: Vec<Line> = (w.changes(before, d).into_iter())
        .filter_map(|c| match c {
            // Whole numbers: «Знать +7.84» read as noise (stage 26b).
            Change::Axis(a, v) => {
                Some((format!("{} {}", axis_name(d, &a), plus(v)), Some(v > Fx(0))))
            }
            Change::Born(name) => Some((format!("Рождение: {name}"), Some(true))),
            Change::HeirGone(name) => Some(heir_gone(g, before, w, &name)),
            Change::Holder(..) => None,
            Change::Done(id, key) => {
                let def = d.actions.iter().find(|a| a.id == id);
                let what = def.map_or(id.clone(), |a| acted(w, a, key.as_deref()));
                Some((format!("Завершено: {what}"), None))
            }
        })
        .collect();
    lines.extend(lands(d, before, w));
    lines
}

/// «Земли: +2 — короне Ольховка, вассалу Вейр Броды», «Земли: −1 — Нордмарк взял Скалу»
/// (stage 26b): the change in the land of the crown and its vassals, then who got which
/// province; without a change in number (a grant) no number. None when no land moved.
pub(crate) fn lands(d: &Data, before: &World, w: &World) -> Option<Line> {
    let own = |w: &World| {
        let all = w.provinces.values();
        all.filter(|p| !matches!(p.holder, Holder::Foreign(_)))
            .count() as i64
    };
    // Who got them, in the order the provinces come: (holder, names).
    let mut got: Vec<(&Holder, Vec<&str>)> = vec![];
    for p in w.provinces.values() {
        if before
            .provinces
            .get(&p.id)
            .is_none_or(|b| b.holder == p.holder)
        {
            continue;
        }
        match got.iter_mut().find(|(h, _)| **h == p.holder) {
            Some((_, names)) => names.push(&p.name),
            None => got.push((&p.holder, vec![&p.name])),
        }
    }
    if got.is_empty() {
        return None;
    }
    let and = |names: Vec<String>| match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} и {last}", rest.join(", ")),
        _ => names.concat(),
    };
    let parts: Vec<String> = (got.into_iter())
        .map(|(h, names)| {
            let names = names.into_iter();
            match h {
                Holder::Crown => format!("короне {}", and(names.map(String::from).collect())),
                Holder::Vassal(v) => {
                    let house = w.vassals.get(v).map_or(&v.0, |v| &v.name);
                    format!("вассалу {house} {}", and(names.map(String::from).collect()))
                }
                Holder::Foreign(n) => {
                    let n = w.neighbours.get(n).map_or(&n.0, |n| &n.name);
                    let took = bd_core::text::fill("{n:взял|взяла}", &d.names, &[("n", n, None)]);
                    let names = names.map(|p| d.names.declined(p, 3)).collect();
                    format!("{n} {took} {}", and(names))
                }
            }
        })
        .collect();
    let delta = own(w) - own(before);
    let text = match delta {
        0 => format!("Земли: {}", parts.join(", ")),
        _ => format!(
            "Земли: {}{} — {}",
            if delta > 0 { "+" } else { "−" },
            delta.abs(),
            parts.join(", ")
        ),
    };
    Some((text, (delta != 0).then_some(delta > 0)))
}

/// An heir gone from the list, told as the chronicle tells it (`sim.heir_death_age`, aged
/// a year since `before` like there): «Смерть наследника: Конрад»; a child younger is no
/// heir yet, «Умер в детстве королевский сын Генрих» (stage 25). No number in it: the
/// journal keeps the lines with numbers under «Изменения за год».
fn heir_gone(g: &Game, before: &World, w: &World, name: &str) -> Line {
    let gone = (before.heirs.iter()).find(|h| h.name == name && w.heir_index(h.id).is_none());
    let Some(h) = gone.filter(|h| h.age + 1 < g.data.sim.heir_death_age) else {
        return (format!("Смерть наследника: {name}"), Some(false));
    };
    let (baby, own) = (h.age == 0, h.id >= before.line_from);
    let who = match (h.sex, baby, own) {
        (Sex::Male, true, true) => "Умер младенцем королевский сын",
        (Sex::Female, true, true) => "Умерла младенцем королевская дочь",
        (Sex::Male, false, true) => "Умер в детстве королевский сын",
        (Sex::Female, false, true) => "Умерла в детстве королевская дочь",
        (Sex::Male, ..) => "Умер в детстве из боковой линии",
        (Sex::Female, ..) => "Умерла в детстве из боковой линии",
    };
    (format!("{who} {name}"), Some(false))
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

/// The journal, the latest year on top and stressed: the choices, then the news in one
/// block (stage 26b), then the numbers.
fn journal(ui: &mut Ui, journal: &[(String, Vec<Line>, Vec<Line>)]) {
    heading(ui, "Итоги года");
    if journal.is_empty() {
        let hint = "Здесь появится, что случилось за год, после «Подождать год».";
        ui.small(RichText::new(hint).color(FG2));
    }
    for (i, (date, chosen, lines)) in journal.iter().rev().enumerate() {
        if i == 1 {
            heading(ui, "Прежние годы");
        }
        let head = RichText::new(date).strong();
        ui.label(if i == 0 {
            head.size(16.0)
        } else {
            head.color(FG2)
        });
        // Told in words; the lines with numbers (the treasury, the axes) under a header
        // opened by a click (stage 25).
        let (numbers, words): (Vec<_>, Vec<_>) = lines
            .iter()
            .partition(|(t, _)| t.chars().any(|c| c.is_ascii_digit()));
        for (text, up) in chosen {
            ui.small(RichText::new(text).color(tone(*up)));
        }
        if !words.is_empty() {
            ui.small(RichText::new(NEWS).color(FG2).strong());
        }
        for (text, up) in words.iter().copied() {
            ui.small(RichText::new(format!("· {text}")).color(tone(*up)));
        }
        if words.is_empty() && chosen.is_empty() {
            ui.small(RichText::new("Тихий год").color(FG2));
        }
        if !numbers.is_empty() {
            let head = RichText::new(YEAR_NUMBERS).small().color(FG2);
            egui::CollapsingHeader::new(head)
                .id_salt(date)
                .show(ui, |ui| {
                    for (text, up) in numbers {
                        ui.small(RichText::new(text).color(tone(*up)));
                    }
                });
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

/// The tips of the buttons beside the actions (stage 25).
const LAWS_TIP: &str = "Все законы по группам: что дают, сколько стоят и сколько лет вводятся. \
                        Закон переживает правителя.";
const WILL_TIP: &str = "Наказ потомкам: заповедь, наказ и наследник. Вскроют после смерти \
                        государя; переписать можно в любой год.";
const ABDICATE_TIP: &str = "Сложить корону сейчас. Почти всегда бьёт по наследнику: претензия \
                            слабеет, престол оспорят.";
const WAIT_TIP: &str = "Год вперёд: действия идут, случаются события.";
const LINK_TIP: &str = "Ссылка на эту партию: тот же seed и те же решения.";

/// Why action `a` cannot start now, in words; None when it can. `open`: it is in
/// `Game::available_actions`, so only a slot can be missing. The reasons by the order of
/// that check: the condition, the price, the crown power, the targets.
fn why_not(g: &Game, a: &Action, open: bool) -> Option<String> {
    let (w, d) = (&g.world, &g.data);
    if open {
        return (!d.action_slots.free(w, &d.actions, a)).then(|| slots_full(g, a));
    }
    if let Some(why) = unmet(d, w, &a.requires) {
        return Some(why);
    }
    let treasury = w.axes[&d.economy.treasury];
    if a.cost > treasury {
        let (cost, now) = (round(a.cost), round(treasury));
        return Some(format!("Нужно {cost} золота, в казне {now}"));
    }
    let builds = (a.on_complete.iter()).any(|e| matches!(e, Effect::Build(..)));
    let busy = match (builds, a.id.starts_with(ENACT)) {
        (true, _) => "Уже строится",
        (_, true) => "Уже вводится",
        _ => "Уже идёт",
    };
    let power = round(a.min_crown_power);
    let capital = w.provinces.get(&w.capital.province);
    let capital = capital.map_or(Fx(0), |p| p.crown_power);
    Some(match &a.target {
        ActionTarget::Province(f) => {
            let fit: Vec<_> = (w.provinces.values()).filter(|p| f.matches(p, w)).collect();
            match fit.iter().any(|p| p.crown_power >= a.min_crown_power) {
                _ if fit.is_empty() && f.without_building.is_some() => {
                    "Во всех подходящих землях уже построено".into()
                }
                _ if fit.is_empty() => "Нет подходящих земель".into(),
                false => format!("Нужна сила короны в земле от {power}"),
                true => busy.into(),
            }
        }
        _ if capital < a.min_crown_power => format!("Нужна сила короны в столице от {power}"),
        ActionTarget::Neighbour if a.marries() => "Ни один двор не примет сватов".into(),
        ActionTarget::Heir if w.heirs.is_empty() => "Нет наследников".into(),
        ActionTarget::Heir if (a.on_complete).contains(&Effect::HeirOp(HeirOp::TargetMarry)) => {
            format!("Нет неженатых наследников от {} лет", d.marriage.age)
        }
        _ => busy.into(),
    })
}

/// Why `p` does not hold, in words; None when it holds. The conditions the actions and the
/// laws use have words of their own, any other one is «условие не выполнено».
fn unmet(d: &Data, w: &World, p: &Predicate) -> Option<String> {
    use Predicate as P;
    if p.eval(w) {
        return None;
    }
    let law = |f: &str| d.law(f).map(|l| l.name.clone());
    Some(match p {
        P::All(ps) => return ps.iter().find_map(|p| unmet(d, w, p)),
        P::AxisAbove(a, v) => format!("Нужно: {} от {}", axis_name(d, a), round(*v)),
        P::AxisBelow(a, v) => format!("Нужно: {} ниже {}", axis_name(d, a), round(*v)),
        P::Flag(f) if *f == d.abdication.contested_flag => "Только при споре о престоле".into(),
        P::NotFlag(f) if *f == d.abdication.contested_flag => "Идёт спор о престоле".into(),
        P::Flag(f) if law(f).is_some() => format!("Не действует закон «{}»", law(f)?),
        P::NotFlag(f) if law(f).is_some() => format!("Закон «{}» уже действует", law(f)?),
        P::AtWar => "Только во время войны".into(),
        P::Not(x) if **x == P::AtWar => "Уже идёт война".into(),
        P::Not(x) if matches!(**x, P::WarStage(_)) => "Уже идут переговоры о мире".into(),
        P::BastardCount(..) => "Нет бастардов".into(),
        P::HeirCount(..) => "Нет наследников".into(),
        _ => "Условие не выполнено".into(),
    })
}

/// «Слоты действий заняты» and what opens one more, for the kind of `a` (war or peace).
fn slots_full(g: &Game, a: &Action) -> String {
    match a.target == ActionTarget::Enemy {
        true => "Военный слот занят".into(),
        false => match more_slots(&g.data, &g.world) {
            Some(more) => format!("Слоты действий заняты: {more}"),
            None => "Слоты действий заняты".into(),
        },
    }
}

/// «ещё одно откроет бюрократия 40»: the next step of `action_slots` with more slots; None
/// on the last one.
fn more_slots(d: &Data, w: &World) -> Option<String> {
    let s = &d.action_slots;
    let (now, v) = (s.slots(w), w.axes[&s.axis]);
    let next = (s.steps.iter()).filter(|(at, n)| *at > v && *n > now);
    let (at, _) = next.min_by_key(|(at, _)| *at)?;
    let by = axis_name(d, &s.axis).to_lowercase();
    Some(format!("ещё одно откроет {by} {}", round(*at)))
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
        linked_text(ui, g, &v.text, &links(g));
        ui.add_space(6.0);
        // Every choice with its effects under it (stage 25), its hint on hover.
        for (i, c) in v.choices.iter().enumerate() {
            let button = Button::new(&c.text)
                .wrap()
                .min_size(egui::vec2(520.0, 28.0));
            let mut r = ui.add(button);
            if let Some(hint) = &c.hint {
                r = r.on_hover_text(hint);
            }
            if r.clicked() {
                cmd = Some(Cmd::Choose(i));
            }
            ui.indent(i, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (text, up) in effects(&g.data, &c.effects) {
                        ui.small(RichText::new(text).color(tone(up)));
                    }
                });
            });
            ui.add_space(4.0);
        }
    });
    cmd
}

/// What an event text may name with a tip: every form (`Names::declined`) of the target
/// province, its vassal house, the target state, the state behind the target, the province
/// the war is fought for. Longest first, so «Нордмарка» wins over «Нордмарк».
fn links(g: &Game) -> Vec<(String, Target)> {
    let w = &g.world;
    let p = g.pending_event.as_ref();
    let mut named: Vec<(&str, Target)> = vec![];
    let province = |id: &ProvinceId| {
        w.provinces
            .get(id)
            .map(|p| (p.name.as_str(), Target::Province(id.clone())))
    };
    let state = |id: &NeighbourId| {
        w.neighbours
            .get(id)
            .map(|n| (n.name.as_str(), Target::Neighbour(id.clone())))
    };
    match p.and_then(|p| p.target.as_ref()) {
        Some(Target::Province(id)) => {
            named.extend(province(id));
            if let Some(Holder::Vassal(v)) = w.provinces.get(id).map(|p| &p.holder)
                && let Some(v) = w.vassals.get(v)
            {
                named.push((v.name.as_str(), Target::Province(id.clone())));
            }
        }
        Some(Target::Neighbour(id)) => named.extend(state(id)),
        _ => {}
    }
    named.extend(p.and_then(|p| p.neighbour.as_ref()).and_then(state));
    named.extend((w.war.as_ref().and_then(|x| x.target.as_ref())).and_then(province));
    let mut forms: Vec<(String, Target)> = (named.into_iter())
        .flat_map(|(name, t)| (0..6).map(move |c| (g.data.names.declined(name, c), t.clone())))
        .collect();
    forms.sort_by_key(|(f, _)| std::cmp::Reverse(f.chars().count()));
    forms.dedup_by(|a, b| a.0 == b.0);
    forms
}

/// `text` with the names of `links` stressed, each with its tip: a province its income,
/// people, holder and loyalty; a state its relation and strength.
fn linked_text(ui: &mut Ui, g: &Game, text: &str, links: &[(String, Target)]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut rest = text;
        loop {
            let next = (links.iter())
                .filter_map(|(form, t)| rest.find(form.as_str()).map(|at| (at, form, t)))
                .min_by_key(|(at, form, _)| (*at, std::cmp::Reverse(form.len())));
            let Some((at, form, t)) = next else {
                ui.label(rest);
                break;
            };
            if at > 0 {
                ui.label(&rest[..at]);
            }
            let name = RichText::new(form).color(RUBRIC).underline();
            tip_label(ui, name, |ui| target_tip(ui, &g.world, &g.data, t));
            rest = &rest[at + form.len()..];
        }
    });
}

/// A province: holder, income, people, loyalty, crown power, a foreign one its kingdom; a
/// state: relation, strength, lands, the kingdom. The map's hover and the names in event texts.
pub(crate) fn target_tip(ui: &mut Ui, w: &World, d: &Data, t: &Target) {
    match t {
        Target::Province(id) => {
            let Some(p) = w.provinces.get(id) else { return };
            ui.strong(&p.name);
            ui.label(holder_name(w, &p.holder));
            ui.label(format!("Доход {}", round(p.income)));
            ui.label(format!("Население {}", p.population));
            ui.label(format!("Лояльность {}", round(p.loyalty)));
            ui.label(format!("Сила короны {}", round(p.crown_power)));
            // In words what the map shows as icons (stage 26b).
            let all = map::buildings(w, d, id);
            for (built, what) in [(true, "Постройки"), (false, "Строится")] {
                let names: Vec<String> = (all.iter())
                    .filter(|(_, b)| *b == built)
                    .map(|(b, _)| format!("{} {}", b.icon, b.name))
                    .collect();
                if !names.is_empty() {
                    ui.label(format!("{what}: {}", names.join(", ")));
                }
            }
            if let Holder::Foreign(n) = &p.holder
                && let Some(n) = w.neighbours.get(n)
            {
                realm_tip(ui, d, n);
            }
        }
        Target::Neighbour(id) => {
            let Some(n) = w.neighbours.get(id) else {
                return;
            };
            let lands = (w.provinces.values()).filter(|p| p.holder == Holder::Foreign(id.clone()));
            ui.strong(&n.name);
            ui.label(format!("Отношение {}", plus(n.relation)));
            ui.label(format!("Сила {}", round(n.strength)));
            ui.label(format!("Земель {}", lands.count()));
            realm_tip(ui, d, n);
        }
        Target::Heir(_) => {}
    }
}

/// What the crown sees of a foreign kingdom (`Neighbour.realm`, stage 26): its ruler and
/// house, the succession law, its stability in a word; nothing for a state of numbers alone.
fn realm_tip(ui: &mut Ui, d: &Data, n: &Neighbour) {
    let Some(r) = &n.realm else { return };
    let house = d.names.declined(&r.house, 1);
    if r.fallen {
        ui.label(format!("Династия {house} пала, трон пуст"));
        return;
    }
    ui.label(format!("Правит {} из дома {house}", r.ruler));
    let law = d.heirs.laws.iter().find(|l| l.flag == r.law);
    ui.label(format!("Закон: {}", law.map_or("нет", |l| l.name.as_str())));
    let axis = d.stability.as_ref().map(|s| &s.axis);
    if let Some(a) = d.axes.iter().find(|a| Some(&a.id) == axis) {
        ui.label(format!("Стабильность: {}", level(a, r.stability)));
    }
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
    axes_panel(ui, g);
    if let Some((line, hint)) = overreach(g) {
        ui.label(RichText::new(line).color(RUBRIC));
        ui.label(RichText::new(hint).small().color(FG2));
    }
    // The laws in force and what each does to the graph; succession too, also told with
    // the heirs.
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
    heading(ui, "Наследники");
    let first = bd_core::sim::successor(w, d);
    let rightful = bd_core::sim::rightful(w, d);
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
        tip_label(
            ui,
            format!("Бастарды: {} {INFO}", names.join("; ")),
            |ui| {
                ui.label("Рождены вне брака и не наследуют, пока их не признают.");
            },
        );
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
            tip(row, |ui| {
                ui.strong(&n.name);
                ui.label(format!("Отношение {value}: {mood}"));
                ui.label(format!("Сила {}, {stance}", round(n.strength)));
                realm_tip(ui, d, n);
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

/// Years of the trend arrow, of the forecast and of the far one.
const TREND_YEARS: u32 = 5;
const GENERATION: u32 = 30;
const FAR: u32 = 60;
/// A trend by the change over `TREND_YEARS`, in hundredths of the axis range (`max − min`):
/// down by 5 and more, by 1 and more, under 1 either way, up by 1, up by 5.
const ARROWS: [&str; 5] = ["⇊", "↘", "→", "↗", "⇈"];
/// A node open in words, by fifths of its range.
const LEVELS: [&str; 5] = ["ничтожно", "низко", "средне", "высоко", "предельно"];
/// A value or a cause the bureaucracy does not show yet (stage 25).
const UNKNOWN: &str = "???";

/// «Состояние»: every axis as the bureaucracy shows it (`graph::sight`). In numbers: the
/// value and the trend arrow; in words: the level and the arrow; closed: `UNKNOWN`. The rest
/// in the tip of the value (stage 25): the target it steps to, the two largest pushes, the
/// forecast (`graph::forecast`) in 30 years, and in 60 once every number is open, or what
/// bureaucracy opens a closed one.
fn axes_panel(ui: &mut Ui, g: &Game) {
    let (w, d) = (&g.world, &g.data);
    let soon = graph::forecast(d, w, TREND_YEARS);
    let gen_ = graph::forecast(d, &soon, GENERATION - TREND_YEARS);
    let numbers = (d.reveal.as_ref()).is_some_and(|r| w.axes[&r.axis] >= r.numbers);
    let far = numbers.then(|| graph::forecast(d, &gen_, FAR - GENERATION));
    let by = (d.reveal.as_ref())
        .map_or("", |r| axis_name(d, &r.axis))
        .to_lowercase();
    Grid::new("axes").show(ui, |ui| {
        for a in &d.axes {
            let (v, sight) = (w.axes[&a.id], graph::sight(d, w, a));
            let arrow = arrow(a, soon.axes[&a.id] - v);
            let mut notes = vec![];
            let value = match sight {
                Sight::Closed(None) => continue,
                Sight::Closed(Some(at)) => {
                    notes.push(format!("Откроет {by} {}", round(at)));
                    UNKNOWN.to_string()
                }
                Sight::Words => {
                    let later = level(a, gen_.axes[&a.id]);
                    if later != level(a, v) {
                        notes.push(format!("Через {GENERATION} лет: {later}"));
                    }
                    format!("{} {arrow}", level(a, v))
                }
                Sight::Numbers => {
                    let to = graph::target(d, w, a);
                    if graph::step(d, a) != Fx(0) && round(to) != round(v) {
                        notes.push(format!("Идёт к {}", round(to)));
                    }
                    let later = round(gen_.axes[&a.id]);
                    if later != round(v) {
                        notes.push(format!("Через {GENERATION} лет ≈ {later}"));
                    }
                    let far = far.as_ref().map(|f| round(f.axes[&a.id]));
                    if let Some(far) = far.filter(|f| *f != later) {
                        notes.push(format!("Через {FAR} лет ≈ {far}"));
                    }
                    format!("{} {arrow}", round(v))
                }
            };
            if !matches!(sight, Sight::Closed(_)) {
                notes.extend(pressing(d, w, a, sight == Sight::Numbers));
                notes.extend(movers(d, w, &a.id));
            }
            ui.small(axis_name(d, &a.id));
            ui.horizontal(|ui| {
                if sight == Sight::Numbers {
                    meter(ui, v, a.min, a.max);
                }
                tip_label(ui, RichText::new(value).small(), |ui| {
                    ui.strong(axis_name(d, &a.id));
                    for n in &notes {
                        ui.label(n);
                    }
                });
            });
            ui.end_row();
        }
    });
}

/// The trend arrow of a change of `a` over `TREND_YEARS`, relative to its range: an axis of
/// 0–100 and the treasury moving as fast for their size get the same arrow.
fn arrow(a: &AxisDef, change: Fx) -> &'static str {
    let one = (a.max - a.min).0.max(1);
    ARROWS[match change.0 * 100 {
        c if c <= -5 * one => 0,
        c if c <= -one => 1,
        c if c < one => 2,
        c if c < 5 * one => 3,
        _ => 4,
    }]
}

/// The word of `v` on the range of `a`, by fifths.
fn level(a: &AxisDef, v: Fx) -> &'static str {
    let i = (v - a.min).0 * 5 / (a.max - a.min).0.max(1);
    LEVELS[i.clamp(0, 4) as usize]
}

/// «Держит: Знать +3», «Давит: Расслоение −7»: the two largest pushes on `a`
/// (`graph::pressing`) of at least a half, those up «держат», those down «давят»; without
/// `numbers` their signs alone. Empty when nothing pushes so.
fn pressing(d: &Data, w: &World, a: &AxisDef, numbers: bool) -> Vec<String> {
    let big = |c: &Fx| c.0.abs() >= Fx::SCALE / 2;
    let pushes = graph::pressing(d, w, &a.id, 2);
    let group = |up: bool, verb: [&str; 3]| {
        let list: Vec<String> = (pushes.iter())
            .filter(|(_, c)| big(c) && (*c > Fx(0)) == up)
            .map(|(p, c)| {
                let sign = if up { "+" } else { "−" };
                match numbers {
                    true => format!("{} {sign}{}", pusher(d, w, p), round(Fx(c.0.abs()))),
                    false => format!("{} ({sign})", pusher(d, w, p)),
                }
            })
            .collect();
        let verb = plural(list.len() as u32, verb);
        (!list.is_empty()).then(|| format!("{verb}: {}", list.join(", ")))
    };
    let up = group(true, ["Держит", "Держат", "Держат"]);
    let down = group(false, ["Давит", "Давят", "Давят"]);
    up.into_iter().chain(down).collect()
}

/// «Поднимают: Учредить канцелярию, закон «Свод законов», «Интриги при дворе»» and
/// «Опускают: …» (stage 26b): the actions, the laws (their anchors and what they do once in)
/// and the most frequent events of the random pool (`MOVERS`, by weight) that move axis `a`,
/// all read from the data.
fn movers(d: &Data, w: &World, a: &AxisId) -> Vec<String> {
    let sides = |es: &[Effect]| {
        let mut s = [false; 2];
        signs(es, a, &mut s);
        s
    };
    let mut by: [Vec<String>; 2] = Default::default();
    let own = |x: &&Action| !x.id.starts_with(ENACT) && !x.id.starts_with(REPEAL);
    for act in d.actions.iter().filter(own) {
        let s = sides(&[&act.on_complete[..], &act.yearly[..]].concat());
        (0..2)
            .filter(|&k| s[k])
            .for_each(|k| by[k].push(named(w, &act.name)));
    }
    for l in &d.laws.list {
        let mut s = sides(&l.on_complete);
        let mut shift = |v: Fx| s[(v < Fx(0)) as usize] |= v != Fx(0);
        (l.anchors.iter())
            .filter(|(x, _)| x == a)
            .for_each(|(_, v)| shift(*v));
        if *a == d.economy.treasury {
            shift(l.treasury);
        }
        (0..2)
            .filter(|&k| s[k])
            .for_each(|k| by[k].push(format!("закон «{}»", l.name)));
    }
    let mut pool: Vec<&bd_core::rules::Event> = d.events.iter().filter(|e| e.weight > 0).collect();
    pool.sort_by_key(|e| std::cmp::Reverse(e.weight));
    for (k, list) in by.iter_mut().enumerate() {
        let moving = pool.iter().filter(|e| {
            let all: Vec<Effect> = e.choices.iter().flat_map(|c| c.effects.clone()).collect();
            sides(&all)[k]
        });
        let titles = moving.map(|e| e.title.split(['{', ':']).next().unwrap_or_default().trim());
        let titles = titles.filter(|t| !t.is_empty()).take(MOVERS);
        list.extend(titles.map(|t| format!("«{t}»")));
    }
    let verbs = ["Поднимают", "Опускают"].into_iter().zip(by);
    let lines = verbs.filter(|(_, l)| !l.is_empty());
    lines
        .map(|(v, l)| format!("{v}: {}", l.join(", ")))
        .collect()
}

/// The most frequent events `movers` names each way.
const MOVERS: usize = 3;

/// Which way effects move axis `a`: `[up, down]`, every branch of a chance or a suit.
fn signs(es: &[Effect], a: &AxisId, s: &mut [bool; 2]) {
    for e in es {
        match e {
            Effect::Axis(x, v) if x == a && *v != Fx(0) => s[(*v < Fx(0)) as usize] = true,
            Effect::Chance(c) => {
                signs(&c.then, a, s);
                signs(&c.otherwise, a, s);
            }
            Effect::Marry { then, otherwise } => {
                signs(then, a, s);
                signs(otherwise, a, s);
            }
            Effect::IfFriendly(es) => signs(es, a, s),
            _ => {}
        }
    }
}

/// Who pushes: a law by its name, an edge by its source node, a node still closed as
/// `UNKNOWN` with the bureaucracy that opens it.
fn pusher(d: &Data, w: &World, p: &Push) -> String {
    let from = match p {
        Push::Law(l) => return l.name.clone(),
        Push::Edge(i) => &d.influences[*i].from,
    };
    let def = d.axes.iter().find(|a| a.id == *from);
    match (def.map(|a| graph::sight(d, w, a)), &d.reveal) {
        (Some(Sight::Closed(Some(at))), Some(r)) => {
            let by = axis_name(d, &r.axis).to_lowercase();
            format!("{UNKNOWN} (откроет {by} {})", round(at))
        }
        (Some(Sight::Closed(_)), _) => UNKNOWN.into(),
        _ => axis_name(d, from).into(),
    }
}

/// «держит выше: Закрепощение, Знать · усиливает: Знать → Закрепощение»: what law `l` does
/// to the graph while in force, by the names in the data: its anchor shifts up and down, the
/// edges it strengthens (or switches on) and weakens. Empty when it does none of that.
fn holds(d: &Data, l: &LawDef) -> String {
    let anchors = |up: bool| {
        let shifts = l.anchors.iter().filter(|(_, s)| (*s > Fx(0)) == up);
        shifts
            .map(|(a, _)| axis_name(d, a))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let edges = |up: bool| {
        let edges = l.edges.iter().filter_map(|(id, m)| {
            let e = d.influences.iter().find(|e| e.id == *id)?;
            let one = Fx::from_int(1);
            let way = (e.off || *m > one, !e.off && *m < one);
            (way == (up, !up))
                .then(|| format!("{} → {}", axis_name(d, &e.from), axis_name(d, &e.to)))
        });
        edges.collect::<Vec<_>>().join(", ")
    };
    let parts = [
        ("держит выше", anchors(true)),
        ("держит ниже", anchors(false)),
        ("усиливает", edges(true)),
        ("ослабляет", edges(false)),
    ];
    let parts = parts.iter().filter(|(_, l)| !l.is_empty());
    parts
        .map(|(k, l)| format!("{k}: {l}"))
        .collect::<Vec<_>>()
        .join(" · ")
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
    let mut line = format!(
        "Сверх предела: {k} {lands} ({}), штраф в год: {}",
        names.join(", "),
        fines.join(", ")
    );
    let pressure: Vec<String> = (c.pressure.iter())
        .map(|(a, v)| format!("{} {}", axis_name(d, a).to_lowercase(), round(*v * times)))
        .collect();
    if !pressure.is_empty() {
        line += &format!("; пока земли лишние: {}", pressure.join(", "));
    }
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

/// `+28`, `-5`, `0`: a whole number with its sign.
fn plus(v: Fx) -> String {
    match v > Fx(0) {
        true => format!("+{}", round(v)),
        false => round(v),
    }
}

/// The treasury's year (`war::income_parts`) part by part: the income of the crown lands
/// and the rest, the army, the war, the land over the crown's room, the court and the laws.
fn money_tip(ui: &mut Ui, g: &Game) {
    let (w, d) = (&g.world, &g.data);
    let (income, upkeep) = bd_core::war::income_parts(w, d);
    let crown = w.provinces.values().filter(|p| p.holder == Holder::Crown);
    let land = crown.fold(Fx(0), |s, p| s + p.income);
    let army = bd_core::data::curve(&d.war.army_upkeep, w.axes[&d.war.army]);
    let war = if w.war.is_some() {
        land * d.war.income_penalty
    } else {
        Fx(0)
    };
    let c = &d.crown_capacity;
    let over = c.income * Fx::from_int(c.over(w).len() as i64);
    let line = |what: &str, total: Fx, parts: &[(&str, Fx)]| {
        let parts: Vec<String> = (parts.iter())
            .filter(|(_, v)| round(*v) != "0" && round(*v) != "-0")
            .map(|(k, v)| format!("{k} {}", plus(*v)))
            .collect();
        match parts.is_empty() {
            true => format!("{what} {}", plus(total)),
            false => format!("{what} {}: {}", plus(total), parts.join(", ")),
        }
    };
    let neg = |v: Fx| Fx(0) - v;
    ui.strong(format!("За год {}", plus(income - upkeep)));
    ui.label(line(
        "Доход",
        income,
        &[("земли короны", land), ("прочее", income - land)],
    ));
    let court = upkeep - army - war - over;
    let costs = [
        ("армия", neg(army)),
        ("война", neg(war)),
        ("земли сверх предела", neg(over)),
        ("двор и законы", neg(court)),
    ];
    ui.label(line("Расходы", neg(upkeep), &costs));
    let more = "Сверх этого казну тратят действия и события.";
    ui.small(RichText::new(more).color(FG2));
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
    // The year before the outcome has no battle (events/war.ron): say why (stage 25).
    if war.stage == bd_core::war::WarStage::Peace {
        let talks = "Идут переговоры о мире: битв больше не будет, условия решит счёт войны.";
        ui.label(RichText::new(talks).color(FG2));
    }
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

/// The tip of `r`: on hover, and on a click or tap until the next click (a phone has no
/// hover). For words and grey buttons; a working button keeps its click for itself.
pub(crate) fn tip(r: egui::Response, add: impl Fn(&mut Ui)) -> egui::Response {
    let add = |ui: &mut Ui| {
        ui.set_max_width(320.0);
        add(ui)
    };
    match egui::Popup::from_toggle_button_response(&r).show(add) {
        Some(_) => r,
        None => r.on_hover_ui(add),
    }
}

/// A label that opens its tip by a click too: `tip`.
pub(crate) fn tip_label(
    ui: &mut Ui,
    text: impl Into<egui::WidgetText>,
    add: impl Fn(&mut Ui),
) -> egui::Response {
    let r = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
    tip(r, add)
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
    let label = ui.add(egui::Label::new(RichText::new(label).small()).sense(egui::Sense::click()));
    meter(ui, v, min, max);
    ui.small(value);
    ui.end_row();
    label
}

/// The bar of `bar`.
fn meter(ui: &mut Ui, v: Fx, min: Fx, max: Fx) {
    let share = ((v - min).0 as f32 / (max - min).0.max(1) as f32).clamp(0.0, 1.0);
    let color = match share {
        s if s < 0.35 => RUBRIC,
        s if s < 0.55 => WARN,
        _ => GOOD,
    };
    let bar = ProgressBar::new(share).fill(color).desired_width(110.0);
    ui.add(bar.desired_height(6.0));
}

/// The axis name from rules.ron, or its id.
pub(crate) fn axis_name<'a>(d: &'a Data, id: &'a AxisId) -> &'a str {
    let def = d.axes.iter().find(|a| a.id == *id);
    def.filter(|a| !a.name.is_empty())
        .map_or(&id.0, |a| &a.name)
}

/// «10, из них у короны 6»: provinces of the crown and its vassals, then of the crown alone.
pub(crate) fn realm(w: &World) -> String {
    let own = (w.provinces.values()).filter(|p| !matches!(p.holder, Holder::Foreign(_)));
    let crown = own.clone().filter(|p| p.holder == Holder::Crown).count();
    format!("{}, из них у короны {crown}", own.count())
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
            // A building of no crown power (the dikes of a flood) shows nothing.
            Effect::Build(_, b) => match d.crown_power.buildings.get(b) {
                Some(bonus) => signed("сила короны в провинции", *bonus),
                None => continue,
            },
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

/// Where the browser keeps the saved game.
#[cfg(target_arch = "wasm32")]
const SAVE_KEY: &str = "blessed-dynasty-save";

/// localStorage; None in a private window or where the browser denies it: no saving then.
#[cfg(target_arch = "wasm32")]
fn storage() -> Option<eframe::web_sys::Storage> {
    eframe::web_sys::window()?.local_storage().ok()?
}

#[cfg(target_arch = "wasm32")]
fn load_save() -> Option<String> {
    storage()?.get_item(SAVE_KEY).ok()?
}

#[cfg(target_arch = "wasm32")]
fn write_save(text: Option<&str>) {
    if let Some(s) = storage() {
        let _ = match text {
            Some(t) => s.set_item(SAVE_KEY, t),
            None => s.remove_item(SAVE_KEY),
        };
    }
}

/// `blessed-dynasty/save.txt` in the user's data directory: `$XDG_DATA_HOME`,
/// `~/.local/share`, `%APPDATA%`.
#[cfg(not(target_arch = "wasm32"))]
fn save_path() -> Option<std::path::PathBuf> {
    let var = |k| std::env::var_os(k).map(std::path::PathBuf::from);
    let dir = (var("XDG_DATA_HOME"))
        .or_else(|| var("HOME").map(|h| h.join(".local/share")))
        .or_else(|| var("APPDATA"))?;
    Some(dir.join("blessed-dynasty").join("save.txt"))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_save() -> Option<String> {
    std::fs::read_to_string(save_path()?).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn write_save(text: Option<&str>) {
    let Some(path) = save_path() else { return };
    let _ = match text {
        Some(t) => (path.parent().map_or(Ok(()), std::fs::create_dir_all))
            .and_then(|_| std::fs::write(&path, t)),
        None => std::fs::remove_file(&path),
    };
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
            let mut app = App::persistent(&cc.egui_ctx);
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
            let mut app = App::persistent(&cc.egui_ctx);
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
        /// Of the screen; a long card wants a tall one to show all of it.
        height: f32,
    }

    impl Harness {
        fn new() -> Harness {
            let ctx = egui::Context::default();
            let app = App::new(&ctx);
            let mut h = Harness {
                ctx,
                app,
                height: 800.0,
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, self.height))),
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
            for _ in 0..8 {
                self.frame(vec![]);
            }
            let out = self.frame(vec![]);
            let tree = out
                .platform_output
                .accesskit_update
                .expect("accesskit is on");
            // A button has its text as label, a clickable label as value.
            let node = (tree.nodes.iter())
                .find(|(_, n)| n.label() == Some(label) || n.value() == Some(label));
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
        let end = g.reign_end(g.ended.clone().unwrap());
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
        // Stage 22: the rulers by their epithets, the epilogue after the last entry and in
        // the summary, the numbers of the entry on hover.
        let crowning = reigns.iter().position(|(_, c)| *c).unwrap();
        h.app.apply(Cmd::Entry(crowning));
        let t = texts_of(&mut h);
        let (prev, next) = (c.rulers[0].full_name(), c.rulers[1].full_name());
        assert!(c.rulers.iter().all(|r| !r.epithet.is_empty()));
        assert!(
            t.iter().any(|x| x.ends_with(&format!("† {prev}. {next}"))),
            "{t:?}"
        );
        assert!(
            t.iter().any(|x| x.starts_with(&format!("{next} · "))),
            "{t:?}"
        );
        assert!(!t.contains(&"Провинций".to_string()) && !t.contains(&c.epilogue));
        assert!(hover(&mut h, NUMBERS).contains(&"Провинций".to_string()));
        h.app.apply(Cmd::Entry(c.entries.len() - 1));
        assert!(!c.epilogue.is_empty() && texts_of(&mut h).contains(&c.epilogue));
        h.app.apply(Cmd::Summary);
        assert!(texts_of(&mut h).contains(&c.epilogue));
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
    /// made as the chronicle tells them (stage 22) and what changed, and shows the latest year
    /// on top; the lines with numbers under «Изменения за год», open by a click (stage 25).
    /// Stage 25: Генрих, dead at 3, is no heir in the summary.
    #[test]
    fn the_year_summary_lists_what_happened() {
        let mut h = Harness::new();
        six_years(&mut h);
        let line = |s: &str, up| (s.to_string(), up);
        // Every year opens with the treasury, notable or not.
        let money = |s: &str| line(&format!("{MONEY} {s}"), Some(true));
        let want = vec![
            (
                "1188",
                vec![line(
                    "Отряды Нордмарка перешли границу и жгли сёла земли Арден, но королевское войско отбросило их.",
                    None,
                )],
                vec![money("+27: доход +32, расходы -5")],
            ),
            (
                "1189",
                vec![line(
                    "Бароны потребовали подтвердить их старые вольности, и Ульрих скрепил грамоту. Руки короны стали короче.",
                    None,
                )],
                vec![
                    money("+28: доход +32, расходы -4"),
                    line("Бюрократия -5", Some(false)),
                    line("Знать +11", Some(true)),
                    line("Завершено: Проложить дорогу (Берг)", None),
                ],
            ),
            (
                "1190",
                vec![line(
                    "Купцы Веструма получили право торговать на ярмарках королевства.",
                    None,
                )],
                vec![
                    money("+54: доход +33, расходы -4, действия и события +25"),
                    line("Рождение: Генрих", Some(true)),
                ],
            ),
            (
                "1191",
                vec![line(
                    "Знать съехалась в столицу на собор, и Ульрих выслушал лучших людей королевства.",
                    None,
                )],
                vec![
                    money("+28: доход +33, расходы -4"),
                    line("Знать +8", Some(true)),
                ],
            ),
            (
                "1192",
                vec![line(
                    "Дозор Веструма сжёг пограничную мельницу и убил людей, и корона потребовала виру за убитых.",
                    None,
                )],
                vec![
                    money("+44: доход +33, расходы -4, действия и события +15"),
                    line("Рождение: Освальд", Some(true)),
                ],
            ),
            (
                "1193",
                vec![line(
                    "Купцы Веструма получили право торговать на ярмарках королевства.",
                    None,
                )],
                vec![
                    money("+54: доход +33, расходы -4, действия и события +25"),
                    line("Умер в детстве королевский сын Генрих", Some(false)),
                ],
            ),
        ];
        let got: Vec<_> = (h.app.journal.iter())
            .map(|(d, c, l)| (d.as_str(), c.clone(), l.clone()))
            .collect();
        assert_eq!(got, want);
        let texts = texts(&h.frame(vec![]));
        let (latest, older) = (pos(&texts, "1193"), pos(&texts, "1192"));
        assert!(latest < older, "the latest year comes first");
        // The news of a year in one block under its header (stage 26b).
        let news = pos(&texts, "· Умер в детстве королевский сын Генрих");
        assert_eq!(texts[news - 1], NEWS);
        assert!(
            !texts.iter().any(|t| t.contains("Смерть наследника")),
            "{texts:?}"
        );
        assert!(
            !texts
                .iter()
                .any(|t| t == "Знать +11" || t.starts_with(MONEY))
        );
        // No longer «в цифрах…» on hover: a header to click.
        assert!(!texts.iter().any(|t| t.contains("в цифрах")));
        h.click_label(YEAR_NUMBERS);
        let numbers = settled(&mut h);
        assert!(numbers.iter().any(|t| t.starts_with(MONEY)), "{numbers:?}");
        // A quiet year says so; a reign over leaves the journal to the reign's card.
        let g = h.app.game.as_mut().unwrap();
        (g.data.quiet_weight, g.data.heirs.birth) = (1_000_000, vec![]);
        (g.world.war, g.queue) = (None, vec![]);
        h.app.apply(Cmd::Wait);
        let quiet = vec![money("+28: доход +33, расходы -4")];
        assert_eq!(
            h.app.journal.last().unwrap(),
            &("1194".to_string(), vec![], quiet)
        );
        assert!(texts_of(&mut h).contains(&"Тихий год".to_string()));
    }

    /// Stage 20: the choice of a symptom of the graph is marked «Знамение» in the summary.
    #[test]
    fn an_omen_is_marked_in_the_year_summary() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let d = &mut h.app.game.as_mut().unwrap().data;
        for e in &mut d.events {
            match e.id == "omen_dear_bread" {
                true => (e.weight, e.when) = (1_000_000, bd_core::rules::Predicate::All(vec![])),
                false => e.weight = 0,
            }
        }
        h.app.apply(Cmd::Wait);
        h.app.apply(Cmd::Choose(0));
        let lines = &h.app.journal.last().unwrap().1;
        let omen =
            "Знамение. Хлеб на торгу подорожал втрое, и Ульрих запретил вывозить его за рубеж.";
        assert!(lines.iter().any(|(t, _)| t == omen), "{lines:?}");
        assert_eq!(lines.iter().filter(|(t, _)| t.starts_with(OMEN)).count(), 1);
    }

    /// Stage 21: an axis shows its trend, target, the two largest pushes and the forecast; a
    /// push from a closed node is «неясная причина» with the bureaucracy that opens it; the
    /// bureaucracy opens the nodes in words, then in numbers with the far forecast.
    #[test]
    fn the_axes_tell_where_they_go_and_what_pushes_them() {
        let mut h = Harness::new();
        h.height = 1400.0;
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let set = |h: &mut Harness, axes: &[(&str, i64)]| {
            let g = h.app.game.as_mut().unwrap();
            for (a, v) in axes {
                g.world.axes.insert(AxisId((*a).into()), Fx::from_int(*v));
            }
            g.world.recompute_loyalty(&g.data);
            texts_of(h)
        };
        let after = |t: &[String], k: &str| t[pos(t, k) + 1].clone();
        // Strata 80 drags the people to 50 - 32 by e6; the bureaucracy 20 does not show it.
        // Stage 25: the row keeps the value and the arrow, the rest goes to its tip.
        let t = set(&mut h, &[("strata", 80)]);
        assert_eq!(after(&t, "Народ"), "50 ⇊");
        let gone = [
            "Давит",
            "давит",
            "Держит",
            "держит",
            "через поколение",
            "неясная",
            "откроет бюрократия 70",
        ];
        assert!(
            !t.iter().any(|x| gone.iter().any(|g| x.contains(g))),
            "{t:?}"
        );
        let tip = hover(&mut h, "50 ⇊");
        for want in ["Идёт к 18", "Давит: ??? (откроет бюрократия 70) −32"]
        {
            assert!(tip.contains(&want.to_string()), "{want}: {tip:?}");
        }
        assert!(
            tip.iter().any(|x| x.starts_with("Через 30 лет ≈ ")),
            "{tip:?}"
        );
        // Below 70 strata is closed: «???», what opens it in the tip; grain is open in words.
        assert_eq!(after(&t, "Расслоение"), "???");
        assert_eq!(after(&t, "Хлебные запасы"), "средне →");
        let tip = hover(&mut h, "???");
        assert!(
            tip.iter().any(|x| x.starts_with("Откроет бюрократия ")),
            "{tip:?}"
        );
        // The nobles over their target: down a lot, toward 50.
        let t = set(&mut h, &[("loyalty_nobles", 90)]);
        assert_eq!(after(&t, "Знать"), "90 ⇊");
        assert!(hover(&mut h, "90 ⇊").contains(&"Идёт к 50".to_string()));
        // At 70 strata opens in words and names itself in the people's pushes.
        let t = set(&mut h, &[("bureaucracy", 70)]);
        assert_eq!(after(&t, "Расслоение"), "предельно ↘"); // back to its anchor 40
        let tip = hover(&mut h, "50 ⇊");
        assert!(
            tip.contains(&"Давит: Расслоение −32".to_string()),
            "{tip:?}"
        );
        assert!(!tip.iter().any(|x| x.contains("Через 60 лет")));
        // At 85 every node in numbers, with the forecast of two generations.
        let t = set(&mut h, &[("bureaucracy", 85)]);
        assert!(after(&t, "Расслоение").starts_with("80 "), "{t:?}");
        assert!(!t.contains(&"???".to_string()));
        let tip = hover(&mut h, "50 ⇊");
        assert!(
            tip.iter().any(|x| x.starts_with("Через 60 лет ≈")),
            "{tip:?}"
        );
    }

    /// Stage 22: the arrow by the share of the axis range, so the treasury (−1000…10000) and
    /// an axis of 0–100 moving as fast for their size get the same arrow.
    #[test]
    fn the_trend_arrow_is_relative_to_the_range() {
        let d = load_data();
        let axis = |id: &str| d.axes.iter().find(|a| a.id.0 == id).unwrap();
        let (treasury, stability) = (axis("treasury"), axis("stability"));
        // 3% of the range: 330 of the treasury, 3 of stability.
        for (pct, want) in [(-6, "⇊"), (-3, "↘"), (0, "→"), (3, "↗"), (6, "⇈")] {
            assert_eq!(arrow(treasury, Fx::from_int(110 * pct)), want, "{pct}");
            assert_eq!(arrow(stability, Fx::from_int(pct)), want, "{pct}");
        }
        // +28 a year is no rush for a treasury of 11000.
        assert_eq!(arrow(treasury, Fx::from_int(5 * 28)), "↗");
    }

    /// Stage 21: a law in force says what it holds up and down and which edges it feeds,
    /// by the names in the data.
    #[test]
    fn a_law_in_force_tells_what_it_holds_and_feeds() {
        let d = load_data();
        let law = |id: &str| holds(&d, d.law(id).unwrap());
        assert_eq!(
            law("law_serfdom"),
            "держит выше: Закрепощение, Знать · усиливает: Знать → Закрепощение"
        );
        assert_eq!(
            law("law_free_peasants"),
            "держит выше: Торговля · держит ниже: Закрепощение, Знать · ослабляет: Знать → Закрепощение"
        );
        // An edge off without the law is switched on by it.
        assert_eq!(
            law("law_charters"),
            "держит выше: Городские вольности, Грамотность · держит ниже: Знать · усиливает: Расслоение → Городские вольности"
        );
        assert_eq!(law("law_primogeniture"), "");
        // In the side panel under the law.
        let mut h = Harness::new();
        h.height = 1400.0;
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let w = &mut h.app.game.as_mut().unwrap().world;
        w.flags.insert("law_serfdom".into());
        w.axes
            .insert(AxisId("bureaucracy".into()), Fx::from_int(85));
        let t = texts_of(&mut h);
        assert_eq!(t[pos(&t, "Крепостное право ℹ") + 1], law("law_serfdom"));
        // And the anchor it shifts is a push on the node: up, so it «держит» (stage 22; down
        // «давит», the_axes_tell_where_they_go_and_what_pushes_them), in the tip (stage 25).
        let value = t[pos(&t, "Закрепощение") + 1].clone();
        let tip = hover(&mut h, &value);
        assert!(
            tip.iter()
                .any(|x| x.starts_with("Держит: Крепостное право +30")),
            "{tip:?}"
        );
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
            line.ends_with(
                "штраф в год: доход -4, лояльность этих земель -3; \
                 пока земли лишние: стабильность -4"
            ),
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

    /// A card sizes itself in its first frame, and an open header unfolds over a few more.
    fn settle(h: &mut Harness) {
        for _ in 0..10 {
            h.frame(vec![]);
        }
    }

    fn texts_of(h: &mut Harness) -> Vec<String> {
        texts(&h.frame(vec![]))
    }

    /// The texts once a card just opened has laid itself out.
    fn settled(h: &mut Harness) -> Vec<String> {
        for _ in 0..3 {
            h.frame(vec![]);
        }
        texts_of(h)
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
        // The topmost of the widgets so labelled (the map legend names the neighbours too):
        // the order of the nodes is not the order on screen.
        let nodes = (tree.nodes.iter())
            .filter(|(_, n)| n.label() == Some(label) || n.value() == Some(label));
        let b = (nodes.filter_map(|(_, n)| n.bounds()))
            .min_by(|a, b| (a.y0, a.x0).partial_cmp(&(b.y0, b.x0)).unwrap())
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

    /// Stage 24: «Составить завещание» opens the card with its cost now; a precept picked,
    /// it is sealed into the world and the year's record, then rewritten; the dynasty reads
    /// it at the founder's death, and the chronicle shows its strength to his heirs.
    #[test]
    fn the_testament_is_written_rewritten_and_read() {
        let mut h = Harness::new();
        h.height = 1000.0;
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        h.click_label("Составить завещание");
        assert_eq!(h.app.will, Some(Testament::default()));
        let shown = settled(&mut h);
        assert!(shown.iter().any(|t| t.contains("Сейчас: ")), "{shown:?}");
        h.click_label("Полная казна — крепость державы");
        let draft = h.app.will.clone().unwrap();
        assert_eq!(draft.precept.as_deref(), Some("treasury"));
        h.click_label("Скрепить печатью");
        assert_eq!(h.app.will, None);
        let t = h.game().world.testament.clone().unwrap();
        assert_eq!(t.precept.as_deref(), Some("treasury"));
        assert!(h.app.chosen.iter().any(|(l, _)| l.contains("завещание")));
        assert!(h.app.chosen.iter().any(|(_, up)| *up == Some(false)));
        // Rewritten: the card opens on the testament in force.
        h.click_label("Переписать завещание");
        assert_eq!(
            h.app.will.as_ref().unwrap().precept.as_deref(),
            Some("treasury")
        );
        h.click_label("Отмена");
        assert_eq!(h.app.will, None);
        // To the end of the reign and the chronicle.
        let g = h.app.game.as_mut().unwrap();
        g.world.ruler.health = Fx(0);
        let end = g.reign_end("illness".into());
        h.app.step(Ok(Step::ReignEnded(end)));
        let shown = settled(&mut h);
        assert!(shown.contains(&"ЗАВЕЩАНИЕ".to_string()), "{shown:?}");
        assert!(
            shown
                .iter()
                .any(|t| t.contains("Полная казна — крепость державы"))
        );
        let (c, _) = h.app.dynasty.as_ref().unwrap();
        let heir = (chronicle::reigns(c, &h.game().data).iter()).position(|&(r, _)| r == 1);
        h.app.apply(Cmd::Entry(heir.expect("an heir reigned")));
        let shown = settled(&mut h);
        assert!(
            shown
                .iter()
                .any(|t| t.starts_with("Завет ") && t.contains("сила")),
            "{shown:?}"
        );
    }

    #[test]
    fn hovering_tells_what_actions_neighbours_and_the_law_do() {
        let mut h = Harness::new();
        // The side panel scrolls; the neighbours come after the axes and the heirs.
        h.height = 1000.0;
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

        let law = hover(&mut h, "Закон: Абсолютное первородство ℹ");
        let text = h.game().data.heirs.laws[0].text();
        assert!(law.contains(&text) && text.contains("ниже 70"), "{law:?}");
        assert!(texts_of(&mut h).contains(&"Первый в очереди: Конрад".to_string()));

        let n = hover(&mut h, "Веструм");
        for t in [
            "Отношение +40: друг",
            "Сила 45, торгует",
            "Союзов и браков нет",
            // Stage 26: the kingdom behind the numbers.
            "Правит Годфрид из дома Вестингов",
            "Закон: Выборный закон",
        ] {
            assert!(n.contains(&t.to_string()), "{t}: {n:?}");
        }
        assert!(n.iter().any(|t| t.starts_with("Стабильность: ")), "{n:?}");
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

    /// Acceptance (stage 16): a law changes by mouse: «Ввести закон» (stage 19: every law by
    /// its group), the list with every law's text and price, a pick; the law is in force once
    /// the years pass. Stage 19: the laws in force on the reign screen, a repeal from the card.
    #[test]
    fn the_law_changes_from_its_list() {
        let mut h = Harness::new();
        h.height = 4000.0;
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let row = texts_of(&mut h);
        assert!(
            !row.iter().any(|t| t.starts_with("Ввести закон «")),
            "{row:?}"
        );
        h.click_label("Ввести закон");
        assert!(h.app.laws);
        settle(&mut h);
        let list = texts_of(&mut h);
        let d = h.game().data.clone();
        for g in ["Наследование", "Крестьяне", "Вера", "Прочие законы"]
        {
            assert!(list.contains(&g.to_string()), "{g}: {list:?}");
        }
        for l in d.heirs.laws.iter().skip(1) {
            assert!(
                list.contains(&l.name) && list.contains(&l.text()),
                "{}: {list:?}",
                l.name
            );
        }
        assert!(
            list.contains(&"действует".to_string()),
            "the law in force says so"
        );
        for t in [
            "Стоимость 45 · 2 года · сила короны от 40",
            "пока вводят, к цели: Церковь -10 · по введении: Знать +3, Церковь -2",
        ] {
            assert!(list.iter().any(|x| x.starts_with(t)), "{t}: {list:?}");
        }
        // Each group opens: its laws, what they hold and feed, their price and resistance.
        for g in ["Наследование", "Крестьяне"] {
            h.click_label(g);
            settle(&mut h);
        }
        let list = texts_of(&mut h);
        let serfdom = d.law("law_serfdom").unwrap();
        assert!(list.contains(&serfdom.description), "{list:?}");
        let t = "пока вводят, к цели: Народ -10 · по введении: Армия +10 · в год: Казна +3";
        assert!(list.iter().any(|x| x.starts_with(t)), "{t}: {list:?}");
        for g in ["Крестьяне", "Наследование"] {
            h.click_label(g);
            settle(&mut h);
        }
        h.click_label("Салический закон");
        assert!(!h.app.laws);
        let running = &h.game().world.active_actions;
        assert_eq!(running[0].id, "enact_law_salic");
        let wait = |h: &mut Harness, n: u32| {
            for _ in 0..n {
                h.app.apply(Cmd::Wait);
                while let Screen::Event(_) = h.app.screen {
                    h.app.apply(Cmd::Choose(0));
                }
            }
        };
        wait(&mut h, 2);
        assert!(texts_of(&mut h).contains(&"Закон: Салический закон ℹ".to_string()));
        // Another law, in force on the reign screen, and its repeal for half the price.
        h.app
            .game
            .as_mut()
            .unwrap()
            .world
            .axes
            .insert(bd_core::state::AxisId("treasury".into()), Fx::from_int(500));
        h.click_label("Ввести закон");
        settle(&mut h);
        h.click_label("Прочие законы");
        settle(&mut h);
        h.click_label("Ярмарочное право");
        wait(&mut h, 2);
        let tip = hover(&mut h, "Ярмарочное право ℹ");
        assert!(
            tip.contains(&d.law("law_fairs").unwrap().description),
            "{tip:?}"
        );
        h.click_label("Ввести закон");
        settle(&mut h);
        h.click_label("Отменить · 22");
        assert_eq!(h.game().world.active_actions[0].id, "repeal_law_fairs");
        wait(&mut h, 2);
        assert!(!texts_of(&mut h).contains(&"Ярмарочное право ℹ".to_string()));
        // While the throne is disputed, nothing to pick and the reason said.
        h.app
            .game
            .as_mut()
            .unwrap()
            .world
            .flags
            .insert("succession_contested".into());
        h.click_label("Ввести закон");
        settle(&mut h);
        let list = texts_of(&mut h);
        let busy = "Пока идёт спор о престоле, закон о престоле не сменить.";
        assert!(list.contains(&busy.to_string()));
        h.click_label("Мужское первородство");
        assert!(h.app.laws && h.game().world.active_actions.is_empty());
        h.click_label("Отмена");
        assert!(!h.app.laws);
    }

    #[test]
    fn the_reign_ends_with_its_summary_then_the_chronicle() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(8));
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
        // Stage 26b: the land in one line, who got which province.
        let land = shown
            .iter()
            .find(|t| t.starts_with("Земли:"))
            .expect("the land line");
        assert!(land.contains("вассалу Вейр Гарт"), "{land}");
        // The founder's life, and the successor as the chronicle crowned him.
        assert!(shown.contains(&c.rulers[0].full_name()), "{shown:?}");
        assert!(shown.contains(&c.rulers[0].biography), "{shown:?}");
        let reigns = chronicle::reigns(c, &h.app.data);
        let crowned = reigns
            .iter()
            .position(|&(r, crowns)| r == 1 && crowns)
            .unwrap();
        assert!(shown.contains(&c.entries[crowned].text), "{shown:?}");
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
            "♔ Ульрих (р. 1155), на троне с 1187",
            "Конрад (р. 1181)",
            "Генрих (1190–1193)",
        ] {
            assert!(shown.contains(&t.to_string()), "{t}: {shown:?}");
        }
        // Children under their parent, deeper.
        let kin = &h.game().world.kin;
        assert_eq!(chronicle::family(kin), [(0, 0), (1, 1), (2, 1), (3, 1)]);
        h.click_label("Закрыть");
        assert!(!h.app.tree);

        // After the dynasty: the rulers from the chronicle, the first ones on screen (a long
        // dynasty scrolls).
        play(&mut h, 8);
        h.click_label("Родословная");
        let shown = texts_of(&mut h);
        let c = &h.app.dynasty.as_ref().unwrap().0;
        assert!(c.rulers.len() > 1);
        for r in &c.rulers[..2] {
            let crowned = format!("♔ {} (", r.full_name());
            assert!(shown.iter().any(|t| t.starts_with(&crowned)), "{crowned}");
        }
        assert_eq!(chronicle::family(&c.kin).len(), c.kin.len());
        // Stage 22: a click on a ruler tells his life under him, a second click hides it.
        // The chronicle under the card may tell it too: counted.
        let (life, heir) = (c.rulers[1].biography.clone(), c.rulers[1].full_name());
        let told = |t: &[String]| t.iter().filter(|x| **x == life).count();
        let before = told(&shown);
        assert!(!life.is_empty());
        let label = shown.iter().find(|t| t.starts_with(&format!("♔ {heir} (")));
        let label = label.unwrap().clone();
        h.click_label(&label);
        assert_eq!(told(&texts_of(&mut h)), before + 1, "{life}");
        h.click_label(&label);
        assert_eq!(told(&texts_of(&mut h)), before);
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
            shown.contains(&"Бастарды: Ольга, 0 ℹ".to_string()),
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
        settle(&mut h);
        let shown = texts_of(&mut h);
        let founder = "(р. 1155, отречение в 1187)";
        assert!(
            shown
                .iter()
                .any(|t| t.starts_with("♔ Ульрих") && t.contains(founder)),
            "{shown:?}"
        );
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
        assert!(running_line(&h).starts_with(
            "Действия 1 из 1 (ещё одно откроет бюрократия 40): идёт Построить крепость (Берг), 0 из"
        ));
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
        // Stage 25: the year's summary says why the war ended, by the battles.
        let mut lines = (h.app.journal.iter()).flat_map(|(_, c, l)| c.iter().chain(l));
        assert!(
            lines.any(|(t, _)| t.contains(" сражени")),
            "{:?}",
            h.app.journal
        );
        let out = h.frame(vec![]);
        assert!(!texts(&out).iter().any(|t| t.starts_with("⚔")));
        assert_eq!(outlines(&out, map::RUBRIC, 4.0), 0);
    }

    /// Seed 1, the reign begun.
    fn begun(h: &mut Harness) {
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
    }

    /// Puts event `id` with this target on screen, as `wait` would.
    fn force_event(h: &mut Harness, id: &str, target: Option<Target>) {
        let g = h.app.game.as_mut().unwrap();
        g.pending_event = Some(bd_core::game::PendingEvent {
            event_id: id.into(),
            target,
            neighbour: None,
        });
        let res = g.wait();
        h.app.step(res);
        settle(h);
    }

    fn set_axis(h: &mut Harness, axis: &str, v: i64) {
        let g = h.app.game.as_mut().unwrap();
        g.world.axes.insert(AxisId(axis.into()), Fx::from_int(v));
    }

    /// Acceptance (stage 25): every action to be had has a tip with what it gives, costs and
    /// takes; so have the buttons beside them.
    #[test]
    fn every_action_has_a_tip() {
        let mut h = Harness::new();
        h.height = 1000.0;
        begun(&mut h);
        let open = h.game().available_actions();
        assert!(open.len() > 5);
        for (id, _) in open
            .iter()
            .filter(|(id, _)| !id.starts_with(ENACT) && !id.starts_with(REPEAL))
        {
            let name = action_name(&h.game().world, &h.game().data, id);
            let tip = hover(&mut h, &name);
            let cost = tip.iter().any(|t| t.starts_with("Стоимость "));
            assert!(cost && tip.len() > 2, "{name}: {tip:?}");
        }
        for (button, tip) in [
            ("Ввести закон", LAWS_TIP),
            ("Составить завещание", WILL_TIP),
            ("Отречься", ABDICATE_TIP),
            ("Подождать год ▸", WAIT_TIP),
        ] {
            assert!(hover(&mut h, button).contains(&tip.to_string()), "{button}");
        }
        // The treasury: what a year brings, part by part.
        let money = hover(&mut h, &format!("за год +27 {INFO}"));
        for t in [
            "За год +27",
            "Доход +32: земли короны +27, прочее +5",
            "Расходы -5: армия -5",
        ] {
            assert!(money.contains(&t.to_string()), "{t}: {money:?}");
        }
    }

    /// Acceptance (stage 25): an action not to be had stays, grey, with the reason in its
    /// tip: the price, the condition, the slots; a click on it starts nothing. The laws
    /// say theirs in their card.
    #[test]
    fn an_action_not_to_be_had_shows_why() {
        let mut h = Harness::new();
        h.height = 1000.0;
        begun(&mut h);
        set_axis(&mut h, "treasury", 80);
        let shown = texts_of(&mut h);
        for t in [
            "Построить крепость",
            "Признать бастарда",
            "Женить наследника",
        ] {
            assert!(shown.contains(&t.to_string()), "{t}");
        }
        let why = |h: &mut Harness, name: &str| {
            let tip = hover(h, name);
            assert!(tip.iter().any(|t| t.starts_with("Стоимость ")), "{tip:?}");
            tip
        };
        assert!(why(&mut h, "Построить крепость").contains(&"Нужно 120 золота, в казне 80".into()));
        assert!(why(&mut h, "Признать бастарда").contains(&"Нет бастардов".into()));
        let heir = "Нет неженатых наследников от 14 лет".to_string();
        assert!(why(&mut h, "Женить наследника").contains(&heir));
        h.click_label("Построить крепость");
        assert!(h.app.picking.is_none() && h.game().world.active_actions.is_empty());
        // The slot taken: every action of peace grey, with what opens one more.
        set_axis(&mut h, "treasury", 1000);
        h.app.apply(Cmd::Act("royal_progress".into(), None));
        let full = "Слоты действий заняты: ещё одно откроет бюрократия 40".to_string();
        assert!(why(&mut h, "Проложить дорогу").contains(&full));
        // The laws in their card: the condition, the price.
        set_axis(&mut h, "treasury", 1000);
        h.app.apply(Cmd::Wait);
        while let Screen::Event(_) = h.app.screen {
            h.app.apply(Cmd::Choose(0));
        }
        h.height = 4000.0;
        h.click_label("Ввести закон");
        settle(&mut h);
        h.click_label("Прочие законы");
        settle(&mut h);
        let card = texts_of(&mut h);
        assert!(
            card.contains(&"Нужно: Бюрократия от 40".to_string()),
            "{card:?}"
        );
        set_axis(&mut h, "treasury", 10);
        assert!(texts_of(&mut h).contains(&"Нужно 45 золота, в казне 10".to_string()));
    }

    /// Acceptance (stage 25): with the slots taken the buttons are blocked, the reason in
    /// their tip, and the slots' line tells what opens the next one; on the last step it
    /// tells nothing more.
    #[test]
    fn full_slots_block_the_buttons_and_tell_what_opens_more() {
        let mut h = Harness::new();
        begun(&mut h);
        assert_eq!(
            running_line(&h),
            "Действия 0 из 1 (ещё одно откроет бюрократия 40)"
        );
        h.app.apply(Cmd::Act("royal_progress".into(), None));
        h.click_label("Проложить дорогу");
        assert!(h.app.picking.is_none(), "blocked");
        assert_eq!(h.game().world.active_actions.len(), 1);
        let line = running_line(&h);
        assert!(line.starts_with("Действия 1 из 1 (ещё одно откроет бюрократия 40): идёт"));
        set_axis(&mut h, "bureaucracy", 50);
        assert!(running_line(&h).starts_with("Действия 1 из 2 (ещё одно откроет бюрократия 70)"));
        h.click_label("Проложить дорогу");
        assert!(h.app.picking.is_some(), "a slot free again");
        set_axis(&mut h, "bureaucracy", 70);
        assert!(running_line(&h).starts_with("Действия 1 из 3: идёт"));
    }

    /// Text shapes with their positions.
    fn texts_at(out: &egui::FullOutput) -> Vec<(String, Pos2)> {
        fn walk(s: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos)),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut all = Vec::new();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut all));
        all
    }

    /// Acceptance (stage 25): the effects of an event stand under their own choice, the
    /// hint is no line of its own; a province named in the text has its tip.
    #[test]
    fn effects_stand_under_their_choice() {
        let mut h = Harness::new();
        begun(&mut h);
        force_event(&mut h, "cap_fire", None);
        let shown = texts_at(&h.frame(vec![]));
        let y = |t: &str| {
            let at = shown.iter().find(|(x, _)| x == t);
            at.unwrap_or_else(|| panic!("no «{t}»: {shown:?}")).1.y
        };
        let choices = [
            "Отстроить посад из казны",
            "Пусть отстраиваются сами",
            "Строить заново только из камня",
        ];
        let [a, b, c] = choices.map(y);
        assert!(a < b && b < c);
        for (effect, (lo, hi)) in [
            ("Казна -50", (a, b)),
            ("доход провинции -2", (b, c)),
            ("Казна -80", (c, f32::MAX)),
        ] {
            assert!(lo < y(effect) && y(effect) < hi, "{effect}");
        }
        assert!(!shown.iter().any(|(t, _)| t.contains("Наведите")));
        // A province of the text: stressed, its tip on hover.
        h.app.apply(Cmd::Choose(1));
        let berg = Target::Province(ProvinceId("berg".into()));
        force_event(&mut h, "prov_crop_failure", Some(berg));
        let tip = hover(&mut h, "Берг");
        assert!(tip.contains(&"корона".to_string()), "{tip:?}");
        for t in ["Доход ", "Население ", "Лояльность "] {
            assert!(tip.iter().any(|x| x.starts_with(t)), "{t}: {tip:?}");
        }
    }

    /// Acceptance (stage 25): the game is saved once a year as its link; «Продолжить
    /// партию» opens the same game; the end of the reign takes the save away.
    #[test]
    fn the_game_is_saved_yearly_and_continued() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(2));
        h.app.apply(Cmd::Begin);
        assert_eq!(h.app.saved, None);
        h.app.apply(Cmd::Act("royal_progress".into(), None));
        assert_eq!(h.app.saved, None, "not before the year closes");
        for _ in 0..3 {
            h.app.apply(Cmd::Wait);
            while let Screen::Event(_) = h.app.screen {
                h.app.apply(Cmd::Choose(0));
            }
        }
        let g = h.game();
        let link = link::encode(PRESETS[0].0, 2, g);
        assert_eq!(h.app.saved.as_ref(), Some(&link));
        // Another start of the game, with that save: the same game goes on.
        let mut other = Harness::new();
        other.app.saved = Some(link.clone());
        assert!(texts_of(&mut other).contains(&"Продолжить партию".to_string()));
        other.click_label("Продолжить партию");
        assert!(
            matches!(other.app.screen, Screen::Reign),
            "{}",
            other.app.note
        );
        assert_eq!(other.game().world, h.game().world);
        assert_eq!(other.game().decisions, h.game().decisions);
        // No save, no button.
        assert!(!texts_of(&mut Harness::new()).contains(&"Продолжить партию".to_string()));
        // The reign over: nothing to go on with.
        h.app.apply(Cmd::Abdicate);
        let id = h.app.data.abdication.event.clone();
        let ev = h.app.data.events.iter().find(|e| e.id == id).unwrap();
        let confirm = (ev.choices.iter()).position(|c| c.effects.contains(&Effect::Abdicate));
        h.app.apply(Cmd::Choose(confirm.unwrap()));
        assert!(matches!(h.app.screen, Screen::ReignOver));
        assert_eq!(h.app.saved, None);
    }

    /// Stage 25: tips come at once, and a click (a tap on a phone) opens one without hover.
    #[test]
    fn a_tip_opens_by_a_click() {
        let mut h = Harness::new();
        assert!(h.ctx.global_style().interaction.tooltip_delay <= 0.1);
        begun(&mut h);
        let law = h.game().data.heirs.laws[0].clone();
        let label = format!("Закон: {} {INFO}", law.name);
        // No hover tip in this test: only the click can show it.
        h.ctx
            .global_style_mut(|s| s.interaction.tooltip_delay = 1000.0);
        assert!(!texts_of(&mut h).contains(&law.text()));
        h.click_label(&label);
        h.frame(vec![Event::PointerMoved(Pos2::new(1.0, 1.0))]);
        assert!(texts_of(&mut h).contains(&law.text()));
    }

    /// Bug of playtest 02: a child dead before `sim.heir_death_age` is no heir in the
    /// year's summary, as in the chronicle; from that age on he is.
    #[test]
    fn a_child_dead_is_no_heir() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        let g = h.game();
        let age = g.data.sim.heir_death_age;
        let told = |age: u32| {
            let g = h.game();
            let mut before = g.world.clone();
            before.heirs[0].age = age;
            let mut after = before.clone();
            after.heirs.remove(0);
            change_lines(g, &before, &after)
        };
        let name = g.world.heirs[0].name.clone();
        assert_eq!(
            told(age - 2),
            [(
                format!("Умер в детстве королевский сын {name}"),
                Some(false)
            )]
        );
        assert_eq!(
            told(age - 1),
            [(format!("Смерть наследника: {name}"), Some(false))]
        );
        assert_eq!(
            told(0),
            [(
                format!("Умер младенцем королевский сын {name}"),
                Some(false)
            )]
        );
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

    /// Every text shape with its position and colour.
    fn colored_texts(out: &egui::FullOutput) -> Vec<(String, Pos2, egui::Color32)> {
        fn walk(s: &egui::Shape, out: &mut Vec<(String, Pos2, egui::Color32)>) {
            match s {
                egui::Shape::Text(t) => {
                    out.push((t.galley.text().to_string(), t.pos, t.fallback_color))
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut all = Vec::new();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut all));
        all
    }

    /// Acceptance, stage 26b: a province with a fort and a market shows their two icons on
    /// the map, a building going up shows pale; its tip names them in words.
    #[test]
    fn the_map_shows_the_buildings_of_a_province() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let g = h.app.game.as_mut().unwrap();
        let berg = ProvinceId("berg".into());
        let p = g.world.provinces.get_mut(&berg).unwrap();
        p.buildings = ["fort", "market"].map(String::from).into();
        h.app
            .apply(Cmd::Act("build_road".into(), Some(Target::Province(berg))));
        let out = h.frame(vec![]);
        let at = h.province_on_screen("berg");
        let d = load_data();
        let icon = |id: &str| {
            d.buildings
                .iter()
                .find(|b| b.id == id)
                .unwrap()
                .icon
                .clone()
        };
        let near: Vec<(String, egui::Color32)> = (colored_texts(&out).into_iter())
            .filter(|(t, p, _)| d.buildings.iter().any(|b| b.icon == *t) && p.distance(at) < 40.0)
            .map(|(t, _, c)| (t, c))
            .collect();
        let pale = FG.gamma_multiply(map::UNDERWAY_ALPHA);
        assert_eq!(
            near,
            [
                (icon("fort"), FG),
                (icon("road"), pale),
                (icon("market"), FG)
            ]
        );
        // The legend names every icon, its last row not clipped away under the map panel.
        let last = d
            .buildings
            .last()
            .map(|b| format!("{} {}", b.icon, b.name))
            .unwrap();
        let shown = out.shapes.iter().any(|c| match &c.shape {
            egui::Shape::Text(t) => {
                t.galley.text() == last && c.clip_rect.contains_rect(t.visual_bounding_rect())
            }
            _ => false,
        });
        assert!(shown, "{last}");
        h.ctx
            .global_style_mut(|s| s.interaction.tooltip_delay = 0.0);
        h.frame(vec![Event::PointerMoved(Pos2::new(1.0, 1.0))]);
        h.frame(vec![Event::PointerMoved(at)]);
        settle(&mut h);
        let tip = texts_of(&mut h);
        let line = |s: &str| tip.contains(&s.to_string());
        assert!(line("Постройки: ♜ крепость, ⚖ рынок"), "{tip:?}");
        assert!(line("Строится: ═ дорога"), "{tip:?}");
    }

    /// Stage 26b: every building icon is a glyph of the game's own font, DejaVu Sans, not of
    /// egui's fallbacks.
    #[test]
    fn building_icons_are_in_the_game_font() {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::empty();
        let font = egui::FontData::from_static(FONT);
        fonts
            .font_data
            .insert("dejavu".into(), std::sync::Arc::new(font));
        let family = egui::FontFamily::Name("dejavu".into());
        fonts.families.insert(family.clone(), vec!["dejavu".into()]);
        fonts
            .families
            .insert(egui::FontFamily::Proportional, vec!["dejavu".into()]);
        fonts
            .families
            .insert(egui::FontFamily::Monospace, vec!["dejavu".into()]);
        ctx.set_fonts(fonts);
        ctx.run_ui(RawInput::default(), |_| {})
            .textures_delta
            .clear();
        let d = load_data();
        assert!(d.buildings.len() >= 6);
        // A glyph not in the font has no width (egui's `has_glyph` says no for every glyph
        // of a family of one font: that font is also its replacement).
        let id = egui::FontId::new(14.0, family);
        let has = |c: char| ctx.fonts_mut(|f| f.glyph_width(&id, c)) > 0.0;
        for b in &d.buildings {
            assert!(b.icon.chars().all(has), "{}", b.id);
        }
        assert!(!has('⛪'), "the check checks");
    }

    /// Acceptance, stage 26b: the tip of an axis tells what raises it and what lowers it,
    /// read from the data: the bureaucracy names «Учредить канцелярию» among those raising it.
    #[test]
    fn an_axis_tells_what_raises_and_lowers_it() {
        let mut h = Harness::new();
        h.height = 1400.0;
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let t = texts_of(&mut h);
        let value = t[pos(&t, "Бюрократия") + 1].clone();
        let tip = hover(&mut h, &value);
        let up = tip
            .iter()
            .find(|x| x.starts_with("Поднимают: "))
            .unwrap_or_else(|| panic!("{tip:?}"));
        assert!(up.contains("Учредить канцелярию"), "{up}");
        assert!(
            up.contains("закон «Монастырские школы»") || !up.contains("закон"),
            "{up}"
        );
        let down = tip
            .iter()
            .find(|x| x.starts_with("Опускают: "))
            .unwrap_or_else(|| panic!("{tip:?}"));
        assert!(down.contains("«Знать требует»"), "{down}");
        assert!(!down.contains("Учредить канцелярию"), "{down}");
        // The nobles: the chancery lowers them.
        let d = load_data();
        let w = &h.game().world;
        let nobles = movers(&d, w, &AxisId("loyalty_nobles".into()));
        assert!(nobles[1].starts_with("Опускают: ") && nobles[1].contains("Учредить канцелярию"));
    }

    /// Acceptance, stage 26b: the land of a year in one line under «Изменения за год»: the
    /// count and who got which province.
    #[test]
    fn the_land_line_names_the_provinces_and_who_got_them() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let g = h.app.game.as_mut().unwrap();
        (g.data.quiet_weight, g.data.heirs.birth) = (1_000_000, vec![]);
        let holder = |w: &mut World, id: &str, h: Holder| {
            w.provinces.get_mut(&ProvinceId(id.into())).unwrap().holder = h;
        };
        let mut before = g.world.clone();
        holder(&mut before, "skala", Holder::Crown);
        holder(&mut g.world, "frostad", Holder::Crown);
        holder(&mut g.world, "nordheim", Holder::Crown);
        holder(
            &mut g.world,
            "berg",
            Holder::Vassal(bd_core::state::VassalId("weir".into())),
        );
        h.app.year_start = Some(before);
        h.app.apply(Cmd::Wait);
        let want = "Земли: +1 — вассалу Вейр Берг, короне Фростад и Нордхейм, Нордмарк взял Скалу";
        let (_, _, lines) = h.app.journal.last().unwrap();
        assert!(lines.contains(&(want.to_string(), Some(true))), "{lines:?}");
        assert!(!texts_of(&mut h).contains(&want.to_string()), "folded");
        h.click_label(YEAR_NUMBERS);
        assert!(settled(&mut h).contains(&want.to_string()));
    }

    /// Stage 26b: an event of one choice is a message: not asked in a card, it goes into the
    /// year's news, its choice made and recorded.
    #[test]
    fn a_message_goes_into_the_news_unasked() {
        let mut h = Harness::new();
        h.app.apply(Cmd::Start(1));
        h.click_label("Править");
        let g = h.app.game.as_mut().unwrap();
        for e in &mut g.data.events {
            match e.id == "cap_festival" {
                true => {
                    (e.weight, e.when) = (1_000_000, bd_core::rules::Predicate::All(vec![]));
                    e.choices.truncate(1);
                }
                false => e.weight = 0,
            }
        }
        h.app.apply(Cmd::Wait);
        assert!(matches!(h.app.screen, Screen::Reign));
        let g = h.game();
        assert!(g.pending_event.is_none());
        let last = g.decisions.last().unwrap();
        assert!(
            matches!(&last.kind, bd_core::game::DecisionKind::EventChoice { event_id, .. } if event_id == "cap_festival")
        );
        let (_, chosen, lines) = h.app.journal.last().unwrap();
        assert!(chosen.is_empty(), "{chosen:?}");
        let told = "В день святого покровителя Ульрих устроил турнир";
        assert!(lines.iter().any(|(t, _)| t.starts_with(told)), "{lines:?}");
        let shown = texts_of(&mut h);
        assert!(
            shown.iter().any(|t| t.starts_with(&format!("· {told}"))),
            "{shown:?}"
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
