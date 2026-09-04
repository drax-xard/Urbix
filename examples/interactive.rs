//! Interactive streaming explorer — Option A demo.
//!
//! `cargo run --example interactive --release`
//!
//! WASD / drag pans `WorldEngine::set_center` + streaming `generate_chunk`.
//! Wheel zooms, extent slider controls visible grid, mode toggles `hybrid` vs
//! `affinity`, interior dots show `interior_id != 0`. HUD shows
//! `generated_count` / `cache_len` proving bounded LRU.
//!
//! Click any cell to select it: the controls panel then reports the interior
//! mini-world for the selected lot (zone, floors, entrance side, per-storey
//! room/circulation stats, and an ASCII map of the current storey), exactly
//! like `examples/cli_demo.rs` does headlessly.

use eframe::egui;
use urbix::chunk::interior_context_for;
use urbix::config::WorldConfig;
use urbix::data::{Cell, CellFlags, ZONE_COUNT};
use urbix::engine::WorldEngine;
use urbix::interior::generate_layout;
use urbix::layout::{Floor, InteriorLayout, Tile};

/// Fallback hues matching `WorldConfig::default().zone_hues` (promoted to config
/// in Milestone 8). `colour_cell` prefers `WorldConfig::zone_hues` when available.
const ZONE_HUES: [[u8; 3]; ZONE_COUNT] = [
    [100, 150, 220],
    [96, 180, 90],
    [235, 160, 70],
    [150, 130, 115],
    [140, 205, 120],
];
const ROAD_RGB: [u8; 3] = [40, 40, 46];

fn dominant_zone(cell: &Cell) -> usize {
    let mut best = 0;
    let mut best_w = -1.0;
    for (i, w) in cell.zone_affinity.iter().enumerate() {
        if *w > best_w {
            best = i;
            best_w = *w;
        }
    }
    best
}

fn colour_cell(cell: &Cell, mode: &str, config: &WorldConfig) -> [u8; 3] {
    if cell.flags.contains(CellFlags::IS_STREET) {
        return ROAD_RGB;
    }
    let zone = dominant_zone(cell);
    // Prefer config's hues (modular customization); fallback to compiled-in.
    let hue = config
        .zone_hues
        .get(zone)
        .copied()
        .unwrap_or(ZONE_HUES[zone]);
    if mode == "affinity" {
        return hue;
    }
    let params = urbix::zones::zone_params(&cell.zone_affinity);
    let scale = if params.height_max > f32::EPSILON {
        (cell.height / params.height_max).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let to_u8 = |c: u8, t: f32| (f32::from(c) * (1.0 - t) + 255.0 * t) as u8;
    [
        to_u8(hue[0], scale),
        to_u8(hue[1], scale),
        to_u8(hue[2], scale),
    ]
}

struct App {
    engine: WorldEngine,
    center_cx: i32,
    center_cy: i32,
    extent: u32,
    zoom: f32,
    mode: String,
    seed: u64,
    selection: Option<Selection>,
}

/// A lot clicked in the map: its absolute world coordinates and the interior
/// mini-world generated for it (recomputed once on selection).
struct Selection {
    world_x: i64,
    world_z: i64,
    layout: Option<InteriorLayout>,
    /// Storey currently shown in the ASCII map (0 = ground).
    floor: usize,
}

impl App {
    fn new(seed: u64) -> Self {
        let cfg = WorldConfig {
            seed,
            ..Default::default()
        };
        Self {
            engine: WorldEngine::with_config(cfg),
            center_cx: 0,
            center_cy: 0,
            extent: 4,
            zoom: 4.0,
            mode: "hybrid".to_string(),
            seed,
            selection: None,
        }
    }

    /// (Re)select the cell at absolute world coordinates and regenerate its
    /// interior. Streets and bare lots select as "no interior".
    fn select_cell(&mut self, world_x: i64, world_z: i64) {
        let n = i64::from(self.engine.config().chunk_size);
        let cx = world_x.div_euclid(n) as i32;
        let cz = world_z.div_euclid(n) as i32;
        let lx = world_x.rem_euclid(n);
        let lz = world_z.rem_euclid(n);
        let cell = self
            .engine
            .generate_chunk(cx, cz)
            .get_cell((lz * n + lx) as usize);
        let layout = if cell.height > 0.0 {
            let ctx = interior_context_for(self.engine.config(), world_x, world_z, &cell);
            let bp = self.engine.config().blueprint_for(ctx.zone);
            Some(generate_layout(cell.interior_id, &ctx, &bp))
        } else {
            None
        };
        self.selection = Some(Selection {
            world_x,
            world_z,
            layout,
            floor: 0,
        });
    }
}

/// Legend glyph for a tile (shared with `examples/cli_demo.rs` / `viz.rs`).
fn tile_glyph(tile: Tile, kind: u8) -> char {
    match tile {
        Tile::Void => '.',
        Tile::Wall => '#',
        Tile::Door => 'D',
        Tile::Core => '+',
        Tile::Corridor => ' ',
        Tile::Room => char::from(b'a' + kind % 26),
    }
}

/// One floor rendered as rows of legend glyphs.
fn floor_rows(floor: &Floor) -> Vec<String> {
    let w = usize::from(floor.width);
    (0..usize::from(floor.depth))
        .map(|z| {
            let mut row = String::with_capacity(w);
            for x in 0..w {
                let i = z * w + x;
                row.push(tile_glyph(floor.tiles[i], floor.kinds[i]));
            }
            row
        })
        .collect()
}

/// Compact interior summary shown in the side panel: the lot's context, the
/// blueprint that shaped it, and one line of room/circulation stats per storey.
fn selection_summary(sel: &Selection) -> String {
    let mut out = format!("selected cell ({}, {})\n", sel.world_x, sel.world_z);
    let layout = match &sel.layout {
        Some(layout) => layout,
        None => return out + "no building on this cell — no interior",
    };
    let ctx = &layout.context;
    out.push_str(&format!(
        "zone {:?}, footprint {}x{}, {} floor(s), entrance {:?}\n",
        ctx.zone, ctx.footprint_w, ctx.footprint_d, ctx.floor_count, ctx.door_side
    ));
    for (f, floor) in layout.floors.iter().enumerate() {
        let mut rooms = 0;
        let mut corridors = 0;
        let mut walls = 0;
        let mut cores = 0;
        let mut doors = 0;
        for tile in &floor.tiles {
            match tile {
                Tile::Wall => walls += 1,
                Tile::Door => doors += 1,
                Tile::Core => cores += 1,
                Tile::Corridor => corridors += 1,
                Tile::Room => rooms += 1,
                Tile::Void => {}
            }
        }
        out.push_str(&format!(
            "  floor {f}: {rooms} room tiles, {corridors} corridor, {walls} wall, {cores} core, {doors} door\n"
        ));
    }
    out
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keyboard pan.
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::A)) {
            self.center_cx -= 1;
            self.engine.set_center(self.center_cx, self.center_cy);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::D)) {
            self.center_cx += 1;
            self.engine.set_center(self.center_cx, self.center_cy);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::W)) {
            self.center_cy -= 1;
            self.engine.set_center(self.center_cx, self.center_cy);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::S)) {
            self.center_cy += 1;
            self.engine.set_center(self.center_cx, self.center_cy);
        }

        egui::SidePanel::left("controls").show(ctx, |ui| {
            ui.heading("Urbix Explorer");
            ui.label(format!("seed {}", self.seed));
            ui.horizontal(|ui| {
                ui.label("center");
                if ui.button("◀").clicked() {
                    self.center_cx -= 1;
                    self.engine.set_center(self.center_cx, self.center_cy);
                }
                if ui.button("▶").clicked() {
                    self.center_cx += 1;
                    self.engine.set_center(self.center_cx, self.center_cy);
                }
                if ui.button("▲").clicked() {
                    self.center_cy -= 1;
                    self.engine.set_center(self.center_cx, self.center_cy);
                }
                if ui.button("▼").clicked() {
                    self.center_cy += 1;
                    self.engine.set_center(self.center_cx, self.center_cy);
                }
            });
            ui.add(egui::Slider::new(&mut self.extent, 1..=8).text("extent"));
            ui.add(egui::Slider::new(&mut self.zoom, 1.0..=8.0).text("zoom"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.mode, "hybrid".to_string(), "hybrid");
                ui.selectable_value(&mut self.mode, "affinity".to_string(), "affinity");
            });
            ui.separator();
            ui.label(format!(
                "center chunk ({},{})",
                self.center_cx, self.center_cy
            ));
            ui.label(format!("generated {}", self.engine.generated_count()));
            ui.label(format!("cache {}", self.engine.cache_len()));
            ui.label(format!("cache {} bytes", self.engine.cache_memory_bytes()));
            if ui.button("Reset").clicked() {
                self.center_cx = 0;
                self.center_cy = 0;
                self.engine.set_center(0, 0);
            }
            ui.small("WASD/Arrows pan, wheel zooms");
            ui.separator();
            let clear = self.selection.is_some() && ui.button("clear selection ✕").clicked();
            if clear {
                self.selection = None;
            } else if let Some(sel) = &mut self.selection {
                ui.monospace(format!("selected cell ({}, {})", sel.world_x, sel.world_z));
                ui.monospace(selection_summary(sel));
                if let Some(layout) = &sel.layout {
                    if !layout.floors.is_empty() {
                        let max = layout.floors.len() - 1;
                        ui.add(egui::Slider::new(&mut sel.floor, 0..=max).text("storey"));
                        ui.monospace(floor_rows(&layout.floors[sel.floor]).join("\n"));
                    }
                }
            } else {
                ui.small("click a cell to inspect its interior");
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
            // Drag panning (10 px ≈ 1 chunk at zoom 4).
            if response.dragged() {
                let delta = response.drag_delta();
                if delta.x.abs() > 10.0 {
                    self.center_cx -= delta.x.signum() as i32;
                    self.engine.set_center(self.center_cx, self.center_cy);
                }
                if delta.y.abs() > 10.0 {
                    self.center_cy -= delta.y.signum() as i32;
                    self.engine.set_center(self.center_cx, self.center_cy);
                }
            }
            // Scroll zoom.
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom + scroll * 0.01).clamp(1.0, 8.0);
            }

            let painter = ui.painter_at(rect);
            let cs = self.engine.config().chunk_size as i32;
            let half = (self.extent as i32) / 2;
            let start_cx = self.center_cx - half;
            let start_cy = self.center_cy - half;
            let cell_px = self.zoom;
            let origin = rect.min;

            // A click selects the exterior cell under the cursor: convert the
            // pixel offset back to a world coordinate (cell space begins at the
            // top-left chunk's corner).
            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let gx = ((p.x - origin.x) / cell_px).floor() as i64;
                    let gz = ((p.y - origin.y) / cell_px).floor() as i64;
                    let span = i64::from(self.extent) * i64::from(cs);
                    if gx >= 0 && gz >= 0 && gx < span && gz < span {
                        let world_x = i64::from(start_cx) * i64::from(cs) + gx;
                        let world_z = i64::from(start_cy) * i64::from(cs) + gz;
                        self.select_cell(world_x, world_z);
                    }
                }
            }

            for cy in 0..self.extent as i32 {
                for cx in 0..self.extent as i32 {
                    let chunk = self.engine.generate_chunk(start_cx + cx, start_cy + cy);
                    for ly in 0..cs {
                        for lx in 0..cs {
                            let idx = (ly * cs + lx) as usize;
                            let cell = chunk.get_cell(idx);
                            let rgb = colour_cell(&cell, &self.mode, self.engine.config());
                            let x = origin.x + (cx * cs + lx) as f32 * cell_px;
                            let y = origin.y + (cy * cs + ly) as f32 * cell_px;
                            let r = egui::Rect::from_min_size(
                                egui::pos2(x, y),
                                egui::vec2(cell_px, cell_px),
                            );
                            if rect.intersects(r) {
                                let col = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                                painter.rect_filled(r, 0.0, col);
                                if cell.interior_id != 0 {
                                    // Small dot for interior hook.
                                    painter.circle_filled(
                                        r.center(),
                                        cell_px * 0.25,
                                        egui::Color32::WHITE,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            // Selection marker: outline the chosen cell in the same coordinate
            // space as the map (drawn after the cells so it stays on top).
            if let Some(sel) = &self.selection {
                let gx = sel.world_x - i64::from(start_cx) * i64::from(cs);
                let gz = sel.world_z - i64::from(start_cy) * i64::from(cs);
                let span = i64::from(self.extent) * i64::from(cs);
                if gx >= 0 && gz >= 0 && gx < span && gz < span {
                    let r = egui::Rect::from_min_size(
                        egui::pos2(
                            origin.x + gx as f32 * cell_px,
                            origin.y + gz as f32 * cell_px,
                        ),
                        egui::vec2(cell_px, cell_px),
                    );
                    painter.rect_stroke(r, 0.0, egui::Stroke::new(2.0_f32, egui::Color32::YELLOW));
                }
            }
            // Chunk grid overlay.
            for i in 0..=self.extent as i32 {
                let x = origin.x + i as f32 * cs as f32 * cell_px;
                painter.line_segment(
                    [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
                    egui::Stroke::new(1.0_f32, egui::Color32::from_black_alpha(40)),
                );
                let y = origin.y + i as f32 * cs as f32 * cell_px;
                painter.line_segment(
                    [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
                    egui::Stroke::new(1.0_f32, egui::Color32::from_black_alpha(40)),
                );
            }
        });
        ctx.request_repaint();
    }
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Urbix — Interactive Explorer",
        opts,
        Box::new(|_| Box::new(App::new(445566))),
    )
}
