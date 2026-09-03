//! Interactive streaming explorer — Option A demo.
//!
//! `cargo run --example interactive --release`
//!
//! WASD / drag pans `WorldEngine::set_center` + streaming `generate_chunk`.
//! Wheel zooms, extent slider controls visible grid, mode toggles `hybrid` vs
//! `affinity`, interior dots show `interior_id != 0`. HUD shows
//! `generated_count` / `cache_len` proving bounded LRU.

use eframe::egui;
use urbix::config::WorldConfig;
use urbix::data::{Cell, CellFlags, ZONE_COUNT};
use urbix::engine::WorldEngine;

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

fn colour_cell(cell: &Cell, mode: &str) -> [u8; 3] {
    if cell.flags.contains(CellFlags::IS_STREET) {
        return ROAD_RGB;
    }
    let zone = dominant_zone(cell);
    let hue = ZONE_HUES[zone];
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
        }
    }
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

            for cy in 0..self.extent as i32 {
                for cx in 0..self.extent as i32 {
                    let chunk = self.engine.generate_chunk(start_cx + cx, start_cy + cy);
                    for ly in 0..cs {
                        for lx in 0..cs {
                            let idx = (ly * cs + lx) as usize;
                            let cell = chunk.get_cell(idx);
                            let rgb = colour_cell(&cell, &self.mode);
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
