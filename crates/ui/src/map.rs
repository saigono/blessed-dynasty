//! The old map (stage 29): the provinces on paper in the manner of the Carta Marina, from the
//! raster art of `art.rs`: the sea and the paper in tiles, the land washed in its state's
//! colour, bands along the borders and the coast, rivers and roads, ink, sprites and labels.
//! Floats are display only.

use crate::art::{Art, Sprite, When, hash};
use bd_core::data::{BuildingDef, Data};
use bd_core::rules::{Effect, Target};
use bd_core::state::{Holder, Map, NeighbourId, ProvinceId, RealmsStart, World};
use eframe::egui::{
    Color32, FontFamily, FontId, Galley, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke,
    TextureId, Ui, Vec2, epaint::TextShape, epaint::Vertex, pos2, vec2,
};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

// The palette of parchment and ink.
pub const BG: Color32 = Color32::from_rgb(0xf6, 0xef, 0xdd);
pub const BG2: Color32 = Color32::from_rgb(0xe2, 0xd3, 0xb4);
pub const FG: Color32 = Color32::from_rgb(0x2a, 0x20, 0x18);
pub const FG2: Color32 = Color32::from_rgb(0x57, 0x49, 0x3a);
pub const RUBRIC: Color32 = Color32::from_rgb(0x9b, 0x2b, 0x1e);
pub const CROWN: Color32 = Color32::from_rgb(0xc4, 0x9a, 0x3c);
pub const GOOD: Color32 = Color32::from_rgb(0x3f, 0x7f, 0x3e);
pub const WARN: Color32 = Color32::from_rgb(0xb9, 0x7f, 0x22);
/// Vassal houses by their order in the world.
const VASSALS: [Color32; 3] = [
    Color32::from_rgb(0xc0, 0x6a, 0x5c),
    Color32::from_rgb(0x9a, 0x7a, 0x3a),
    Color32::from_rgb(0x8a, 0x64, 0xa8),
];
/// Foreign states by their ordinal (stage 29c): washed over the parchment, any two of them,
/// our gold and the empire's red stay apart (`the_states_differ_on_parchment`).
const FOREIGN: [Color32; 8] = [
    Color32::from_rgb(0x70, 0xb0, 0xcc),
    Color32::from_rgb(0x63, 0x33, 0xcc),
    Color32::from_rgb(0x45, 0x80, 0x20),
    Color32::from_rgb(0x33, 0xcc, 0xa3),
    Color32::from_rgb(0x24, 0x2d, 0x66),
    Color32::from_rgb(0x78, 0x64, 0x48),
    Color32::from_rgb(0xcc, 0x33, 0xa3),
    Color32::from_rgb(0x33, 0xcc, 0x33),
];
/// How much of its state's colour the land takes over the paper.
const WASH: f32 = 0.32;
/// A building being built, pale on the map.
pub const UNDERWAY_ALPHA: f32 = 0.45;

/// Map units between two points of a wobbled outline, and how far a point wobbles.
const STEP: f32 = 7.0;
const WOBBLE: f32 = 1.2;
/// The sea round the land, in map units.
const SEA: f32 = 44.0;
/// Map units a tile of paper and of sea covers.
const PAPER_TILE: f32 = 110.0;
const SEA_TILE: f32 = 190.0;
/// Bands, in map units across: the borders (inward), the coast (outward), rivers, roads.
const BAND: f32 = 7.0;
const COAST: f32 = 10.0;
const RIVER: f32 = 5.0;
const ROAD: f32 = 2.6;
/// Sprite heights in map units at the whole map.
const SETTLEMENT: f32 = 15.0;
const CAPITAL: f32 = 21.0;
const BUILDING: f32 = 14.0;
const NATURE: f32 = 12.0;
const MARK: f32 = 12.0;
const SHIP: f32 = 20.0;
/// Province names and state ribbons, in screen pixels at the whole map.
const NAME_PX: f32 = 10.5;
const STATE_PX: f32 = 12.5;
/// Stage 29d: a capital's name is this much larger than a province's.
const CAPITAL_NAME: f32 = 1.3;
/// Stage 29d: below this many screen pixels per map unit (the chronicle's little map) the
/// labels shrink with the map, no smaller than `MIN_PX`, and only the states are named.
const SMALL: f32 = 0.7;
const MIN_PX: f32 = 7.0;

/// The narrowest view with the cartouche and the compass: not the chronicle's.
const OVERLAYS: f32 = 500.0;

/// The closest zoom of the big map.
const MAX_ZOOM: f32 = 4.0;

/// The map font (Cormorant SC), registered by the app as the family «map».
pub fn map_font(px: f32) -> FontId {
    FontId::new(px, FontFamily::Name("map".into()))
}

/// What a click on the map asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Click {
    Province(ProvinceId),
    State(NeighbourId),
    /// Our ribbon or cartouche.
    Kingdom,
}

/// The ink of an outline edge: the coast and a state's border solid, a vassal's appanage
/// dashed, a province within one holder's land dotted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ink {
    Solid,
    Dashed,
    Dotted,
}

struct Land {
    /// The outline as given, interior on the left of its edges.
    raw: Vec<Pos2>,
    /// Wobbled, and its triangles.
    outline: Vec<Pos2>,
    tris: Vec<u32>,
    /// Where its settlement stands.
    anchor: Pos2,
    area: f32,
}

/// An outline edge once, between two original points: its wobbled points from the smaller
/// to the larger and the provinces on their left and right (none for the sea).
struct Edge {
    pts: Vec<Pos2>,
    sides: [Option<ProvinceId>; 2],
}

/// What the washes, bands and ink of a world are made of: the holders and their colours.
type Key = Vec<(Holder, Color32)>;

/// The washes, bands and ink in map coordinates, and the key they were made for.
type Made = (Key, Vec<Mesh>, Vec<(Vec<Pos2>, Ink)>);

/// Where the ribbons go (`MapView::ribbons`): for the holders of the land, the view, its zoom
/// and pan and the cartouche, of every state the size of its name and the ribbon's centre on
/// screen.
type Spots = (
    (Vec<Holder>, Rect, f32, Vec2, Option<Rect>),
    BTreeMap<Option<NeighbourId>, (f32, Pos2)>,
);

pub struct MapView {
    lands: BTreeMap<ProvinceId, Land>,
    edges: Vec<Edge>,
    /// The land and the sea round it, in map coordinates.
    frame: Rect,
    /// Drawn as splines through the points of the map file.
    pub(crate) rivers: Vec<Vec<Pos2>>,
    /// Trees, hills and fields: where, the terrain, a salt for the variant.
    nature: Vec<(Pos2, String, String)>,
    /// The ships, the fish and the wind on the sea.
    sea: Vec<(Pos2, String)>,
    terrain: BTreeMap<ProvinceId, String>,
    /// The capitals the states started with (the preset's kingdoms).
    capitals: BTreeMap<NeighbourId, ProvinceId>,
    /// Where the last frame painted the map; clicks and tests read it.
    pub(crate) rect: Cell<Rect>,
    /// Stage 28: the wheel zooms in (1 is the whole map) round the pointer, a drag moves the
    /// view: the centre of the view in map coordinates, off the frame's centre.
    zoom: Cell<f32>,
    pan: Cell<Vec2>,
    /// Made again only when the holders change.
    cache: RefCell<Option<Made>>,
    spots: RefCell<Option<Spots>>,
    /// What the ribbons and the cartouche painted last answer to a click.
    pub(crate) hits: RefCell<Vec<(Rect, Click)>>,
    /// The labels painted last on screen: the ribbons (a state's, the kingdom's), the names
    /// (a province's).
    pub(crate) labels: RefCell<Vec<(Click, Rect)>>,
}

impl MapView {
    pub fn new(map: &Map, realms: Option<&RealmsStart>) -> MapView {
        let at = |(x, y): (i32, i32)| pos2(x as f32, y as f32);
        // Every polygon with its interior on the left: positive shoelace area.
        let polys: BTreeMap<&ProvinceId, Vec<(i32, i32)>> = (map.polygons.iter())
            .map(|(id, pts)| {
                let mut pts = pts.clone();
                if signed_area(&pts.iter().map(|&p| at(p)).collect::<Vec<_>>()) < 0.0 {
                    pts.reverse();
                }
                (id, pts)
            })
            .collect();
        // The edges, each once from its smaller point: neighbours share their points.
        let mut edges: BTreeMap<[(i32, i32); 2], Edge> = BTreeMap::new();
        for (id, pts) in &polys {
            for (i, &u) in pts.iter().enumerate() {
                let v = pts[(i + 1) % pts.len()];
                let (a, b) = (u.min(v), u.max(v));
                let e = edges.entry([a, b]).or_insert_with(|| {
                    let (a, b) = (at(a), at(b));
                    let n = ((b - a).length() / STEP).ceil().max(1.0) as usize;
                    let pts = (0..=n).map(|k| wobble(a + (b - a) * (k as f32 / n as f32)));
                    Edge {
                        pts: pts.collect(),
                        sides: [None, None],
                    }
                });
                e.sides[(u != a) as usize] = Some((*id).clone());
            }
        }
        // Stage 29d: the coast a spline through its (wobbled) points, not a wobbled line; the
        // borders inland end on those points and meet it there. Each coast edge from its land's side on.
        let coast: BTreeMap<(i32, i32), (i32, i32)> = (edges.iter())
            .filter_map(|(&[a, b], e)| match e.sides {
                [Some(_), None] => Some((a, b)),
                [None, Some(_)] => Some((b, a)),
                _ => None,
            })
            .collect();
        let back: BTreeMap<_, _> = coast.iter().map(|(&u, &v)| (v, u)).collect();
        for (&u, &v) in &coast {
            let mut pts = spline([back[&u], u, v, coast[&v]].map(|p| wobble(at(p))));
            if u > v {
                pts.reverse();
            }
            edges.get_mut(&[u.min(v), u.max(v)]).expect("an edge").pts = pts;
        }
        let lands = (polys.iter())
            .map(|(id, pts)| {
                let mut outline = vec![];
                for (i, &u) in pts.iter().enumerate() {
                    let v = pts[(i + 1) % pts.len()];
                    let e = &edges[&[u.min(v), u.max(v)]].pts;
                    match u < v {
                        true => outline.extend(&e[..e.len() - 1]),
                        false => outline.extend(e.iter().rev().take(e.len() - 1)),
                    }
                }
                let raw: Vec<Pos2> = pts.iter().map(|&p| at(p)).collect();
                let land = Land {
                    tris: triangulate(&outline),
                    outline,
                    anchor: anchor(&raw),
                    area: signed_area(&raw),
                    raw,
                };
                ((*id).clone(), land)
            })
            .collect::<BTreeMap<_, _>>();
        let all: Vec<Pos2> = lands.values().flat_map(|l| l.raw.clone()).collect();
        let capitals = (realms.iter())
            .flat_map(|r| &r.kingdoms)
            .map(|k| (k.id.clone(), k.capital.clone()))
            .collect();
        let mut view = MapView {
            lands,
            edges: edges.into_values().collect(),
            frame: Rect::from_points(&all).expand(SEA),
            rivers: (map.rivers.iter())
                .map(|r| smooth(&r.iter().map(|&p| at(p)).collect::<Vec<_>>()))
                .collect(),
            nature: vec![],
            sea: vec![],
            terrain: map.terrain.clone(),
            capitals,
            rect: Cell::new(Rect::NOTHING),
            zoom: Cell::new(1.0),
            pan: Cell::new(Vec2::ZERO),
            cache: RefCell::new(None),
            spots: RefCell::new(None),
            hits: RefCell::new(vec![]),
            labels: RefCell::new(vec![]),
        };
        view.nature = view.scatter();
        view.sea = view.seafaring();
        view
    }

    /// Up to four points of every province with a `terrain`, inside it and off its
    /// settlement and each other.
    fn scatter(&self) -> Vec<(Pos2, String, String)> {
        let mut out = vec![];
        for (id, l) in &self.lands {
            let Some(kind) = self.terrain.get(id) else {
                continue;
            };
            let want = (l.area / 1400.0).clamp(1.0, 4.0) as usize;
            let b = Rect::from_points(&l.raw);
            let mut got: Vec<Pos2> = vec![];
            for k in 0..40u32 {
                let h = hash(&format!("{}:{k}", id.0));
                let p = b.min + vec2(unit(h) * b.width(), unit(h >> 20) * b.height());
                let room = [vec2(0.0, 0.0), vec2(7.0, 0.0), vec2(-7.0, 0.0), vec2(0.0, -9.0)];
                let inside = room.iter().all(|d| contains(&l.raw, p + *d));
                let clear = p.distance(l.anchor) > 17.0 && got.iter().all(|q| q.distance(p) > 12.0);
                if inside && clear {
                    got.push(p);
                }
                if got.len() == want {
                    break;
                }
            }
            let salted = got.into_iter().enumerate();
            out.extend(salted.map(|(i, p)| (p, kind.clone(), format!("{}{i}", id.0))));
        }
        out
    }

    /// Ships, fish and the wind on the sea round the land: spots of the margin off the land.
    fn seafaring(&self) -> Vec<(Pos2, String)> {
        let f = self.frame;
        let spots = [
            (0.30, 0.04, "ship"),
            (0.72, 0.035, "fish"),
            (0.02, 0.55, "ship"),
            (0.975, 0.62, "fish"),
            (0.45, 0.975, "ship"),
            (0.14, 0.09, "wind"),
            (0.86, 0.975, "fish"),
        ];
        (spots.iter())
            .map(|(x, y, what)| (f.min + vec2(x * f.width(), y * f.height()), what.to_string()))
            .filter(|(p, _)| self.province_at_map(*p).is_none())
            .collect()
    }

    fn fit(&self) -> f32 {
        let (r, b) = (self.rect.get(), self.frame);
        (r.width() / b.width()).min(r.height() / b.height())
    }

    fn scale(&self) -> f32 {
        self.fit() * self.zoom.get()
    }

    /// Screen pixels per map unit of a sprite: they grow slower than the map as it zooms, so
    /// a close view stays readable and uncluttered.
    fn sprite_scale(&self) -> f32 {
        self.fit() * self.zoom.get().sqrt()
    }

    /// Of the size of a label at the whole map: as the sprites, but no larger than on a map
    /// fit to more than a pixel per unit (stage 29d: the chronicle's little map names smaller).
    fn text_scale(&self) -> f32 {
        self.fit().min(1.0) * self.zoom.get().sqrt()
    }

    /// The map is shown small: the states named, not the provinces.
    fn small(&self) -> bool {
        self.scale() < SMALL
    }

    /// The capitals of the states of `w`: ours and each foreign state's.
    pub(crate) fn capitals(&self, w: &World) -> BTreeSet<ProvinceId> {
        state_capitals(w, &self.capitals)
    }

    fn view_centre(&self) -> Pos2 {
        self.frame.center() + self.pan.get()
    }

    pub fn to_screen(&self, p: Pos2) -> Pos2 {
        self.rect.get().center() + (p - self.view_centre()) * self.scale()
    }

    pub fn to_map(&self, screen: Pos2) -> Pos2 {
        self.view_centre() + (screen - self.rect.get().center()) / self.scale()
    }

    /// Zooms by `factor` keeping the map point under `screen` in place; the view stays on the
    /// map, and at zoom 1 it is the whole map again.
    pub fn zoom_at(&self, screen: Pos2, factor: f32) {
        let at = self.to_map(screen);
        self.zoom
            .set((self.zoom.get() * factor).clamp(1.0, MAX_ZOOM));
        self.move_by(at - self.to_map(screen));
    }

    /// Moves the view by `d` in map coordinates.
    pub fn move_by(&self, d: Vec2) {
        let room = self.frame.size() * (1.0 - 1.0 / self.zoom.get()) / 2.0;
        let p = self.pan.get() + d;
        self.pan
            .set(vec2(p.x.clamp(-room.x, room.x), p.y.clamp(-room.y, room.y)));
    }

    fn province_at_map(&self, p: Pos2) -> Option<&ProvinceId> {
        let mut hit = self.lands.iter().filter(|(_, l)| contains(&l.raw, p));
        hit.next().map(|(id, _)| id)
    }

    pub fn province_at(&self, screen: Pos2) -> Option<&ProvinceId> {
        self.province_at_map(self.to_map(screen))
    }

    /// What a click at `screen` asks for: a ribbon or the cartouche over the land,
    /// else the province under it (by the inverse of the view).
    pub fn click_at(&self, screen: Pos2) -> Option<Click> {
        let hits = self.hits.borrow();
        let over = hits.iter().rev().find(|(r, _)| r.contains(screen));
        (over.map(|(_, c)| c.clone()))
            .or_else(|| self.province_at(screen).cloned().map(Click::Province))
    }

    /// Where the settlement of a province stands, in map coordinates.
    #[cfg(test)]
    pub fn centre(&self, id: &ProvinceId) -> Option<Pos2> {
        self.lands.get(id).map(|l| l.anchor)
    }

    /// The ink of the border between provinces `a` and `b`, if they have one.
    #[cfg(test)]
    pub(crate) fn ink_between(&self, w: &World, a: &str, b: &str) -> Option<Ink> {
        let sides = |e: &Edge| e.sides.iter().flatten().map(|p| p.0.clone()).collect::<Vec<_>>();
        let (inks, edges) = (self.inks(w), &self.edges);
        let mut both = edges.iter().zip(inks).filter(|(e, _)| {
            let s = sides(e);
            s.contains(&a.to_string()) && s.contains(&b.to_string())
        });
        both.next().map(|(_, (_, ink))| ink)
    }

    /// Paints the world into the rest of `ui`; `selected` is outlined. Returns a click.
    pub fn show(
        &self,
        ui: &mut Ui,
        w: &World,
        data: &Data,
        art: &Art,
        selected: Option<&ProvinceId>,
    ) -> Option<Click> {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        self.rect.set(resp.rect);
        if resp.dragged() {
            self.move_by(-resp.drag_delta() / self.scale());
        }
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if let Some(at) = resp.hover_pos().filter(|_| wheel != 0.0) {
            self.zoom_at(at, (wheel / 200.0).exp());
        }
        let painter = painter.with_clip_rect(resp.rect);
        self.hits.borrow_mut().clear();
        self.labels.borrow_mut().clear();
        self.paint(&painter, w, data, art, selected);
        // The chronicle's little map has no room for the cartouche and the rest.
        if resp.rect.width() >= OVERLAYS {
            self.overlays(&painter, w, data, art);
        }

        let clicked = resp.clicked().then(|| resp.interact_pointer_pos()).flatten();
        let clicked = clicked.and_then(|pos| self.click_at(pos));
        let hovered = resp.hover_pos().and_then(|pos| self.province_at(pos));
        let war_target = w.war.as_ref().and_then(|x| x.target.as_ref());
        if let Some(p) = hovered.and_then(|id| w.provinces.get(id)) {
            let capital = self.capitals(w).contains(&p.id);
            resp.on_hover_ui_at_pointer(|ui| {
                crate::target_tip(ui, w, data, &Target::Province(p.id.clone()));
                if capital {
                    ui.label(format!("Столица: {}", state_name(w, state_of(&p.holder))));
                }
                if war_target == Some(&p.id) {
                    ui.label(eframe::egui::RichText::new("Цель войны").color(RUBRIC));
                }
            });
        }
        clicked
    }

    fn paint(&self, painter: &Painter, w: &World, d: &Data, art: &Art, selected: Option<&ProvinceId>) {
        let tile = |name: &str| art.tiles[name];
        // The sea, tiled over the whole frame.
        let mut sea = Mesh::with_texture(tile("sea"));
        // Over the whole view: a map wider than the land shows more sea, not a void.
        let view = corners(self.rect.get()).map(|p| self.to_map(p));
        let uv = view.map(|p| (p.to_vec2() / SEA_TILE).to_pos2());
        quad(&mut sea, corners(self.rect.get()), uv, Color32::WHITE);
        painter.add(sea);
        let key: Key = (self.lands.keys())
            .filter_map(|id| w.provinces.get(id))
            .map(|p| (p.holder.clone(), holder_color(w, &p.holder)))
            .collect();
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|(k, ..)| *k != key) {
            *cache = Some((key, self.washes(w, art), self.inks(w)));
        }
        let (_, meshes, inks) = cache.as_ref().expect("just made");
        let screen = |m: &Mesh| {
            let mut m = m.clone();
            m.vertices.iter_mut().for_each(|v| v.pos = self.to_screen(v.pos));
            m
        };
        // The coast first, out into the sea, then what sails there, then the land.
        painter.add(screen(&meshes[0]));
        let k = self.sprite_scale();
        for (p, what) in &self.sea {
            let salt = format!("{p:?}");
            let name = match what.as_str() {
                "ship" => &art.style.ships[hash(&salt) as usize % art.style.ships.len()],
                n => n,
            };
            if let Some(sp) = art.sprite(name, &salt) {
                painter.add(sprite_shape(sp, self.to_screen(*p), SHIP * k, Color32::WHITE));
            }
        }
        meshes[1..].iter().for_each(|m| {
            painter.add(screen(m));
        });
        for r in &self.rivers {
            let m = band(r, false, RIVER, 0.0, RIVER * 700.0 / 64.0, tile("river"), Color32::WHITE);
            painter.add(screen(&m));
        }
        for r in self.roads(w, art) {
            let tex = tile("road-cobble");
            let m = band(&r, false, ROAD, 0.0, ROAD * 685.0 / 64.0, tex, Color32::WHITE);
            painter.add(screen(&m));
        }
        let ink = |width: f32| Stroke::new(width * (0.6 + 0.4 * self.zoom.get().sqrt()), FG);
        for (pts, how) in inks {
            let pts: Vec<Pos2> = pts.iter().map(|&p| self.to_screen(p)).collect();
            match how {
                Ink::Solid => {
                    painter.add(Shape::line(pts, ink(1.3)));
                }
                Ink::Dashed => painter.extend(Shape::dashed_line(&pts, ink(1.0), 4.5, 3.0)),
                Ink::Dotted => {
                    painter.extend(Shape::dotted_line(&pts, FG2, 3.2, 0.55 * ink(1.0).width))
                }
            }
        }
        let ring = |id: &ProvinceId| (self.lands[id].outline.iter()).map(|&p| self.to_screen(p)).collect();
        if let Some(id) = selected.filter(|id| self.lands.contains_key(*id)) {
            painter.add(Shape::closed_line(ring(id), Stroke::new(2.5, FG)));
        }
        let war_target = w.war.as_ref().and_then(|x| x.target.as_ref());
        if let Some(id) = war_target.filter(|id| self.lands.contains_key(*id)) {
            painter.add(Shape::closed_line(ring(id), Stroke::new(4.0, RUBRIC)));
        }
        // Sprites by their foot, the nearer (lower) over the farther.
        let mut sprites = self.sprites(w, d, art);
        sprites.sort_by(|a, b| a.0.total_cmp(&b.0));
        painter.extend(sprites.into_iter().map(|(_, s)| s));
        self.names(painter, w);
        self.ribbons(painter, w, d, art);
    }

    /// In map coordinates: the coast band (first), the land washed in its state's colour over
    /// the paper, the bands inside every state's border.
    fn washes(&self, w: &World, art: &Art) -> Vec<Mesh> {
        let tile = |name: &str| art.tiles[name];
        let mut coast = Mesh::with_texture(tile("coast"));
        for l in self.loops(|_| true) {
            coast.append(band(&l, true, COAST, -1.0, COAST * 567.0 / 64.0, tile("coast"), Color32::WHITE));
        }
        let mut paper = Mesh::with_texture(tile("paper"));
        for (id, l) in &self.lands {
            let Some(p) = w.provinces.get(id) else {
                continue;
            };
            let color = lerp(Color32::WHITE, holder_color(w, &p.holder), WASH);
            let first = paper.vertices.len() as u32;
            let uv = |q: Pos2| (q.to_vec2() / PAPER_TILE).to_pos2();
            (paper.vertices).extend(l.outline.iter().map(|&q| Vertex { pos: q, uv: uv(q), color }));
            paper.indices.extend(l.tris.iter().map(|i| first + i));
        }
        let mut out = vec![coast, paper];
        // Our kingdom in its own rose band, a foreign state in the white one tinted.
        let mut states: Vec<Option<&NeighbourId>> = vec![None];
        states.extend(w.neighbours.keys().map(Some));
        for s in states {
            let ours = |id: &ProvinceId| w.provinces.get(id).is_some_and(|p| state_of(&p.holder) == s);
            let (tex, color) = match s {
                None => (tile("border-band"), Color32::WHITE.gamma_multiply(0.8)),
                Some(n) => {
                    let c = holder_color(w, &Holder::Foreign(n.clone()));
                    (tile("border-band-mask"), c.gamma_multiply(0.6))
                }
            };
            for l in self.loops(ours) {
                out.push(band(&l, true, BAND, 1.0, BAND * 777.0 / 64.0, tex, color));
            }
        }
        out
    }

    /// The boundary of the provinces `inside` holds, as closed polylines with the inside on
    /// their left.
    fn loops(&self, inside: impl Fn(&ProvinceId) -> bool) -> Vec<Vec<Pos2>> {
        let is = |s: &Option<ProvinceId>| s.as_ref().is_some_and(&inside);
        // Every boundary edge directed with the inside on its left, by its first point.
        let key = |p: Pos2| ((p.x * 8.0).round() as i32, (p.y * 8.0).round() as i32);
        let mut from: BTreeMap<(i32, i32), Vec<Vec<Pos2>>> = BTreeMap::new();
        for e in &self.edges {
            let pts = match (is(&e.sides[0]), is(&e.sides[1])) {
                (true, false) => e.pts.clone(),
                (false, true) => e.pts.iter().rev().copied().collect(),
                _ => continue,
            };
            from.entry(key(pts[0])).or_default().push(pts);
        }
        let mut out = vec![];
        while let Some(start) = from.keys().next().copied() {
            let (mut line, mut at) = (vec![], start);
            while let Some(list) = from.get_mut(&at) {
                let next = list.pop().expect("no list is left empty");
                if list.is_empty() {
                    from.remove(&at);
                }
                at = key(*next.last().expect("an edge has two points"));
                line.extend(&next[..next.len() - 1]);
                if at == start {
                    break;
                }
            }
            out.push(line);
        }
        out
    }

    /// Every outline edge with its ink.
    fn inks(&self, w: &World) -> Vec<(Vec<Pos2>, Ink)> {
        let holder = |s: &Option<ProvinceId>| s.as_ref().and_then(|id| w.provinces.get(id)).map(|p| &p.holder);
        (self.edges.iter())
            .map(|e| (e.pts.clone(), ink_of(holder(&e.sides[0]), holder(&e.sides[1]))))
            .collect()
    }

    /// The roads: from every province with a road to each neighbour with one or the capital,
    /// through the middle of their common border.
    fn roads(&self, w: &World, art: &Art) -> Vec<Vec<Pos2>> {
        let has = |id: &ProvinceId| w.provinces.get(id).is_some_and(|p| p.buildings.contains(&art.style.road));
        let joins = |id: &ProvinceId| has(id) || *id == w.capital.province;
        let mut done = BTreeSet::new();
        let mut out = vec![];
        for e in &self.edges {
            let [Some(a), Some(b)] = &e.sides else { continue };
            if joins(a) && joins(b) && (has(a) || has(b)) && done.insert((a, b)) {
                out.push(vec![self.lands[a].anchor, e.pts[e.pts.len() / 2], self.lands[b].anchor]);
            }
        }
        out
    }

    /// Every sprite of the land on screen with its foot's y: nature, settlements, crowns,
    /// buildings (those going up pale under scaffolding), banners, the marks of what goes on.
    fn sprites(&self, w: &World, d: &Data, art: &Art) -> Vec<(f32, Shape)> {
        let k = self.sprite_scale();
        let st = &art.style;
        let mut out = vec![];
        let mut put = |name: &str, salt: &str, at: Pos2, units: f32, tint: Color32| {
            if let Some(sp) = art.sprite(name, salt) {
                let foot = self.to_screen(at);
                out.push((foot.y, sprite_shape(sp, foot, units * k, tint)));
            }
        };
        for (p, kind, salt) in &self.nature {
            let names = st.terrain.get(kind).map_or(&[][..], |n| &n[..]);
            if let Some(name) = names.get(hash(salt) as usize % names.len().max(1)) {
                put(name, salt, *p, NATURE, Color32::WHITE);
            }
        }
        let capitals = state_capitals(w, &self.capitals);
        let marks = marks(w, d, art);
        for (id, l) in &self.lands {
            let Some(p) = w.provinces.get(id) else {
                continue;
            };
            let a = l.anchor + vec2(0.0, 3.0);
            let capital = capitals.contains(id);
            let (town, h) = match capital {
                true => (st.capital.as_str(), CAPITAL),
                false => (settlement(art, p.population), SETTLEMENT),
            };
            put(town, &id.0, a, h, Color32::WHITE);
            if capital {
                // On top of the capital at any zoom: its height in map units shrinks as it does.
                let top = (h + 0.5) / self.zoom.get().sqrt();
                put(&st.crown, &id.0, a - vec2(0.0, top), 7.0, Color32::WHITE);
            }
            for (i, (b, built)) in buildings(w, d, id).into_iter().enumerate() {
                let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                let at = a + vec2(side * (12.0 + 10.0 * (i / 2) as f32), 2.0);
                let name = (st.in_capital.get(&b.sprite)).filter(|_| capital).unwrap_or(&b.sprite);
                if built {
                    put(name, &id.0, at, BUILDING, Color32::WHITE);
                } else {
                    put(name, &id.0, at, BUILDING, Color32::WHITE.gamma_multiply(UNDERWAY_ALPHA));
                    let over = st.underway.get(&b.sprite).unwrap_or(&st.scaffold);
                    put(over, &id.0, at + vec2(0.0, 0.5), BUILDING, Color32::WHITE);
                }
            }
            for (i, (_, sprite)) in marks.iter().filter(|(m, _)| m == id).enumerate() {
                let at = l.anchor + vec2(-11.0 - 11.0 * i as f32, -6.0);
                put(sprite, &id.0, at, MARK, Color32::WHITE);
            }
        }
        // A banner in its colour at the seat of every vassal house: the land of its name, or
        // the first it holds.
        for v in w.vassals.keys() {
            let holder = Holder::Vassal(v.clone());
            let held = |id: &&ProvinceId| w.provinces.get(*id).is_some_and(|p| p.holder == holder);
            let seat = (self.lands.keys().filter(held).find(|id| id.0 == v.0))
                .or_else(|| self.lands.keys().find(held));
            if let Some(id) = seat {
                let tint = lerp(Color32::WHITE, holder_color(w, &holder), 0.6);
                put(&st.banner, &id.0, self.lands[id].anchor + vec2(8.0, -8.0), 11.0, tint);
            }
        }
        out
    }

    /// The name of every province under its settlement, in ink on a halo of paper; a
    /// state's capital larger (stage 29d); the province fought for in rubric with ⚔. None on
    /// a small map.
    fn names(&self, painter: &Painter, w: &World) {
        if self.small() {
            return;
        }
        let px = (NAME_PX * self.text_scale()).min(20.0);
        let war_target = w.war.as_ref().and_then(|x| x.target.as_ref());
        let capitals = self.capitals(w);
        for (id, l) in &self.lands {
            let Some(p) = w.provinces.get(id) else {
                continue;
            };
            let (name, color) = match war_target == Some(id) {
                true => (format!("⚔ {}", p.name), RUBRIC),
                false => (p.name.clone(), FG),
            };
            let at = self.to_screen(l.anchor + vec2(0.0, 5.0));
            let px = if capitals.contains(id) { px * CAPITAL_NAME } else { px };
            let g = painter.layout_no_wrap(name, map_font(px), color);
            let at = at - vec2(g.size().x / 2.0, 0.0);
            let label = (Click::Province(id.clone()), Rect::from_min_size(at, g.size()));
            self.labels.borrow_mut().push(label);
            halo(painter, at, g, color);
        }
    }

    /// Every state's name on a ribbon over the largest connected piece of its land, along it
    /// (upright for a tall land). Stage 29c: of the places about that piece the ribbon takes
    /// one where it covers no other ribbon or the cartouche, lies on the piece and covers the
    /// fewest settlements (stage 29d: and rivers); finding none it is written smaller, so
    /// shorter. On a small map a ribbon with no such place on the piece is left out.
    fn ribbons(&self, painter: &Painter, w: &World, d: &Data, art: &Art) {
        let px = (STATE_PX * self.text_scale()).clamp(MIN_PX, 24.0);
        let Some(tex) = art.sprite("ribbon", "") else {
            return;
        };
        // The settlements and their names stay in sight if they can, and the rivers.
        let k = self.sprite_scale();
        let towns: Vec<Rect> = (self.lands.values())
            .map(|l| {
                let p = self.to_screen(l.anchor);
                Rect::from_min_max(p - vec2(10.0, CAPITAL) * k, p + vec2(10.0 * k, 5.0 * k + 10.0))
            })
            .collect();
        let rivers: Vec<Pos2> = self.rivers.iter().flatten().map(|p| self.to_screen(*p)).collect();
        let view = self.rect.get();
        let cartouche = (view.width() >= OVERLAYS).then(|| {
            let (c, _, _, at) = self.cartouche(painter, w, d);
            c.plate.translate(at.to_vec2())
        });
        let mut placed: Vec<Rect> = cartouche.into_iter().collect();
        // Looked for again only when the land or the view changes.
        let holders = (self.lands.keys().filter_map(|id| w.provinces.get(id))).map(|p| p.holder.clone());
        let key = (holders.collect(), view, self.zoom.get(), self.pan.get(), cartouche);
        let cached = (self.spots.borrow().as_ref()).filter(|(k, _)| *k == key).map(|(_, s)| s.clone());
        let mut spots = BTreeMap::new();
        let mut states: Vec<Option<&NeighbourId>> = vec![None];
        states.extend(w.neighbours.keys().map(Some));
        for s in states {
            let mine = self.largest_piece(w, s);
            let Some(at) = self.label_point(&mine) else {
                continue;
            };
            let all: Vec<Pos2> = mine.iter().flat_map(|id| self.lands[*id].raw.clone()).collect();
            let b = Rect::from_points(&all);
            let angle = match b.height() > 1.7 * b.width() {
                true => -std::f32::consts::FRAC_PI_2,
                false => 0.0,
            };
            let click = match s {
                None => Click::Kingdom,
                Some(n) => Click::State(n.clone()),
            };
            let name = state_name(w, s);
            let found = match &cached {
                Some(c) => c.get(&s.cloned()).copied(),
                None => {
                    let boxes: Vec<(Rect, &[Pos2])> = (mine.iter())
                        .map(|id| (Rect::from_points(&self.lands[*id].raw), &self.lands[*id].raw[..]))
                        .collect();
                    let on = |p: Pos2| {
                        let p = self.to_map(p);
                        boxes.iter().any(|(b, raw)| b.contains(p) && contains(raw, p))
                    };
                    // About the middle of the piece first, then about each of its settlements.
                    let mut bases = vec![self.to_screen(at)];
                    bases.extend(mine.iter().map(|id| self.to_screen(self.lands[*id].anchor - vec2(0.0, 12.0))));
                    let mut best: Option<(usize, f32, Pos2)> = None;
                    for smaller in [1.0, 0.85, 0.72] {
                        let g = painter.layout_no_wrap(name.clone(), map_font(px * smaller), FG);
                        let r = lay(&RIBBON, g.size());
                        let across = rotate(vec2(0.0, r.plate.height() * 1.15), angle);
                        let along = rotate(vec2(r.plate.width() * 0.25, 0.0), angle);
                        let mut tries = vec![];
                        for b in &bases {
                            for j in [0.0, -1.0, 1.0, -2.0, 2.0, -3.0, 3.0] {
                                for i in [0.0, -1.0, 1.0] {
                                    tries.push(*b + across * j + along * i);
                                }
                            }
                        }
                        // Covering a ribbon costs more than anything but leaving the view;
                        // lying off the piece more than covering towns and rivers.
                        let cost = |c: &Pos2| {
                            let rib = ribbon_at(&r, *c, angle);
                            let place = |p: Pos2| *c + rotate(p - r.plate.center(), angle);
                            let off = corners(r.zone).into_iter().map(place).filter(|p| !on(*p)).count();
                            let crossed = placed.iter().filter(|p| p.intersects(rib)).count();
                            let covers = towns.iter().filter(|t| t.shrink(2.0).intersects(rib)).count();
                            let wet = rivers.iter().any(|p| rib.contains(*p)) as usize;
                            1000 * !view.contains_rect(rib) as usize
                                + 100 * crossed
                                + 10 * (off + !on(*c) as usize)
                                + covers
                                + 10 * wet
                        };
                        // The nearest to the middle of those that cost least; the first that costs
                        // nothing saves looking further.
                        tries.sort_by(|a, b| a.distance(bases[0]).total_cmp(&b.distance(bases[0])));
                        let mut pick: Option<(usize, Pos2)> = None;
                        for t in &tries {
                            let paid = cost(t);
                            if pick.is_none_or(|(least, _)| paid < least) {
                                pick = Some((paid, *t));
                            }
                            if paid == 0 {
                                break;
                            }
                        }
                        let Some((paid, c)) = pick else {
                            continue;
                        };
                        if best.as_ref().is_none_or(|b| paid < b.0) {
                            best = Some((paid, smaller, c));
                        }
                        if paid < 100 {
                            break;
                        }
                    }
                    // A small map names a state only over its land, covering none other.
                    let fits = |b: &(usize, f32, Pos2)| b.0 < 30 || !self.small();
                    best.filter(fits).map(|(_, smaller, c)| (smaller, c))
                }
            };
            let Some((smaller, centre)) = found else {
                continue;
            };
            spots.insert(s.cloned(), (smaller, centre));
            let g = painter.layout_no_wrap(name, map_font(px * smaller), FG);
            let r = lay(&RIBBON, g.size());
            let place = |p: Pos2| centre + rotate(p - r.plate.center(), angle);
            let mut mesh = Mesh::with_texture(tex.id);
            for (local, uv) in slices(&RIBBON, r.scale, r.stretch) {
                quad(&mut mesh, corners(local).map(place), corners(uv), Color32::WHITE);
            }
            painter.add(mesh);
            painter.add(TextShape::new(place(r.text.min), g, FG).with_angle(angle));
            let rib = ribbon_at(&r, centre, angle);
            placed.push(rib);
            self.hits.borrow_mut().push((rib, click.clone()));
            self.labels.borrow_mut().push((click, rib));
        }
        *self.spots.borrow_mut() = Some((key, spots));
    }

    /// The provinces of state `s` (ours: None) on the map: the largest connected piece of
    /// them by area.
    fn largest_piece(&self, w: &World, s: Option<&NeighbourId>) -> Vec<&ProvinceId> {
        let mut left: BTreeSet<&ProvinceId> = (self.lands.keys())
            .filter(|id| w.provinces.get(*id).is_some_and(|p| state_of(&p.holder) == s))
            .collect();
        let area = |piece: &[&ProvinceId]| piece.iter().map(|id| self.lands[*id].area).sum::<f32>();
        let mut best = vec![];
        while let Some(first) = left.pop_first() {
            let mut piece = vec![first];
            let mut i = 0;
            while i < piece.len() {
                for n in &w.provinces[piece[i]].neighbours {
                    piece.extend(left.take(n));
                }
                i += 1;
            }
            if area(&piece) > area(&best) {
                best = piece;
            }
        }
        best
    }

    /// The cartouche of the kingdom in the top left corner of the view: its plate laid round
    /// the title and the year, and where its top left goes.
    fn cartouche(&self, painter: &Painter, w: &World, d: &Data) -> (Laid, Arc<Galley>, Arc<Galley>, Pos2) {
        let r = self.rect.get();
        let (title, year) = cartouche_text(w, d);
        let px = (r.height() * 0.026).clamp(13.0, 19.0);
        let title = painter.layout_no_wrap(title, map_font(px), FG);
        let year = painter.layout_no_wrap(year, map_font(px * 0.8), RUBRIC);
        let c = cartouche(&title, &year);
        (c, title, year, r.left_top() + vec2(14.0, 10.0))
    }

    /// Where a state's name goes: the middle of its land when that is its own, else its
    /// largest province; above a settlement it would cover.
    fn label_point(&self, mine: &[&ProvinceId]) -> Option<Pos2> {
        let size = |id: &&&ProvinceId| self.lands[**id].area;
        let largest = mine.iter().max_by(|a, b| size(a).total_cmp(&size(b)))?;
        let total: f32 = mine.iter().map(|id| self.lands[*id].area).sum();
        let mid = (mine.iter()).fold(Vec2::ZERO, |s, id| {
            let l = &self.lands[*id];
            s + centroid(&l.raw).to_vec2() * l.area / total
        });
        let mid = mid.to_pos2();
        let at = match mine.iter().any(|id| contains(&self.lands[*id].raw, mid)) {
            true => mid,
            false => self.lands[*largest].anchor,
        };
        let near = mine.iter().any(|id| self.lands[*id].anchor.distance(at) < 16.0);
        Some(if near { at - vec2(0.0, 24.0) } else { at })
    }

    /// Over the map on screen: the ornaments in the corners, the compass rose, the cartouche
    /// of the kingdom with its name and year. Stage 29c: no legend, the ribbons name the states.
    fn overlays(&self, painter: &Painter, w: &World, d: &Data, art: &Art) {
        let r = self.rect.get();
        if let Some(c) = art.sprite("corner", "") {
            let size = vec2(c.size.x / c.size.y, 1.0) * (r.height() * 0.11).clamp(40.0, 80.0);
            for (fx, fy) in [(false, false), (true, false), (false, true), (true, true)] {
                let x = if fx { r.right() - size.x } else { r.left() };
                let y = if fy { r.bottom() - size.y } else { r.top() };
                let (u0, u1) = if fx { (1.0, 0.0) } else { (0.0, 1.0) };
                let (v0, v1) = if fy { (1.0, 0.0) } else { (0.0, 1.0) };
                let mut m = Mesh::with_texture(c.id);
                let uv = [pos2(u0, v0), pos2(u1, v0), pos2(u1, v1), pos2(u0, v1)];
                quad(&mut m, corners(Rect::from_min_size(pos2(x, y), size)), uv, Color32::WHITE);
                painter.add(m);
            }
        }
        if let Some(c) = art.sprite("compass", "") {
            let h = (r.height() * 0.16).clamp(50.0, 110.0);
            let foot = r.right_top() + vec2(-h * 0.8, h + 14.0);
            painter.add(sprite_shape(c, foot, h, Color32::WHITE));
        }
        // The cartouche: «Королевство Ульриха», «лета 1195».
        let (c, title, year, at) = self.cartouche(painter, w, d);
        let place = |p: Pos2| at + p.to_vec2();
        if let Some(sp) = art.sprite("cartouche", "") {
            let mut mesh = Mesh::with_texture(sp.id);
            for (local, uv) in slices(&CARTOUCHE, c.scale, c.stretch) {
                quad(&mut mesh, corners(local).map(place), corners(uv), Color32::WHITE);
            }
            painter.add(mesh);
        }
        let x = |g: &Arc<Galley>| c.text.center().x - g.size().x / 2.0;
        let below = c.text.top() + title.size().y;
        painter.galley(place(pos2(x(&year), below)), year, RUBRIC);
        painter.galley(place(pos2(x(&title), c.text.top())), title, FG);
        let plate = Rect::from_min_max(place(c.plate.min), place(c.plate.max));
        self.hits.borrow_mut().push((plate, Click::Kingdom));
    }
}

/// The state of a holder: ours (None) for the crown and its vassals.
fn state_of(h: &Holder) -> Option<&NeighbourId> {
    match h {
        Holder::Foreign(n) => Some(n),
        _ => None,
    }
}

/// The name of state `s` on the map: «Королевство» for ours.
pub(crate) fn state_name(w: &World, s: Option<&NeighbourId>) -> String {
    match s.and_then(|n| w.neighbours.get(n)) {
        Some(n) => n.name.clone(),
        None => "Королевство".into(),
    }
}

/// The ink between the two sides of an edge (`None` the sea).
fn ink_of(a: Option<&Holder>, b: Option<&Holder>) -> Ink {
    match (a, b) {
        (Some(a), Some(b)) if a == b => Ink::Dotted,
        (Some(a), Some(b)) if state_of(a) == state_of(b) => Ink::Dashed,
        _ => Ink::Solid,
    }
}

/// The capital of every state: ours, and of each foreign one the capital it started with
/// while it holds it, else its most peopled province.
fn state_capitals(w: &World, start: &BTreeMap<NeighbourId, ProvinceId>) -> BTreeSet<ProvinceId> {
    let mut out = BTreeSet::from([w.capital.province.clone()]);
    for n in w.neighbours.keys() {
        let holder = Holder::Foreign(n.clone());
        let theirs = || w.provinces.values().filter(|p| p.holder == holder);
        let first = start.get(n).filter(|id| theirs().any(|p| p.id == **id)).cloned();
        let peopled = || theirs().max_by_key(|p| p.population).map(|p| p.id.clone());
        out.extend(first.or_else(peopled));
    }
    out
}

/// The settlement of a province of `people`: the last of `settlements` they reached.
fn settlement(art: &Art, people: i32) -> &str {
    let reached = art.style.settlements.iter().filter(|(from, _)| people >= *from);
    reached.last().map_or("", |(_, s)| s.as_str())
}

/// The marks of what goes on (`data/sprites.ron` `marks`): which province shows which sprite.
pub(crate) fn marks(w: &World, d: &Data, art: &Art) -> Vec<(ProvinceId, String)> {
    let war = w.war.as_ref();
    let enemy = war.map(|x| Holder::Foreign(x.enemy.clone()));
    let own = |p: &bd_core::state::Province| !matches!(p.holder, Holder::Foreign(_));
    let mut out = vec![];
    for m in &art.style.marks {
        let here = |p: &&bd_core::state::Province| match &m.when {
            When::Flag(f) => w.flags.contains(f) && p.id == w.capital.province,
            When::Unrest => own(p) && p.loyalty < d.unrest_below,
            When::WarBorder => {
                let theirs = |id: &ProvinceId| w.provinces.get(id).map(|q| &q.holder) == enemy.as_ref();
                own(p) && p.neighbours.iter().any(theirs)
            }
            When::WarTarget => war.and_then(|x| x.target.as_ref()) == Some(&p.id),
            When::Fewer(n) => p.population < *n,
        };
        out.extend(w.provinces.values().filter(here).map(|p| (p.id.clone(), m.sprite.clone())));
    }
    out
}

/// «Королевство Ульриха», «лета 1195».
pub(crate) fn cartouche_text(w: &World, d: &Data) -> (String, String) {
    let name = d.names.declined(&w.ruler.name, 1);
    let year = w.tick.date(w.time_unit, w.start_year);
    (format!("Королевство {name}"), format!("лета {year}"))
}

/// A sprite standing on `foot` (the middle of its bottom), `h` pixels tall.
fn sprite_shape(s: Sprite, foot: Pos2, h: f32, tint: Color32) -> Shape {
    let size = vec2(s.size.x / s.size.y * h, h);
    let rect = Rect::from_min_size(foot - vec2(size.x / 2.0, size.y), size);
    Shape::image(s.id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint)
}

/// The bounds on screen of a ribbon laid as `r` about `centre` and turned by `angle`.
fn ribbon_at(r: &Laid, centre: Pos2, angle: f32) -> Rect {
    Rect::from_points(&corners(r.plate).map(|p| centre + rotate(p - r.plate.center(), angle)))
}

/// `g` with its top left at `at`, over a halo of paper.
fn halo(painter: &Painter, at: Pos2, g: Arc<Galley>, color: Color32) {
    for d in [vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, -1.0), vec2(0.0, 1.0)] {
        let paper = BG.gamma_multiply(0.85);
        painter.add(TextShape::new(at + d, g.clone(), color).with_override_text_color(paper));
    }
    painter.add(TextShape::new(at, g, color));
}

/// A plate cut in three: its caps (to `left` and from `right`, pixels of its PNG `size`) kept,
/// the middle stretched to the text; `zone` is where text goes, in PNG pixels unstretched.
pub(crate) struct Plate {
    size: Vec2,
    left: f32,
    right: f32,
    zone: Rect,
}

/// decor/ribbon-1.png.
pub(crate) const RIBBON: Plate = Plate {
    size: vec2(160.0, 27.0),
    left: 40.0,
    right: 118.0,
    zone: Rect::from_min_max(pos2(38.0, 3.0), pos2(121.0, 15.5)),
};

/// decor/cartouche-1.png.
pub(crate) const CARTOUCHE: Plate = Plate {
    size: vec2(160.0, 67.0),
    left: 40.0,
    right: 110.0,
    zone: Rect::from_min_max(pos2(32.0, 18.0), pos2(117.0, 49.0)),
};

/// A plate laid round a text block: its scale (screen px per PNG px) and the PNG pixels its
/// middle is stretched by; on screen with the plate's top left at 0, the plate, its text zone
/// and the text block centred in it.
#[derive(Debug)]
pub(crate) struct Laid {
    scale: f32,
    stretch: f32,
    pub(crate) plate: Rect,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) zone: Rect,
    pub(crate) text: Rect,
}

/// The plate sized to a text block of `size`: the zone as tall as the text and a margin,
/// the middle stretched as far as the text needs. Font and plate go together.
pub(crate) fn lay(p: &Plate, size: Vec2) -> Laid {
    let scale = size.y / (p.zone.height() * 0.92);
    let stretch = (size.x / (scale * 0.9) - p.zone.width()).max(0.0);
    let plate = Rect::from_min_size(Pos2::ZERO, vec2(p.size.x + stretch, p.size.y) * scale);
    let zone = Rect::from_min_max(
        (p.zone.min.to_vec2() * scale).to_pos2(),
        pos2((p.zone.max.x + stretch) * scale, p.zone.max.y * scale),
    );
    Laid {
        scale,
        stretch,
        plate,
        zone,
        text: Rect::from_center_size(zone.center(), size),
    }
}

/// The cartouche round a title and the year under it.
pub(crate) fn cartouche(title: &Galley, year: &Galley) -> Laid {
    let size = vec2(title.size().x.max(year.size().x), title.size().y + year.size().y);
    lay(&CARTOUCHE, size)
}

/// The three pieces of a laid plate: on screen (top left at 0) and in the texture.
fn slices(p: &Plate, scale: f32, stretch: f32) -> [(Rect, Rect); 3] {
    let xs = [0.0, p.left, p.right + stretch, p.size.x + stretch];
    let us = [0.0, p.left, p.right, p.size.x];
    std::array::from_fn(|i| {
        let local = Rect::from_min_max(pos2(xs[i] * scale, 0.0), pos2(xs[i + 1] * scale, p.size.y * scale));
        let uv = Rect::from_min_max(pos2(us[i] / p.size.x, 0.0), pos2(us[i + 1] / p.size.x, 1.0));
        (local, uv)
    })
}

/// Clockwise from the top left.
fn corners(r: Rect) -> [Pos2; 4] {
    [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
}

fn rotate(v: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// A textured quad: corners clockwise from the top left and their texture points.
fn quad(m: &mut Mesh, pts: [Pos2; 4], uv: [Pos2; 4], color: Color32) {
    let first = m.vertices.len() as u32;
    (m.vertices).extend(pts.into_iter().zip(uv).map(|(pos, uv)| Vertex { pos, uv, color }));
    m.indices.extend([0, 1, 2, 0, 2, 3].map(|i| first + i));
}

/// A band of `width` along `pts` (back to the first when `closed`), textured by `tex`
/// repeating every `repeat` along it: u is the length so far over `repeat`; v is 0 on the line
/// and 1 at `width` to its left (`side` 1) or right (-1), or 0 and 1 at the right and left
/// edges of a band centred on it (0). Mitred joins: neighbouring quads share their vertices.
pub fn band(
    pts: &[Pos2],
    closed: bool,
    width: f32,
    side: f32,
    repeat: f32,
    tex: TextureId,
    color: Color32,
) -> Mesh {
    let mut m = Mesh::with_texture(tex);
    let mut pts = pts.to_vec();
    pts.dedup_by(|a, b| a.distance(*b) < 1e-3);
    if closed && pts.len() > 2 {
        pts.push(pts[0]);
    }
    let n = pts.len();
    if n < 2 {
        return m;
    }
    let left = |a: Pos2, b: Pos2| {
        let d = (b - a).normalized();
        vec2(-d.y, d.x)
    };
    let (near, far) = match side {
        s if s > 0.0 => (0.0, 1.0),
        s if s < 0.0 => (0.0, -1.0),
        _ => (-0.5, 0.5),
    };
    let mut u = 0.0;
    for i in 0..n {
        let prev = match i {
            0 if closed => Some(left(pts[n - 2], pts[0])),
            0 => None,
            _ => Some(left(pts[i - 1], pts[i])),
        };
        let next = match i {
            _ if i + 1 < n => Some(left(pts[i], pts[i + 1])),
            _ if closed => Some(left(pts[0], pts[1])),
            _ => None,
        };
        let normal = match (prev, next) {
            (Some(a), Some(b)) => {
                let mid = (a + b).normalized();
                mid / mid.dot(b).max(0.35)
            }
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => Vec2::ZERO,
        };
        if i > 0 {
            u += pts[i].distance(pts[i - 1]) / repeat;
        }
        for (k, o) in [near, far].into_iter().enumerate() {
            let (pos, uv) = (pts[i] + normal * width * o, pos2(u, k as f32));
            m.vertices.push(Vertex { pos, uv, color });
        }
        if i > 0 {
            let j = 2 * i as u32;
            m.indices.extend([j - 2, j, j - 1, j - 1, j, j + 1]);
        }
    }
    m
}

/// A Catmull-Rom curve from `p[1]` to `p[2]`, `p[0]` and `p[3]` their neighbours: a point
/// every `STEP` or so, both ends exact (stage 29d: the coast and the rivers).
fn spline(p: [Pos2; 4]) -> Vec<Pos2> {
    let n = ((p[2] - p[1]).length() / STEP).ceil().max(1.0) as usize;
    let [a, b, c, d] = p.map(|q| q.to_vec2());
    let mut out: Vec<Pos2> = (0..=n)
        .map(|k| {
            let t = k as f32 / n as f32;
            let (t2, t3) = (t * t, t * t * t);
            let q = b * 2.0 + (c - a) * t + (a * 2.0 - b * 5.0 + c * 4.0 - d) * t2 + (b * 3.0 - a - c * 3.0 + d) * t3;
            (q / 2.0).to_pos2()
        })
        .collect();
    (out[0], out[n]) = (p[1], p[2]);
    out
}

/// An open polyline as a spline through its points.
fn smooth(p: &[Pos2]) -> Vec<Pos2> {
    let mut out: Vec<Pos2> = vec![];
    for i in 0..p.len().saturating_sub(1) {
        let s = spline([p[i.saturating_sub(1)], p[i], p[i + 1], p[(i + 2).min(p.len() - 1)]]);
        out.extend(&s[(i > 0) as usize..]);
    }
    out
}

/// A point moved a little by its own coordinates: alike wherever it is drawn from.
fn wobble(p: Pos2) -> Pos2 {
    let h = hash(&format!("{:.2},{:.2}", p.x, p.y));
    p + vec2(unit(h) - 0.5, unit(h >> 24) - 0.5) * 2.0 * WOBBLE
}

/// 20 bits of `h` as a number in [0, 1).
fn unit(h: u64) -> f32 {
    (h & 0xfffff) as f32 / 0x100000 as f32
}

/// `a` toward `b` by `t`.
fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The buildings of province `id` the data name (`Data.buildings`), in their order: built
/// (true) and being built (false) by an action; in the capital also those told by flags.
pub fn buildings<'a>(w: &World, data: &'a Data, id: &ProvinceId) -> Vec<(&'a BuildingDef, bool)> {
    let Some(p) = w.provinces.get(id) else {
        return vec![];
    };
    let started = (w.active_actions.iter())
        .filter(|a| a.target.as_deref() == Some(id.0.as_str()))
        .flat_map(|a| built_by(data, &a.id));
    let started: BTreeSet<&str> = started.collect();
    let capital = *id == w.capital.province;
    let flag = |f: &Option<String>| capital && f.as_ref().is_some_and(|f| w.flags.contains(f));
    (data.buildings.iter())
        .filter_map(|b| match p.buildings.contains(&b.id) || flag(&b.flag) {
            true => Some((b, true)),
            false => (started.contains(b.id.as_str()) || flag(&b.underway)).then_some((b, false)),
        })
        .collect()
}

/// The buildings action `id` puts up when done (`Effect::Build`).
pub(crate) fn built_by<'a>(data: &'a Data, id: &str) -> Vec<&'a str> {
    let def = data.actions.iter().find(|a| a.id == id);
    let effects = def.map_or(&[][..], |a| &a.on_complete);
    (effects.iter())
        .filter_map(|e| match e {
            Effect::Build(_, b) => Some(b.as_str()),
            _ => None,
        })
        .collect()
}

/// «корона», «вассал Вейр», «Нордмарк».
pub fn holder_name(w: &World, h: &Holder) -> String {
    match h {
        Holder::Crown => "корона".into(),
        Holder::Vassal(v) => format!("вассал {}", w.vassals.get(v).map_or(&v.0, |v| &v.name)),
        Holder::Foreign(n) => w.neighbours.get(n).map_or(n.0.clone(), |n| n.name.clone()),
    }
}

/// Crown gold; each vassal house a colour by its order, each foreign state by its
/// `Neighbour.ordinal`, so a new state does not repaint the old ones.
pub(crate) fn holder_color(w: &World, h: &Holder) -> Color32 {
    match h {
        Holder::Crown => CROWN,
        Holder::Vassal(v) => {
            VASSALS[w.vassals.keys().position(|x| x == v).unwrap_or(0) % VASSALS.len()]
        }
        // Stage 28: a state with a colour of its own in the preset (the empire) keeps it.
        Holder::Foreign(n) => match w.neighbours.get(n) {
            Some(n) if n.color.is_some() => {
                let (r, g, b) = n.color.unwrap_or_default();
                Color32::from_rgb(r, g, b)
            }
            n => FOREIGN[n.map_or(0, |n| n.ordinal as usize) % FOREIGN.len()],
        },
    }
}

/// An `Fx` rounded to a whole number for display.
pub fn round(v: bd_core::fx::Fx) -> String {
    format!("{:.0}", v.0 as f64 / 1000.0)
}

/// Shoelace area, positive with the interior on the left of the edges.
fn signed_area(p: &[Pos2]) -> f32 {
    let n = p.len();
    let twice: f32 = (0..n)
        .map(|i| p[i].x * p[(i + 1) % n].y - p[(i + 1) % n].x * p[i].y)
        .sum();
    twice / 2.0
}

/// The centre of mass of an outline.
fn centroid(p: &[Pos2]) -> Pos2 {
    let n = p.len();
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in 0..n {
        let (q, r) = (p[i], p[(i + 1) % n]);
        let c = q.x * r.y - r.x * q.y;
        (cx, cy) = (cx + (q.x + r.x) * c, cy + (q.y + r.y) * c);
    }
    let a6 = 6.0 * signed_area(p);
    pos2(cx / a6, cy / a6)
}

/// Where a province's settlement stands: its centre of mass, or the vertex mean when that
/// falls outside a concave outline.
fn anchor(p: &[Pos2]) -> Pos2 {
    let c = centroid(p);
    if contains(p, c) {
        return c;
    }
    let sum = p.iter().fold(Vec2::ZERO, |s, q| s + q.to_vec2());
    (sum / p.len() as f32).to_pos2()
}

/// Even-odd ray casting.
fn contains(poly: &[Pos2], p: Pos2) -> bool {
    let n = poly.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

/// Ear clipping of a simple polygon in either winding: triangle vertex indices.
/// egui fills only convex paths, and the wobbled provinces are concave.
fn triangulate(p: &[Pos2]) -> Vec<u32> {
    let n = p.len();
    let area2: f32 = (0..n)
        .map(|i| p[i].x * p[(i + 1) % n].y - p[(i + 1) % n].x * p[i].y)
        .sum();
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::new();
    while idx.len() > 3 {
        let m = idx.len();
        let corner = |i: usize| [idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]];
        let ear = (0..m).find(|&i| {
            let [a, b, c] = corner(i).map(|k| p[k]);
            let convex = ((b - a).x * (c - b).y - (b - a).y * (c - b).x) * area2 > 0.0;
            convex
                && idx
                    .iter()
                    .all(|&k| [a, b, c].contains(&p[k]) || !contains(&[a, b, c], p[k]))
        });
        // Only a degenerate outline has no ear; paint what was found.
        let Some(i) = ear else { return out };
        out.extend(corner(i).map(|k| k as u32));
        idx.remove(i);
    }
    out.extend(idx.iter().map(|&k| k as u32));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(p: &[Pos2]) -> f32 {
        signed_area(p).abs()
    }

    #[test]
    fn concave_outline_is_covered_by_its_triangles() {
        // An arrow head: the vertex (2, 1) is reflex.
        let p = [
            pos2(0.0, 0.0),
            pos2(4.0, 0.0),
            pos2(2.0, 1.0),
            pos2(4.0, 4.0),
            pos2(0.0, 4.0),
        ];
        let t = triangulate(&p);
        assert_eq!(t.len(), 3 * (p.len() - 2));
        let tri_area: f32 = t
            .chunks(3)
            .map(|c| area(&[p[c[0] as usize], p[c[1] as usize], p[c[2] as usize]]))
            .sum();
        assert_eq!(tri_area, area(&p));
        assert!(contains(&p, pos2(1.0, 1.0)));
        assert!(!contains(&p, pos2(3.5, 1.5)));
    }

    fn square() -> Map {
        let square = [(0, 0), (100, 0), (100, 100), (0, 100)].to_vec();
        Map {
            polygons: [(ProvinceId("p".into()), square)].into(),
            ..Map::default()
        }
    }

    /// Stage 28: the wheel zooms round the pointer, a drag moves the view, and the view
    /// never leaves the map: at zoom 1 it is the whole map (stage 29: with its sea).
    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let map = MapView::new(&square(), None);
        let side = 100.0 + 2.0 * SEA;
        map.rect
            .set(Rect::from_min_size(pos2(0.0, 0.0), vec2(side, side)));
        let pointer = pos2(150.0, 50.0);
        let under = map.to_map(pointer);
        map.zoom_at(pointer, 2.0);
        assert!((map.to_map(pointer) - under).length() < 1e-3);
        map.move_by(vec2(1000.0, 0.0));
        assert_eq!(map.pan.get().x, side / 4.0, "no further than the edge");
        map.zoom_at(pointer, 0.1);
        assert_eq!((map.zoom.get(), map.pan.get()), (1.0, Vec2::ZERO));
        map.zoom_at(pointer, 100.0);
        assert_eq!(map.zoom.get(), MAX_ZOOM);
    }

    /// Acceptance: a band along a polyline is continuous in u: the u where a segment ends is
    /// the u where the next begins (they share the vertices), and u grows with the length.
    #[test]
    fn a_band_is_continuous_along_its_line() {
        let line = [pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(10.0, 5.0), pos2(3.0, 9.0)];
        for closed in [false, true] {
            let m = band(&line, closed, 2.0, 1.0, 4.0, TextureId::default(), Color32::WHITE);
            let quads: Vec<&[u32]> = m.indices.chunks(6).collect();
            assert_eq!(quads.len(), line.len() - 1 + closed as usize);
            let u = |i: &u32| m.vertices[*i as usize].uv.x;
            for q in quads.windows(2) {
                let end = q[0].iter().map(u).fold(f32::MIN, f32::max);
                let start = q[1].iter().map(u).fold(f32::MAX, f32::min);
                assert_eq!(end, start);
                assert!(end > q[0].iter().map(u).fold(f32::MAX, f32::min));
            }
            let open: f32 = line.windows(2).map(|w| w[0].distance(w[1])).sum();
            let total = open + if closed { line[3].distance(line[0]) } else { 0.0 };
            let last = m.vertices.last().unwrap().uv.x;
            assert!((last - total / 4.0).abs() < 1e-4, "{last}");
            // On the line at v 0, off it to the left at v 1.
            assert_eq!(m.vertices[0].pos, line[0]);
            assert!(m.vertices[1].pos.y > 0.0);
        }
    }

    /// Neighbouring provinces wobble their common border alike: the same points either way;
    /// the boundary of the two is one loop without it.
    #[test]
    fn a_common_border_wobbles_alike_on_both_sides() {
        let mut map = square();
        let right = [(100, 0), (200, 0), (200, 100), (100, 100)].to_vec();
        map.polygons.insert(ProvinceId("q".into()), right);
        let map = MapView::new(&map, None);
        let a = &map.lands[&ProvinceId("p".into())].outline;
        let b = &map.lands[&ProvinceId("q".into())].outline;
        let shared: Vec<&Pos2> = a.iter().filter(|x| b.contains(x)).collect();
        assert!(shared.len() >= 100 / STEP as usize, "{shared:?}");
        assert!(shared.iter().any(|x| x.x != 100.0), "they wobble");
        let loops = map.loops(|_| true);
        assert_eq!(loops.len(), 1);
        assert!(!loops[0].iter().any(|x| shared.contains(&x) && x.y > 5.0 && x.y < 95.0));
    }

    /// Acceptance (stage 29c): the land of any two states, the foreign palette, our gold and
    /// the empire's red (its own in the preset), washed over the parchment as `washes` paints
    /// it (the paper tile's mean colour times the wash), differ by CIE76 ΔE of at least 10 in
    /// Lab (2.3 is barely seen, 10 is plain side by side; the palette of stage 29 had 5 for
    /// the empire and Зюдмарк).
    #[test]
    fn the_states_differ_on_parchment() {
        let paper = [217.0, 201.0, 174.0];
        let lab = |c: Color32| {
            let c = lerp(Color32::WHITE, c, WASH);
            let lin = |i: usize, v: u8| {
                let v = paper[i] * v as f32 / 255.0 / 255.0;
                if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            let (r, g, b) = (lin(0, c.r()), lin(1, c.g()), lin(2, c.b()));
            let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
            let x = f((0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047);
            let y = f(0.2126 * r + 0.7152 * g + 0.0722 * b);
            let z = f((0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883);
            [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
        };
        let de = |a: Color32, b: Color32| {
            let (a, b) = (lab(a), lab(b));
            (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
        };
        let mut all = FOREIGN.to_vec();
        all.extend([CROWN, Color32::from_rgb(150, 46, 52)]);
        for (i, c) in all.iter().enumerate() {
            for o in &all[i + 1..] {
                assert!(de(*c, *o) >= 10.0, "{c:?} {o:?}: {}", de(*c, *o));
            }
        }
        // The empire and Зюдмарк of stage 29 fail it.
        assert!(de(Color32::from_rgb(150, 46, 52), Color32::from_rgb(0xb0, 0x57, 0x7a)) < 10.0);
    }

    /// The ink of an edge: the sea and a state's border solid, a vassal's dashed, within one
    /// holder dotted.
    #[test]
    fn the_ink_tells_states_appanages_and_provinces() {
        let crown = Holder::Crown;
        let weir = Holder::Vassal(bd_core::state::VassalId("weir".into()));
        let nord = Holder::Foreign(NeighbourId("nordmark".into()));
        assert_eq!(ink_of(Some(&crown), None), Ink::Solid);
        assert_eq!(ink_of(Some(&crown), Some(&nord)), Ink::Solid);
        assert_eq!(ink_of(Some(&crown), Some(&weir)), Ink::Dashed);
        assert_eq!(ink_of(Some(&crown), Some(&crown)), Ink::Dotted);
        assert_eq!(ink_of(Some(&nord), Some(&nord)), Ink::Dotted);
    }

    fn default_map() -> Map {
        ron::from_str(include_str!("../../../data/maps/default.ron")).expect("the map file")
    }

    /// The turn from `a`→`b` to `b`→`c` in degrees, positive to the right on screen.
    fn turn(a: Pos2, b: Pos2, c: Pos2) -> f32 {
        let t = ((c - b).angle() - (b - a).angle()).to_degrees();
        (t + 540.0).rem_euclid(360.0) - 180.0
    }

    fn to_line(p: Pos2, a: Pos2, b: Pos2) -> f32 {
        let t = ((p - a).dot(b - a) / (b - a).length_sq()).clamp(0.0, 1.0);
        p.distance(a + (b - a) * t)
    }

    /// Acceptance (stage 29d): the coast of the default map is a smooth curve, its own loop:
    /// from one point of it to the next it turns by no more than `SHARPEST` (the outlines it
    /// goes through turn by 90° and more at their corners, the wobble by up to 40°).
    #[test]
    fn the_coast_is_smooth() {
        const SHARPEST: f32 = 25.0;
        let view = MapView::new(&default_map(), None);
        let coast = view.loops(|_| true);
        assert_eq!(coast.len(), 1);
        let l = &coast[0];
        let n = l.len();
        for i in 0..n {
            let t = turn(l[(i + n - 1) % n], l[i], l[(i + 1) % n]);
            assert!(t.abs() <= SHARPEST, "{t}° at {:?}", l[i]);
        }
        // The borders inland meet it with no gap: each ends on it or inside it, not at sea.
        for e in view.edges.iter().filter(|e| e.sides.iter().all(Option::is_some)) {
            for p in [e.pts[0], e.pts[e.pts.len() - 1]] {
                assert!(l.contains(&p) || contains(l, p), "{p:?} at sea");
            }
        }
    }

    /// Acceptance (stage 29d): every river of the default map winds, with `BENDS` bends at
    /// least (a bend: a turn of `BEND`° or more at a point of the file, the other way from the
    /// bend before it); as drawn it runs on land, and its mouth lies on the coast or just past
    /// it, no further than `MOUTH` map units (under the coast band, not out at sea).
    #[test]
    fn the_rivers_wind_and_end_on_the_coast() {
        const BENDS: usize = 4;
        const BEND: f32 = 15.0;
        const MOUTH: f32 = 4.0;
        let map = default_map();
        let view = MapView::new(&map, None);
        let coast = view.loops(|_| true);
        let off = |p: Pos2| {
            let l = &coast[0];
            (0..l.len()).map(|i| to_line(p, l[i], l[(i + 1) % l.len()])).fold(f32::MAX, f32::min)
        };
        assert!(!map.rivers.is_empty());
        for (raw, drawn) in map.rivers.iter().zip(&view.rivers) {
            let pts: Vec<Pos2> = raw.iter().map(|&(x, y)| pos2(x as f32, y as f32)).collect();
            let turns = pts.windows(3).map(|w| turn(w[0], w[1], w[2])).filter(|t| t.abs() >= BEND);
            let turns: Vec<f32> = turns.collect();
            let bends = turns.len().min(1) + turns.windows(2).filter(|t| t[0] * t[1] < 0.0).count();
            assert!(bends >= BENDS, "{bends} bends of {raw:?}");
            let (mouth, course) = drawn.split_last().expect("a river");
            for p in course {
                assert!(contains(&coast[0], *p) || off(*p) <= MOUTH, "{p:?} of {raw:?} at sea");
            }
            assert!(off(*mouth) <= MOUTH, "the mouth {mouth:?} {} off the coast", off(*mouth));
        }
    }
}

