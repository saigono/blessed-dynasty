//! The province map: polygons from the map file painted by holder. Floats are display only.

use bd_core::data::Data;
use bd_core::rules::{ActionTarget, Effect};
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
pub const VASSAL: Color32 = Color32::from_rgb(0x7f, 0x98, 0xb8);
pub const FOREIGN: Color32 = Color32::from_rgb(0xb5, 0xb9, 0xc2);
pub const UNREST: Color32 = Color32::from_rgb(0xd9, 0x64, 0x4f);
pub const GOOD: Color32 = Color32::from_rgb(0x3f, 0x8f, 0x5e);
pub const WARN: Color32 = Color32::from_rgb(0xc4, 0x8a, 0x2a);

const WEAK_ALPHA: f32 = 0.45;

pub struct MapView {
    /// Outline in map coordinates and its triangles.
    shapes: BTreeMap<ProvinceId, (Vec<Pos2>, Vec<u32>)>,
    bounds: Rect,
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
        MapView {
            shapes,
            bounds: Rect::from_points(&all),
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

    pub fn province_at(&self, screen: Pos2) -> Option<&ProvinceId> {
        let p = self.bounds.center() + (screen - self.rect.get().center()) / self.scale();
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
        let builds = |id: &str| {
            let def = data.actions.iter().find(|a| a.id == id);
            def.is_some_and(|a| a.on_complete.iter().any(|e| matches!(e, Effect::Build(..))))
        };
        let building: BTreeSet<&str> = (w.active_actions.iter())
            .filter(|a| builds(&a.id))
            .filter_map(|a| a.target.as_deref())
            .collect();

        let mut mesh = Mesh::default();
        let (mut base, mut top) = (Vec::new(), Vec::new());
        for (id, (poly, tris)) in &self.shapes {
            let Some(p) = w.provinces.get(id) else {
                continue;
            };
            let own = !matches!(p.holder, Holder::Foreign(_));
            let fill = match p.holder {
                Holder::Crown => CROWN,
                Holder::Vassal(_) => VASSAL,
                Holder::Foreign(_) => FOREIGN,
            };
            let fill = match own && p.crown_power < reach {
                true => fill.gamma_multiply(WEAK_ALPHA),
                false => fill,
            };
            let pts: Vec<Pos2> = poly.iter().map(|&q| self.to_screen(q)).collect();
            let first = mesh.vertices.len() as u32;
            pts.iter().for_each(|&q| mesh.colored_vertex(q, fill));
            mesh.indices.extend(tris.iter().map(|i| first + i));
            base.push(Shape::closed_line(pts.clone(), Stroke::new(1.5, BG2)));
            if own && p.loyalty < data.crown_power.loyalty_threshold {
                top.push(Shape::closed_line(pts.clone(), Stroke::new(2.5, UNREST)));
            }
            if marked.contains(id) {
                top.push(Shape::closed_line(pts.clone(), Stroke::new(2.5, FG)));
            }
            if building.contains(id.0.as_str()) {
                let mut ring = pts;
                ring.push(ring[0]);
                top.extend(Shape::dashed_line(&ring, Stroke::new(2.0, FG), 4.0, 3.0));
            }
        }
        painter.add(mesh);
        painter.extend(base);
        painter.extend(top);
        let size = (6.0 * self.scale()).clamp(10.0, 15.0);
        for (id, p) in &w.provinces {
            let Some(c) = self.centre(id) else { continue };
            let color = if matches!(p.holder, Holder::Foreign(_)) {
                FG2
            } else {
                FG
            };
            let font = eframe::egui::FontId::proportional(size);
            painter.text(
                self.to_screen(c),
                eframe::egui::Align2::CENTER_CENTER,
                &p.name,
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
                let holder = match &p.holder {
                    Holder::Crown => "корона".to_string(),
                    Holder::Vassal(v) => {
                        format!("вассал {}", w.vassals.get(v).map_or(&v.0, |v| &v.name))
                    }
                    Holder::Foreign(n) => {
                        w.neighbours.get(n).map_or(n.0.clone(), |n| n.name.clone())
                    }
                };
                ui.strong(&p.name);
                ui.label(holder);
                ui.label(format!("Лояльность {}", round(p.loyalty)));
                ui.label(format!("Сила короны {}", round(p.crown_power)));
                ui.label(format!("Доход {}", round(p.income)));
            });
        }
        clicked
    }
}

/// Swatches with captions under the map.
pub fn legend(ui: &mut Ui) {
    ui.horizontal(|ui| {
        let items = [
            (CROWN, "корона"),
            (CROWN.gamma_multiply(WEAK_ALPHA), "корона, слабая"),
            (VASSAL, "вассал"),
            (FOREIGN, "соседи"),
            (UNREST, "волнения"),
            (FG, "стройка пунктиром"),
        ];
        for (color, text) in items {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().rect_filled(r, 0.0, color);
            ui.small(text);
        }
    });
}

/// An `Fx` rounded to a whole number for display.
pub fn round(v: bd_core::fx::Fx) -> String {
    format!("{:.0}", v.0 as f64 / 1000.0)
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

    fn area(p: &[Pos2]) -> f32 {
        let n = p.len();
        (0..n)
            .map(|i| p[i].x * p[(i + 1) % n].y - p[(i + 1) % n].x * p[i].y)
            .sum::<f32>()
            .abs()
            / 2.0
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
}
