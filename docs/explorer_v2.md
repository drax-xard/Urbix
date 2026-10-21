# Urbix Explorer v2 (Tauri)

Status: implemented in `explorer-v2/` (engine 0.21.0).

## Motivation

The v1 explorer (`3d-explorer-sdk/`) is a two-process app: a C HTTP server
(`server/serve.c`) linking `liburbix.a`, plus a browser-based Three.js viewer
opened at `http://localhost:8311`. It has no main menu (seed/draw-distance are
CLI flags only), uses an orbit camera for the exterior, and requires a terminal
plus a browser. It also forces the hand-synced vendored SDK copy to stay in
lockstep with the engine ABI.

Explorer v2 delivers a **standalone first-person experience** in a single
windowed app:

- **Main menu**: seed, zone type, initial coordinates, START button.
- On START the app finds the **nearest region with the desired zone type** to
  the initial coordinates and places the player **at ground level on a street**.
- **FPS controls**: WASD + pointer-lock mouselook everywhere (exterior and
  interiors).
- **ENTER** while looking at a building teleports the player to that
  building's **ground floor** (interior); ESC returns to the street.

## Framework decision

**Tauri 2** (Rust shell + system webview) wrapping the evolved Three.js
viewer, with the backend calling the `urbix` Rust crate **directly as a path
dependency**:

| Alternative | Verdict |
|---|---|
| Keep C server + browser | Not standalone; no menu/FPS; rejected |
| Bevy native rewrite | Rebuilds all rendering from scratch; heavy dep; rejected |
| macroquad / SDL2+OpenGL | Same rewrite cost, loses polished shaders; rejected |
| **Tauri + Three.js reuse** | **Chosen**: ~80% reuse of the polished viewer, one binary, no vendored-SDK sync |

Notes:

- The Three.js frontend from v1 (mass merging, facade shader, paving,
  interior meshes) is carried over and evolved; no npm toolchain — plain ES
  modules with vendored `three.module.js`.
- Pointer Lock is supported in WKWebView (macOS Safari ≥ 10.1) and is
  already triggered from a user gesture in the existing code.
- `serve.c` and the HTTP API are **not** used by v2; v1 remains untouched.

## Layout

An independent crate (not a workspace member of the root `urbix` package, so
root `cargo build/test/clippy` stay light):

```
explorer-v2/
├── src/                    # frontend (evolved from 3d-explorer-sdk/server/www)
│   ├── index.html          # menu overlay + canvas + HUD
│   ├── app.js              # viewer logic: menu, FPS, streaming, interiors
│   ├── style.css
│   └── vendor/             # three.module.js copied from v1
└── src-tauri/
    ├── Cargo.toml          # depends on urbix = { path = "../.." }
    ├── tauri.conf.json     # window, distDir = "../src"
    ├── build.rs            # tauri-build
    ├── icons/              # default Tauri icons
    └── src/
        ├── main.rs         # Tauri app setup, command wiring
        └── engine.rs       # engine state, commands, data structs (serde)
```

## Backend (Tauri commands)

One engine instance behind a `Mutex<State>`; rebuilt when START is pressed
(`WorldEngine::with_config` clears the chunk/interior caches, so a re-seed is
safe).

| Command | Maps to | Returns |
|---|---|---|
| `start_world(seed)` | build a `WorldEngine` for the seed | config snapshot |
| `get_config()` | active/default `WorldConfig` snapshot | zone names/hues, floor height, defaults |
| `find_zone_start(zone, wx, wz)` | nearest matching Voronoi site + bounded ring scan | `{ wx, wz, x, z }` spawn on a walkable cell of the target zone |
| `get_chunks(cx, cy, r)` | `generate_chunk` × (2r+1)², `r ≤ 4` | same JSON shape v1 consumed |
| `get_interior(wx, wz)` | interior grids + metadata | same shape v1 consumed |
| `get_rooms(wx, wz)` | per-room records (`rooms_of_floor`) | room list |
| `get_zone(wx, wz)` | zone affinity query | `{ weights: [f32; 5] }` |

All buffers are parsed in Rust (typed structs, never manual C), serialized
with serde, and freed by the engine's own RAII — no FFI free discipline
leaks into the app.

`find_zone_start` details: order the diagram's matching sites by distance to
`(wx, wz)`, then scan expanding Chebyshev rings around each site (bounded,
deterministic); a candidate cell must (a) have the
target zone as argmax of its affinity weights and (b) be walkable
(`IS_STREET | IS_SIDEWALK | IS_PLAZA | IS_GREENWAY | IS_PARK` flags). The
spawn places the player at cell centre, eye height 1.7 m.

## Frontend (app.js v2)

- **Menu overlay**: seed field (random default), zone dropdown (populated
  from `get_config`), wx/wz inputs (default 0,0), START. START calls
  `start_world` + `find_zone_start`, then fades into the world.
- **FPS controller**: click canvas → pointer lock; WASD + mouselook (yaw
  and pitch, pitch clamped); eye height 1.7 m; gravity snapped to the
  ground plane (y = 0 exterior / storey floor interior); cell-based
  collision — movement blocked into built cells, free on
  street/sidewalk/plaza/greenway/park; Shift sprints. Orbit mode retired.
- **ENTER interaction**: center-screen raycast against building instance
  meshes; on hit, fetch interior + rooms, build the interior mesh (v1
  logic), and place the player at the doorway on the ground floor.
  ESC returns to the street just outside the door. Interior keeps WASD +
  mouselook + R/F storey switching and wall collision.
- **Streaming**: driven by player position, radius 3, batch
  `get_chunks` fetches; chunks outside radius removed/disposed.
- **HUD**: seed/zone/position/mode hint + frame stats (trimmed from v1).

## Build & run

```sh
cd explorer-v2/src-tauri
cargo build --release          # runnable binary in target/release
cargo install tauri-cli        # once, for bundling
cargo tauri build              # produces Urbix.app bundle
```

Frontend has no build step (static files served from `../src` via
`frontendDist`).

## Verification

- `cargo build --release` + `cargo clippy --all-targets` in `src-tauri`.
- Backend unit tests: `find_zone_start` returns a walkable target-zone
  cell across seeds/zones; chunk/interior/rooms round-trip struct
  parsing; config serialization.
- Manual GUI smoke: menu → START → walk → ENTER into a building → ESC →
  back out; v1 explorer untouched.

## Risks

- WKWebView pointer-lock quirks (mitigated: request from user gesture,
  already the v1 pattern).
- Tauri window-close vs engine shutdown: process exit is sufficient; no
  `/api/shutdown` equivalent is needed.
- `ZoneParams`/config surface changes in the engine: v2 reads everything
  through the Rust API and `get_config`, so header drift cannot break it
  (unlike v1's `serve.c`).
