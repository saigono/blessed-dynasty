//! The province map: polygons from the map file painted by holder. Floats are display only.

use bd_core::data::{BuildingDef, Data};
use bd_core::rules::{ActionTarget, Effect, Target};
use bd_core::state::{Holder, ProvinceId, World};
use eframe::egui::{Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

// Light palette of the design doc mockups.
pub const BG: Color32 = Color32::from_rgb(0xed, 0xef, 0xf2);
pub const BG2: Color32 = Color32::from_rgb(0xe2, 0xe5, 0xea);
pub const FG: Color32 = Color32::from_rgb(0x1b, 0x22, 0x30);
pub const FG2: Color32 = Color32::from_rgb(0x52, 0x5b, 0x6b);
pub const RUBRIC: Color32 = Color32::from_rgb(0xb1, 0x36, 0x2c);
pub const CROWN: Color32 = Color32::from_rgb(0xc4, 0x9a, 0x3c);
/// Vassal houses and foreign states by their order in the world.
const VASSALS: [Color32; 3] = [
    Color32::from_rgb(0x7f, 0x98, 0xb8),
    Color32::from_rgb(0xa0, 0x8c, 0xc0),
    Color32::from_rgb(0x6f, 0xa8, 0xc0),
];
const FOREIGN: [Color32; 4] = [
    Color32::from_rgb(0xb5, 0xb9, 0xc2),
    Color32::from_rgb(0xc9, 0xb8, 0xa8),
    Color32::from_rgb(0xbd, 0xc4, 0x9e),
    Color32::from_rgb(0xc2, 0xb0, 0xc0),
];
/// The outer border of every holder's land.
pub const BORDER: Color32 = Color32::from_rgb(0x3a, 0x40, 0x4c);
pub const UNREST: Color32 = Color32::from_rgb(0xd9, 0x64, 0x4f);
pub const GOOD: Color32 = Color32::from_rgb(0x3f, 0x8f, 0x5e);
pub const WARN: Color32 = Color32::from_rgb(0xc4, 0x8a, 0x2a);

const WEAK_ALPHA: f32 = 0.45;
/// A building being built, pale on the map.
pub const UNDERWAY_ALPHA: f32 = 0.3;

pub struct MapView {
    /// Outline in map coordinates and its triangles.
    shapes: BTreeMap<ProvinceId, (Vec<Pos2>, Vec<u32>)>,
    bounds: Rect,
    /// Every outline edge once, with the provinces on its sides (one for the map's edge).
    edges: Vec<(Pos2, Pos2, ProvinceId, Option<ProvinceId>)>,
    /// Where the last frame painted the map; clicks and tests read it.
    rect: Cell<Rect>,
}

impl MapView {
    pub fn new(polygons: &BTreeMap<ProvinceId, Vec<(i32, i32)>>) -> MapView {
        let shapes: BTreeMap<_, _> = (polygons.iter())
            .map(|(id, pts)| {
                let poly: Vec<Pos2> = pts.iter().map(|&(x, y)| pos2(x as f32, y as f32)).collect();
                let tris = triangulate(&poly);
                (id.clone(), (poly, tris))
            })
            .collect();
        let all: Vec<Pos2> = shapes.values().flat_map(|(p, _)| p.clone()).collect();
        // Neighbouring outlines share their vertices, so a shared edge is the same pair.
        let mut sides: BTreeMap<[(i32, i32); 2], Vec<ProvinceId>> = BTreeMap::new();
        for (id, pts) in polygons {
            for (i, &a) in pts.iter().enumerate() {
                let b = pts[(i + 1) % pts.len()];
                sides
                    .entry([a.min(b), a.max(b)])
                    .or_default()
                    .push(id.clone());
            }
        }
        let at = |(x, y): (i32, i32)| pos2(x as f32, y as f32);
        let edges = (sides.into_iter())
            .map(|([a, b], ids)| (at(a), at(b), ids[0].clone(), ids.get(1).cloned()))
            .collect();
        MapView {
            shapes,
            bounds: Rect::from_points(&all),
            edges,
            rect: Cell::new(Rect::NOTHING),
        }
    }

    fn scale(&self) -> f32 {
        let (r, b) = (self.rect.get(), self.bounds);
        (r.width() / b.width()).min(r.height() / b.height())
    }

    pub fn to_screen(&self, p: Pos2) -> Pos2 {
        self.rect.get().center() + (p - self.bounds.center()) * self.scale()
    }

    pub fn to_map(&self, screen: Pos2) -> Pos2 {
        self.bounds.center() + (screen - self.rect.get().center()) / self.scale()
    }

    pub fn province_at(&self, screen: Pos2) -> Option<&ProvinceId> {
        let p = self.to_map(screen);
        let mut hit = self
            .shapes
            .iter()
            .filter(|(_, (poly, _))| contains(poly, p));
        hit.next().map(|(id, _)| id)
    }

    /// Vertex mean in map coordinates: where the label goes.
    pub fn centre(&self, id: &ProvinceId) -> Option<Pos2> {
        let (poly, _) = self.shapes.get(id)?;
        let sum = poly.iter().fold(vec2(0.0, 0.0), |s, p| s + p.to_vec2());
        Some((sum / poly.len() as f32).to_pos2())
    }

    /// Paints the world into the rest of `ui`; `marked` get a solid outline.
    /// Returns the clicked province.
    pub fn show(
        &self,
        ui: &mut Ui,
        w: &World,
        data: &Data,
        marked: &[ProvinceId],
    ) -> Option<ProvinceId> {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click());
        self.rect.set(resp.rect);
        painter.rect_filled(resp.rect, 0.0, BG2);
        // Weak: the crown reaches too little for any province action.
        let province_actions = data
            .actions
            .iter()
            .filter(|a| matches!(a.target, ActionTarget::Province(_)));
        let reach = province_actions
            .map(|a| a.min_crown_power)
            .min()
            .unwrap_or_default();
        let building: BTreeSet<&str> = (w.active_actions.iter())
            .filter(|a| !built_by(data, &a.id).is_empty())
            .filter_map(|a| a.target.as_deref())
            .collect();

        let war_target = w.war.as_ref().and_then(|x| x.target.as_ref());
        let mut mesh = Mesh::default();
        let (mut base, mut top) = (Vec::new(), Vec::new());
        for (id, (poly, tris)) in &self.shapes {
            let Some(p) = w.provinces.get(id) else {
                continue;
            };
            let own = !matches!(p.holder, Holder::Foreign(_));
            let fill = holder_color(w, &p.holder);
            let fill = match own && p.crown_power < reach {
                true => fill.gamma_multiply(WEAK_ALPHA),
                false => fill,
            };
            let pts: Vec<Pos2> = poly.iter().map(|&q| self.to_screen(q)).collect();
            let first = mesh.vertices.len() as u32;
            pts.iter().for_each(|&q| mesh.colored_vertex(q, fill));
            mesh.indices.extend(tris.iter().map(|i| first + i));
            base.push(Shape::closed_line(pts.clone(), Stroke::new(1.5, BG2)));
            if own && p.loyalty < data.unrest_below {
                top.push(Shape::closed_line(pts.clone(), Stroke::new(2.5, UNREST)));
            }
            if marked.contains(id) {
                top.push(Shape::closed_line(pts.clone(), Stroke::new(2.5, FG)));
            }
            if war_target == Some(id) {
                top.push(Shape::closed_line(pts.clone(), Stroke::new(4.0, RUBRIC)));
            }
            if building.contains(id.0.as_str()) {
                let mut ring = pts;
                ring.push(ring[0]);
                top.extend(Shape::dashed_line(&ring, Stroke::new(2.0, FG), 4.0, 3.0));
            }
        }
        painter.add(mesh);
        painter.extend(base);
        let holder = |id: &ProvinceId| w.provinces.get(id).map(|p| &p.holder);
        for (a, b, one, other) in &self.edges {
            if other.as_ref().is_none_or(|o| holder(o) != holder(one)) {
                let line = [self.to_screen(*a), self.to_screen(*b)];
                painter.line_segment(line, Stroke::new(2.5, BORDER));
            }
        }
        painter.extend(top);
        let size = (6.0 * self.scale()).clamp(10.0, 15.0);
        // Each state's name above the name of its largest province: the middle of a
        // state's land may well lie in another's.
        for n in w.neighbours.values() {
            let theirs = (self.shapes.iter()).filter(|(id, _)| {
                w.provinces
                    .get(*id)
                    .is_some_and(|p| p.holder == Holder::Foreign(n.id.clone()))
            });
            let largest = theirs.max_by(|a, b| area(&a.1.0).total_cmp(&area(&b.1.0)));
            if let Some(c) = largest.and_then(|(id, _)| self.centre(id)) {
                let at = self.to_screen(c) - vec2(0.0, size * 1.2);
                let font = eframe::egui::FontId::proportional(size * 0.9);
                let name = n.name.to_uppercase();
                painter.text(at, eframe::egui::Align2::CENTER_CENTER, name, font, BORDER);
            }
        }
        for (id, p) in &w.provinces {
            let Some(c) = self.centre(id) else { continue };
            // Its buildings in a row under the name, those being built pale (stage 26b).
            let icons = buildings(w, data, id);
            let font = eframe::egui::FontId::proportional(size);
            let left = (icons.len() as f32 - 1.0) * size * 0.6;
            for (k, (b, built)) in icons.iter().enumerate() {
                let at = self.to_screen(c) + vec2(k as f32 * size * 1.2 - left, size * 1.1);
                let color = if *built { FG } else { FG.gamma_multiply(UNDERWAY_ALPHA) };
                let (center, icon) = (eframe::egui::Align2::CENTER_CENTER, b.icon.clone());
                painter.text(at, center, icon, font.clone(), color);
            }
            let color = if matches!(p.holder, Holder::Foreign(_)) {
                FG2
            } else {
                FG
            };
            let font = eframe::egui::FontId::proportional(size);
            let (name, color) = match war_target == Some(id) {
                true => (format!("⚔ {}", p.name), RUBRIC),
                false => (p.name.clone(), color),
            };
            painter.text(
                self.to_screen(c),
                eframe::egui::Align2::CENTER_CENTER,
                name,
                font,
                color,
            );
        }

        let clicked = resp
            .clicked()
            .then(|| resp.interact_pointer_pos())
            .flatten();
        let clicked = clicked.and_then(|pos| self.province_at(pos)).cloned();
        let hovered = resp.hover_pos().and_then(|pos| self.province_at(pos));
        if let Some(p) = hovered.and_then(|id| w.provinces.get(id)) {
            resp.on_hover_ui_at_pointer(|ui| {
                crate::target_tip(ui, w, data, &Target::Province(p.id.clone()));
                if war_target == Some(&p.id) {
                    ui.label(eframe::egui::RichText::new("Цель войны").color(RUBRIC));
                }
            });
        }
        clicked
    }
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
fn built_by<'a>(data: &'a Data, id: &str) -> Vec<&'a str> {
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
        Holder::Foreign(n) => {
            let at = w.neighbours.get(n).map_or(0, |n| n.ordinal as usize);
            FOREIGN[at % FOREIGN.len()]
        }
    }
}

/// Swatches with captions under the map: the crown, every vassal house, every state.
pub fn legend(ui: &mut Ui, w: &World, data: &Data) {
    ui.horizontal_wrapped(|ui| {
        let holders = (w.vassals.keys().map(|v| Holder::Vassal(v.clone())))
            .chain(w.neighbours.keys().map(|n| Holder::Foreign(n.clone())));
        let mut items = vec![
            (CROWN, "корона".to_string()),
            (CROWN.gamma_multiply(WEAK_ALPHA), "корона, слабая".into()),
        ];
        items.extend(holders.map(|h| (holder_color(w, &h), holder_name(w, &h))));
        items.extend([
            (UNREST, "волнения".into()),
            (FG, "стройка (пунктир, бледный значок)".into()),
            (BORDER, "граница владений".into()),
        ]);
        if w.war.is_some() {
            items.push((RUBRIC, "⚔ цель войны".into()));
        }
        for (color, text) in items {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().rect_filled(r, 0.0, color);
            ui.small(text);
        }
        for b in &data.buildings {
            ui.small(format!("{} {}", b.icon, b.name));
        }
    });
}

/// An `Fx` rounded to a whole number for display.
pub fn round(v: bd_core::fx::Fx) -> String {
    format!("{:.0}", v.0 as f64 / 1000.0)
}

/// Shoelace area of an outline.
fn area(p: &[Pos2]) -> f32 {
    let n = p.len();
    let twice: f32 = (0..n)
        .map(|i| p[i].x * p[(i + 1) % n].y - p[(i + 1) % n].x * p[i].y)
        .sum();
    twice.abs() / 2.0
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
/// egui fills only convex paths, and some provinces are concave.
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
}
