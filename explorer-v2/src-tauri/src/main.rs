//! # main.rs
//!
//! Tauri application entry point for the Urbix Explorer v2.
//!
//! Wires the shared [`EngineState`] into the Tauri runtime and registers the
//! command surface the webview calls. There is no HTTP server and no second
//! process: the frontend (static Three.js) runs in the system webview and
//! talks to the engine through `invoke`, so the whole explorer is one binary.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;

use std::sync::Mutex;

fn main() {
    tauri::Builder::default()
        .manage(Mutex::new(engine::EngineState::default()))
        .invoke_handler(tauri::generate_handler![
            engine::get_config,
            engine::start_world,
            engine::find_zone_start,
            engine::get_chunks,
            engine::get_interior,
            engine::get_rooms,
            engine::get_zone,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Urbix Explorer");
}
