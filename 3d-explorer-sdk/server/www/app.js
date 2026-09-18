/* server/www/app.js
 *
 * Three.js viewer for the Urbix 3D-Explorer SDK.
 *
 * Feeds off the sibling micro-server (serve.c): /api/config for the engine's
 * deterministic settings, /api/chunks (batch) for streamed exterior cells,
 * /api/interior for a selected building's storey grids (tiles + kinds +
 * furniture), and /api/rooms for its per-room records.
 *
 * Controls:
 *   orbit  — drag / scroll (damped); double-click a building (or G/Enter
 *            toward it) to fade into its interior; G or Esc fades back out
 *   inside — WASD/arrows move (walls block), Q/E turn, R/F storey up/down,
 *            mouse-look on click; the view stays level at pedestrian eye height
 *   V      — toggle walkability overlay (dark masses + high-contrast paving)
 *   T      — teleport the orbit target to a world cell (prompt for wx,wz)
 *   stop   — Shift+Esc (after a confirm) asks the server to shut down and
 *            closes the explorer
 */
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";

const TILE = { VOID: 0, WALL: 1, DOOR: 2, CORE: 3, CORRIDOR: 4, ROOM: 5 };
const FLAG = {
  STREET: 1 << 0, PARK: 1 << 1, ARTERIAL: 1 << 2,
  PLAZA: 1 << 3, SIDEWALK: 1 << 4, GREENWAY: 1 << 5,
};
const ROOM_KIND_COLORS = [
  0x7fb3d5, 0xf5b041, 0x82e0aa, 0xaf7ac5, 0xf1948a, 0x85c1e9,
  0xabebc6, 0xf9e79f, 0xd7bde2, 0xfadbd8,
];
/* Furniture codes (FURN_* in urbix.h): colour per fitting family. */
const FURN_COLORS = {
  1: 0x8e5a9e,  /* bed — violet */
  2: 0xb07a3f,  /* table — oak */
  3: 0x9aa3ab,  /* counter — steel */
  4: 0x4f7fa8,  /* desk — blue */
  5: 0x6e5a3a,  /* shelf — dark wood */
  6: 0xdde6ec,  /* bath — porcelain */
};
/* Paved-hierarchy overlay colours (M11/M12 street hierarchy). */
const PAVE_COLORS = {
  arterial: 0x46536a, street: 0x272f3a, sidewalk: 0x707a88,
  plaza: 0xc9a06a, greenway: 0x3f7d4e, park: 0x4a8f5d,
};
const PAVE_WALK_COLORS = {
  arterial: 0x7fb3ff, street: 0x3d6ea8, sidewalk: 0xb9c6d4,
  plaza: 0xffc46b, greenway: 0x4ce07a, park: 0x35d06a,
};
const MAX_FLOORS = 48;      /* cap interior storeys shown (perf guard) */
const TILE_COLORS = {
  [TILE.WALL]: 0x9a9a9a,
  [TILE.CORE]: 0x2f2f2f,
  [TILE.DOOR]: 0xc9a06a,
  [TILE.CORRIDOR]: 0xe6e2da,
};

const canvas = document.getElementById("gl");
const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
renderer.setSize(innerWidth, innerHeight);
renderer.outputColorSpace = THREE.SRGBColorSpace;

/* ---- procedural canvas textures (sky, sun glow) ---- */
function canvasTexture(size, draw) {
  const c = document.createElement("canvas");
  c.width = c.height = size;
  draw(c.getContext("2d"));
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  return t;
}

/* Vertical sky gradient: deep zenith blue -> warm haze at the horizon. */
const skyTex = canvasTexture(8, (g) => {
  const grad = g.createLinearGradient(0, 0, 0, 8);
  grad.addColorStop(0, "#0a2440");
  grad.addColorStop(0.45, "#2d578a");
  grad.addColorStop(0.72, "#7ea3c4");
  grad.addColorStop(1, "#c9d6df");
  g.fillStyle = grad;
  g.fillRect(0, 0, 8, 8);
});

const HAZE = new THREE.Color(0x8aa6bd);   /* fog = horizon haze */
const scene = new THREE.Scene();
scene.background = skyTex;
scene.fog = new THREE.Fog(HAZE, 160, 560);

const camera = new THREE.PerspectiveCamera(60, innerWidth / innerHeight, 0.1, 2000);
camera.position.set(64, 90, 96);

/* Soft cool fill + warm late-afternoon sun for readable silhouettes. */
const hemi = new THREE.HemisphereLight(0xcfe4ff, 0x27344a, 0.75);
const sun = new THREE.DirectionalLight(0xffe3b8, 1.5);
sun.position.set(90, 150, -70);
const fill = new THREE.DirectionalLight(0x9fc4e8, 0.35);
fill.position.set(-90, 40, 80);
scene.add(hemi, sun, fill);

/* Sun glow sprite (adds depth behind the skyline). */
const glowTex = canvasTexture(128, (g) => {
  const grad = g.createRadialGradient(64, 64, 2, 64, 64, 64);
  grad.addColorStop(0, "rgba(255,246,222,0.95)");
  grad.addColorStop(0.22, "rgba(255,214,150,0.30)");
  grad.addColorStop(1, "rgba(255,200,120,0)");
  g.fillStyle = grad;
  g.fillRect(0, 0, 128, 128);
});
const sunGlow = new THREE.Sprite(
  new THREE.SpriteMaterial({ map: glowTex, blending: THREE.AdditiveBlending, depthWrite: false })
);
sunGlow.scale.setScalar(420);
sunGlow.position.copy(sun.position);
scene.add(sunGlow);

/* Shared unit box — one geometry for all instanced buildings (never disposed). */
const UNIT_BOX = new THREE.BoxGeometry(1, 1, 1);

const controls = new OrbitControls(camera, canvas);
controls.target.set(16, 0, 16);
controls.enableDamping = true;
controls.maxPolarAngle = Math.PI / 2.02;
controls.minDistance = 2;
controls.maxDistance = 700;

/* Ground cover: asphalt with a faint block grid (roads read as the darker
   space between building blocks) plus subtle grain. Tiles every 4 world units. */
const groundTex = canvasTexture(64, (g) => {
  g.fillStyle = "#141d27";
  g.fillRect(0, 0, 64, 64);
  g.fillStyle = "#1b2633";
  for (let i = 0; i < 8; i++) {
    g.fillRect(0, i * 8, 64, 1);
    g.fillRect(i * 8, 0, 1, 64);
  }
  for (let i = 0; i < 640; i++) { /* asphalt grain */
    g.fillStyle = `rgba(230,238,248,${0.02 + (i % 3) * 0.015})`;
    g.fillRect((i * 37) % 64, (i * 53) % 64, 1, 1);
  }
});
groundTex.wrapS = groundTex.wrapT = THREE.RepeatWrapping;
const ground = new THREE.Mesh(
  new THREE.PlaneGeometry(6000, 6000),
  new THREE.MeshLambertMaterial({ map: groundTex })
);
ground.material.map.repeat.set(6000 / 4, 6000 / 4);
ground.rotation.x = -Math.PI / 2;
ground.position.y = -0.02;
scene.add(ground);

/* ---- engine config (from the server, deterministic) ---- */
const CFG = {
  chunkSize: 32, zoneHues: [], zoneNames: [], floorHeight: 4.0, seed: 0,
  sdk: "", cellMeters: 4, blockSize: [], arterialEvery: [], drawDistance: 8,
};
let walkMode = false;   /* V toggles the walkability overlay */

function groundKind(flags, h) {
  if (flags & FLAG.ARTERIAL) return "arterial";
  if (flags & FLAG.PLAZA) return "plaza";
  if (flags & FLAG.SIDEWALK) return "sidewalk";
  if (flags & FLAG.GREENWAY) return "greenway";
  if (flags & FLAG.STREET) return "street";
  if ((flags & FLAG.PARK) && h <= 0) return "park";
  return null;   /* built lot or empty lot: no paving overlay */
}
const statusEl = document.getElementById("status");
const seedEl = document.getElementById("seed");
const modeHint = document.getElementById("mode-hint");
const storeyBox = document.getElementById("storey");
const storeySlider = document.getElementById("storey-slider");

async function loadConfig() {
  const r = await fetch("/api/config");
  const cfg = await r.json();
  Object.assign(CFG, {
    seed: cfg.seed,
    chunkSize: cfg.chunk_size || 32,
    zoneHues: cfg.zone_hues,
    zoneNames: cfg.zone_names,
    floorHeight: cfg.floor_height || 4.0,
    sdk: cfg.sdk || "",
    cellMeters: cfg.cell_meters || 4,
    blockSize: cfg.block_size || [],
    arterialEvery: cfg.arterial_every || [],
    drawDistance: cfg.draw_distance || 8,
  });
  seedEl.textContent = `seed ${CFG.seed}${CFG.sdk ? " · sdk " + CFG.sdk : ""}`;
  statusEl.textContent = "streaming chunks…";
}
loadConfig().catch((e) => {
  statusEl.textContent = "config failed: " + e;
});

/* ---- chunk streaming around the camera ---- */
function divEuclid(n, d) {
  const r = n % d;
  return r >= 0 ? Math.floor(n / d) : Math.floor(n / d) - 1;
}

const STREAM_RADIUS = 2;   /* Chebyshev chunk radius */
const chunks = new Map();  /* "cx:cy" -> { group, data[], mesh, cx, cy, dist } */
const inFlight = new Set();
let lastCenter = null;
let pickMeshes = [];

function cellColor(zone, pal, h) {
  if (walkMode) {
    /* Walkability overlay: dark masses so the paved ground reads first. */
    const v = 0.10 + Math.min(h / 900, 0.08);
    return new THREE.Color(v, v * 1.05, v * 1.2);
  }
  const [r, g, b] = CFG.zoneHues[zone] || [150, 150, 150];
  const v = 0.64 + 0.12 * ((pal % 5) / 4) + Math.min(h / 460, 0.13);
  return new THREE.Color(r * v / 255, g * v / 255, b * v / 255);
}

function paveColor(kind) {
  const table = walkMode ? PAVE_WALK_COLORS : PAVE_COLORS;
  return new THREE.Color(table[kind] ?? 0x272f3a);
}

function loadChunk(cx, cy) {
  const key = cx + ":" + cy;
  if (chunks.has(key) || inFlight.has(key)) return;
  inFlight.add(key);
  fetch(`/api/chunk?cx=${cx}&cy=${cy}`)
    .then((r) => r.json())
    .then((c) => {
      inFlight.delete(key);
      spawnChunk(c);
    })
    .catch(() => inFlight.delete(key));
}

/* Batch path: one round-trip for the whole (2r+1)^2 square. Falls back to
 * per-chunk loads when the server predates /api/chunks. */
let batchOk = true;
let fetches = 0;
let lastBatchMs = 0;
function loadSquare(cx, cy, r) {
  if (!batchOk) {
    for (let dy = -r; dy <= r; dy++)
      for (let dx = -r; dx <= r; dx++) loadChunk(cx + dx, cy + dy);
    return;
  }
  const t0 = performance.now();
  fetches++;
  fetch(`/api/chunks?cx=${cx}&cy=${cy}&r=${r}`)
    .then((resp) => {
      if (!resp.ok) throw new Error("no batch endpoint");
      return resp.json();
    })
    .then((d) => {
      lastBatchMs = performance.now() - t0;
      for (const c of d.chunks) spawnChunk(c);
    })
    .catch(() => {
      batchOk = false;   /* old server: fan out once, stay fanned out */
      for (let dy = -r; dy <= r; dy++)
        for (let dx = -r; dx <= r; dx++) loadChunk(cx + dx, cy + dy);
    });
}

function spawnChunk(c) {
  const key = c.cx + ":" + c.cy;
  if (chunks.has(key)) return;

  const built = c.cells.filter((cell) => cell.h > 0);
  const paved = c.cells.filter((cell) => groundKind(cell.flags, cell.h) !== null);
  const group = new THREE.Group();
  const data = [];
  const paveKinds = [];
  let mesh = null;
  let roof = null;
  let pave = null;

  if (built.length > 0) {
    mesh = new THREE.InstancedMesh(
      UNIT_BOX,
      new THREE.MeshLambertMaterial({ color: 0xffffff }),
      built.length
    );
    roof = new THREE.InstancedMesh(
      UNIT_BOX,
      new THREE.MeshLambertMaterial({ color: 0xffffff }),
      built.length
    );
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    roof.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    const m = new THREE.Matrix4();
    const q = new THREE.Quaternion();
    const p = new THREE.Vector3();
    const s = new THREE.Vector3();
    const WHITE = new THREE.Color(0xffffff);
    built.forEach((cell, i) => {
      const h = Math.max(cell.h, 0.05);
      const tint = cellColor(cell.zone, cell.pal, h);
      p.set(cell.x, h / 2, cell.z);
      s.set(1, h, 1);
      mesh.setMatrixAt(i, m.compose(p, q, s));
      mesh.setColorAt(i, tint);
      p.set(cell.x, h + 0.09, cell.z);   /* rooftop crown, ~9cm above the slab */
      s.set(0.92, 0.18, 0.92);
      roof.setMatrixAt(i, m.compose(p, q, s));
      roof.setColorAt(i, tint.clone().lerp(WHITE, 0.55));
      data.push({ wx: cell.x, wz: cell.z, height: h, zone: cell.zone, pal: cell.pal, i });
    });
    mesh.instanceMatrix.needsUpdate = true;
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    roof.instanceMatrix.needsUpdate = true;
    if (roof.instanceColor) roof.instanceColor.needsUpdate = true;
    group.add(mesh, roof);
    pickMeshes.push(mesh, roof);
  }

  if (paved.length > 0) {
    /* Paved-hierarchy overlay: flat quads so arterials read wider/brighter,
     * sidewalks pale, plazas warm, greenways/parks green. Arterials are
     * 2-cell avenues by construction, so two adjacent quads form the width. */
    pave = new THREE.InstancedMesh(
      UNIT_BOX,
      new THREE.MeshLambertMaterial({ color: 0xffffff }),
      paved.length
    );
    const m = new THREE.Matrix4();
    const q = new THREE.Quaternion();
    const p = new THREE.Vector3();
    const s = new THREE.Vector3();
    paved.forEach((cell, i) => {
      const kind = groundKind(cell.flags, cell.h);
      paveKinds.push(kind);
      p.set(cell.x, 0.03, cell.z);
      s.set(1, 0.06, 1);
      pave.setMatrixAt(i, m.compose(p, q, s));
      pave.setColorAt(i, paveColor(kind));
    });
    pave.instanceMatrix.needsUpdate = true;
    if (pave.instanceColor) pave.instanceColor.needsUpdate = true;
    group.add(pave);
  }

  group.userData = { cx: c.cx, cy: c.cy };
  chunks.set(key, { group, data, mesh, roof, pave, paveKinds, cx: c.cx, cy: c.cy });
  scene.add(group);
}

/* Re-tint every loaded chunk in place for the walkability overlay (V). */
function restyleChunks() {
  const WHITE = new THREE.Color(0xffffff);
  for (const entry of chunks.values()) {
    if (entry.mesh) {
      for (let i = 0; i < entry.data.length; i++) {
        const cell = entry.data[i];
        const tint = cellColor(cell.zone, cell.pal, cell.height);
        entry.mesh.setColorAt(i, tint);
        entry.roof.setColorAt(i, tint.clone().lerp(WHITE, 0.55));
      }
      if (entry.mesh.instanceColor) entry.mesh.instanceColor.needsUpdate = true;
      if (entry.roof.instanceColor) entry.roof.instanceColor.needsUpdate = true;
    }
    if (entry.pave) {
      for (let i = 0; i < entry.paveKinds.length; i++)
        entry.pave.setColorAt(i, paveColor(entry.paveKinds[i]));
      if (entry.pave.instanceColor) entry.pave.instanceColor.needsUpdate = true;
    }
  }
}

function centerChunk() {
  return {
    cx: divEuclid(Math.floor(camera.position.x), CFG.chunkSize),
    cy: divEuclid(Math.floor(camera.position.z), CFG.chunkSize),
  };
}

function updateStream() {
  const c = centerChunk();
  const moved = !lastCenter || lastCenter.cx !== c.cx || lastCenter.cy !== c.cy;
  if (!moved) return;
  lastCenter = c;

  loadSquare(c.cx, c.cy, STREAM_RADIUS);

  for (const [key, entry] of chunks) {
    const d = Math.max(Math.abs(entry.cx - c.cx), Math.abs(entry.cy - c.cy));
    if (d > STREAM_RADIUS) {
      scene.remove(entry.group);
      pickMeshes = pickMeshes.filter((m) => m !== entry.mesh && m !== entry.roof);
      for (const em of [entry.mesh, entry.roof, entry.pave])
        if (em && em.material) em.material.dispose();   /* UNIT_BOX geometry is shared, keep it */
      chunks.delete(key);
    }
  }
  refreshStatus();
}

function refreshStatus() {
  const batch = batchOk ? `batch ${lastBatchMs.toFixed(0)}ms` : "single";
  statusEl.textContent =
    `${chunks.size} chunk(s) · ${countBoxes()} boxes · ${frameMs.toFixed(1)}ms/frame · ${batch} · ${fetches} fetches`;
}

function countBoxes() {
  let n = 0;
  for (const e of chunks.values()) n += e.data.length;
  return n;
}

/* ---- interior (double-click a building to go inside) ---- */
let inspecting = null;  /* { wx, wz, zone, floors, w, dep, ox, oz, active } */
const interiorGroup = new THREE.Group();
interiorGroup.visible = false;
scene.add(interiorGroup);
const INTERIOR_BG = new THREE.Color(0x0a0e14);

const popup = document.getElementById("popup");
const fadeEl = document.getElementById("fade");
const shutdownEl = document.getElementById("shutdown");
let transitionBusy = false;
let exitState = null;   /* orbit camera transform, restored on exit */

function fade(alpha) {
  return new Promise((resolve) => {
    fadeEl.classList.toggle("on", alpha === 1);
    setTimeout(resolve, 460);
  });
}
async function withFade(swap) {
  await fade(true);
  swap();
  await fade(false);
}

/* Shift+Esc: tell the server to exit, then close (or cover) the page. */
async function shutdownExplorer() {
  try { await fetch("/api/shutdown"); } catch (_) { /* server may die first */ }
  shutdownEl.classList.remove("hidden");
  try { window.close(); } catch (_) { /* only works for scripted tabs */ }
}

/* Hide/show everything outside the building (exterior boxes + ground). */
function setExteriorVisible(show) {
  for (const entry of chunks.values()) entry.group.visible = show;
  ground.visible = show;
  scene.background = show ? skyTex : INTERIOR_BG;
}

async function inspectCell(wx, wz) {
  const [ir, rr] = await Promise.all([
    fetch(`/api/interior?wx=${wx}&wz=${wz}`),
    fetch(`/api/rooms?wx=${wx}&wz=${wz}`).catch(() => null),
  ]);
  const d = await ir.json();
  if (!d.floor_count || !d.floors.length) {
    popup.textContent = `(${wx}, ${wz}) — no interior here`;
    popup.classList.remove("hidden");
    setTimeout(() => popup.classList.add("hidden"), 2000);
    return;
  }
  let rooms = null;
  try { rooms = rr ? await rr.json() : null; } catch (_) { rooms = null; }
  await enterBuilding(wx, wz, d, rooms);
}

/* Window derivation (mirrors Floor::window_cells in src/layout.rs, applied
 * to all four facades): exterior ring walls minus corners and doors. Pure
 * geometry over the finished floor — no hash, no wire tile. */
function windowCells(tiles, w, dep) {
  const out = [];
  const edge = (x, z) => {
    if (tiles[z * w + x] === TILE.WALL) out.push({ x, z });
  };
  for (let z = 1; z < dep - 1; z++) { edge(0, z); edge(w - 1, z); }
  for (let x = 1; x < w - 1; x++) { edge(x, 0); edge(x, dep - 1); }
  return out;
}

/* Cross-fade into the building: exterior fades out, camera walks to the
   doorway and the interior fades in, so you're never seeing both at once. */
async function enterBuilding(wx, wz, d, rooms) {
  if (transitionBusy || inspecting) return;
  const floors = d.floors.slice(0, MAX_FLOORS);
  const w = d.footprint_w;
  const dep = d.footprint_d;
  const ox = wx - Math.floor(w / 2);
  const oz = wz - Math.floor(dep / 2);
  const door = findDoor(floors, w, dep);
  const pass = firstPassable(floors, w, dep);
  if (!door && !pass) {
    popup.textContent = `(${wx}, ${wz}) — no way into this building (all sealed)`;
    popup.classList.remove("hidden");
    setTimeout(() => popup.classList.add("hidden"), 2500);
    return;
  }

  transitionBusy = true;
  controls.enabled = false;
  clearInspect();
  inspecting = { wx, wz, zone: d.zone, floors, w, dep, ox, oz, active: 0, rooms };
  buildInteriorMesh(d, floors);

  exitState = { pos: camera.position.clone(), quat: camera.quaternion.clone(), target: controls.target.clone() };

  const foot = door ? { x: ox + door.x + 0.5, z: oz + door.z + 0.5 }
            : { x: ox + pass.x + 0.5, z: oz + pass.z + 0.5 };
  const inside = door ? inward(door, dep)
              : { x: Math.sign(wx - ox - pass.x - 0.5), z: Math.sign(wz - oz - pass.z - 0.5) };

  await withFade(() => {
    setExteriorVisible(false);
    interiorGroup.visible = true;
    camera.rotation.order = "YXZ";
    camera.position.set(foot.x + inside.x * 0.7, FLY_EYE, foot.z + inside.z * 0.7);
    fly.yaw = Math.atan2(-inside.x, -inside.z);   /* look into the building */
    fly.anchorY = FLY_EYE;
    setActiveStorey(0, false);
  });

  transitionBusy = false;
  modeHint.textContent = "WASD move · Q/E turn · R/F floor · G/Esc exit · Shift+Esc stop";
  let stats = "";
  if (rooms && rooms.rooms) {
    const units = new Set(rooms.rooms.map((r) => r.unit)).size;
    stats = ` · ${rooms.rooms.length} rooms · ${units} units`;
  }
  const furnCount = floors.reduce((n, f) =>
    n + (f.furn ? f.furn.filter((v) => v !== 0).length : 0), 0);
  popup.textContent =
    `inside (${wx}, ${wz}) · ${CFG.zoneNames[d.zone] || "?"} · ` +
    `${d.floors.length} storeys · ${w}×${dep}${stats} · ${furnCount} furnishings`;
  popup.classList.remove("hidden");
  storeyBox.classList.remove("hidden");
}

function findDoor(floors, w, dep) {
  const g = floors[0].tiles;
  for (let tz = 0; tz < dep; tz++)
    for (let tx = 0; tx < w; tx++)
      if (g[tz * w + tx] === TILE.DOOR && (tx === 0 || tx === w - 1 || tz === 0 || tz === dep - 1))
        return { x: tx, z: tz };
  return null;
}
function firstPassable(floors, w, dep) {
  const g = floors[0].tiles;
  for (let i = 0; i < g.length; i++)
    if (g[i] !== TILE.VOID && g[i] !== TILE.WALL && g[i] !== TILE.CORE)
      return { x: i % w, z: Math.floor(i / w) };
  return null;
}
function inward(door, dep) {
  if (door.z === 0) return { x: 0, z: 1 };
  if (door.z === dep - 1) return { x: 0, z: -1 };
  if (door.x === 0) return { x: 1, z: 0 };
  return { x: -1, z: 0 };
}

function clearInspect() {
  for (const child of [...interiorGroup.children]) {
    if (child.geometry && child.geometry !== UNIT_BOX) child.geometry.dispose();
    if (child.material) child.material.dispose();
    interiorGroup.remove(child);
  }
  inspecting = null;
}

function buildInteriorMesh(d, floors) {
  const fh = CFG.floorHeight;
  const w = d.footprint_w;
  const dep = d.footprint_d;
  const ox = d.wx - Math.floor(w / 2);
  const oz = d.wz - Math.floor(dep / 2);

  /* Opaque per-storey structure: full-height walls & cores, door lintels. */
  const WALL_C = new THREE.Color(0xa9b2bc);
  const CORE_C = new THREE.Color(0x272d36);
  const DOOR_C = new THREE.Color(0xc3a47a);
  const boxList = [];
  const boxColor = [];
  const addBox = (x, z, y, h, c) => { boxList.push([x, z, y, h, c]); };
  for (let f = 0; f < floors.length; f++) {
    const y0 = f * fh;
    for (let tz = 0; tz < dep; tz++) {
      for (let tx = 0; tx < w; tx++) {
        const t = floors[f].tiles[tz * w + tx];
        if (t === TILE.WALL) {
          addBox(ox + tx, oz + tz, y0, fh,
                 WALL_C.clone().multiplyScalar(0.94 + 0.06 * ((tx * 7 + tz * 13) % 5) / 4));
        } else if (t === TILE.CORE) {
          addBox(ox + tx, oz + tz, y0, fh, new THREE.Color(CORE_C));
        } else if (t === TILE.DOOR) {
          /* lintel over the opening — the doorway itself stays passable */
          addBox(ox + tx, oz + tz, y0 + fh * 0.84, fh * 0.16, new THREE.Color(DOOR_C));
        }
      }
    }
  }

  const boxMesh = new THREE.InstancedMesh(
    UNIT_BOX,
    new THREE.MeshLambertMaterial({ color: 0xffffff }),
    boxList.length || 1
  );
  const m = new THREE.Matrix4();
  const q = new THREE.Quaternion();
  const p = new THREE.Vector3();
  const s = new THREE.Vector3(1, 1, 1);
  boxList.forEach(([x, z, y, h, c], i) => {
    p.set(x + 0.5, y + h / 2, z + 0.5);
    s.y = h;
    boxMesh.setMatrixAt(i, m.compose(p, q, s));
    boxMesh.setColorAt(i, c);
  });
  if (boxList.length) {
    boxMesh.instanceMatrix.needsUpdate = true;
    if (boxMesh.instanceColor) boxMesh.instanceColor.needsUpdate = true;
  }

  /* Furniture (M15 parallel furn layer): one fitting per furnished room tile,
     tinted by family. Secondary pieces were density-gated engine-side. */
  const furnList = [];
  for (let f = 0; f < floors.length; f++) {
    const y0 = f * fh;
    const furn = floors[f].furn || [];
    for (let tz = 0; tz < dep; tz++) {
      for (let tx = 0; tx < w; tx++) {
        const code = furn[tz * w + tx] || 0;
        if (code !== 0)
          furnList.push({ x: ox + tx, z: oz + tz, y: y0, c: FURN_COLORS[code] || 0xffffff });
      }
    }
  }
  let furnMesh = null;
  if (furnList.length) {
    furnMesh = new THREE.InstancedMesh(
      UNIT_BOX,
      new THREE.MeshLambertMaterial({ color: 0xffffff }),
      furnList.length
    );
    const m = new THREE.Matrix4();
    const q = new THREE.Quaternion();
    const p = new THREE.Vector3();
    const s = new THREE.Vector3();
    furnList.forEach((it, i) => {
      p.set(it.x + 0.5, it.y + 0.3, it.z + 0.5);   /* low fittings, underfoot-readable */
      s.set(0.55, 0.6, 0.55);
      furnMesh.setMatrixAt(i, m.compose(p, q, s));
      furnMesh.setColorAt(i, new THREE.Color(it.c));
    });
    furnMesh.instanceMatrix.needsUpdate = true;
    if (furnMesh.instanceColor) furnMesh.instanceColor.needsUpdate = true;
  }

  /* Windows (derived, no wire tile): glazing panels on the outer face of
     facade walls, one storey at a time. */
  const winList = [];
  for (let f = 0; f < floors.length; f++) {
    const y0 = f * fh;
    for (const cell of windowCells(floors[f].tiles, w, dep)) {
      let nx = 0, nz = 0, ry = 0;
      if (cell.x === 0) { nx = -0.46; ry = Math.PI / 2; }
      else if (cell.x === w - 1) { nx = 0.46; ry = Math.PI / 2; }
      else if (cell.z === 0) { nz = -0.46; ry = 0; }
      else { nz = 0.46; ry = 0; }
      winList.push({ x: ox + cell.x + 0.5 + nx, z: oz + cell.z + 0.5 + nz, y: y0, ry });
    }
  }
  let winMesh = null;
  if (winList.length) {
    winMesh = new THREE.InstancedMesh(
      UNIT_BOX,
      new THREE.MeshBasicMaterial({ color: 0xffe9b8 }),
      winList.length
    );
    const m = new THREE.Matrix4();
    const p = new THREE.Vector3();
    const s = new THREE.Vector3(0.8, 1.1, 0.06);
    const e = new THREE.Euler();
    winList.forEach((it, i) => {
      p.set(it.x, it.y + fh * 0.55, it.z);
      e.set(0, it.ry, 0);
      winMesh.setMatrixAt(i, m.compose(p, new THREE.Quaternion().setFromEuler(e), s));
    });
    winMesh.instanceMatrix.needsUpdate = true;
  }

  /* Floor + ceiling slabs for walkable tiles (corridor, rooms, doorways). */
  const positions = [];
  const colors = [];
  const addQuad = (x, z, y, color) => {
    positions.push(x, y, z, x + 1, y, z, x + 1, y, z + 1, x, y, z, x + 1, y, z + 1, x, y, z + 1);
    for (let i = 0; i < 6; i++) colors.push(color.r, color.g, color.b);
  };
  const DOORMAT = new THREE.Color(0x4a525c);
  for (let f = 0; f < floors.length; f++) {
    for (let tz = 0; tz < dep; tz++) {
      for (let tx = 0; tx < w; tx++) {
        const i = tz * w + tx;
        const t = floors[f].tiles[i];
        let c = null;
        if (t === TILE.CORRIDOR) c = new THREE.Color(TILE_COLORS[TILE.CORRIDOR]);
        else if (t === TILE.ROOM) c = new THREE.Color(ROOM_KIND_COLORS[(floors[f].kinds[i] || 0) % ROOM_KIND_COLORS.length]);
        else if (t === TILE.DOOR) c = new THREE.Color(DOORMAT);
        if (c) {
          addQuad(ox + tx, oz + tz, f * fh + 0.01, c);        /* slab underfoot */
          addQuad(ox + tx, oz + tz, (f + 1) * fh - 0.02, c);  /* ceiling above  */
        }
      }
    }
  }

  const floorGeom = new THREE.BufferGeometry();
  floorGeom.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  floorGeom.setAttribute("color", new THREE.Float32BufferAttribute(colors, 3));
  const floorMesh = new THREE.Mesh(
    floorGeom,
    new THREE.MeshLambertMaterial({ vertexColors: true, side: THREE.DoubleSide })
  );

  /* Active-storey volume highlight. */
  const frame = new THREE.LineSegments(
    new THREE.EdgesGeometry(new THREE.BoxGeometry(w + 0.05, fh, dep + 0.05)),
    new THREE.LineBasicMaterial({ color: 0x6ea8ff, transparent: true, opacity: 0.9 })
  );
  frame.position.set(d.wx, fh / 2, d.wz);

  interiorGroup.add(boxMesh, floorMesh, frame);
  if (furnMesh) interiorGroup.add(furnMesh);
  if (winMesh) interiorGroup.add(winMesh);
  interiorGroup.userData = { boxMesh, floorMesh, frame, furnMesh, winMesh, fh };

  storeySlider.max = String(floors.length - 1);
  storeySlider.value = "0";
  document.getElementById("storey-n").textContent = "0";
  document.getElementById("storey-total").textContent = String(floors.length);
  storeyBox.classList.remove("hidden");
}

function setActiveStorey(f, moveCamera) {
  if (!inspecting) return;
  const n = inspecting.floors.length;
  inspecting.active = ((f % n) + n) % n;
  storeySlider.value = String(inspecting.active);
  document.getElementById("storey-n").textContent = String(inspecting.active);
  const { frame, fh } = interiorGroup.userData;
  if (frame) frame.position.set(inspecting.wx, inspecting.active * fh + fh / 2, inspecting.wz);
  if (moveCamera) {
    fly.anchorY = inspecting.active * fh + FLY_EYE;   /* glide to that storey's slab */
  }
}

storeySlider.addEventListener("input", () => setActiveStorey(+storeySlider.value, false));

/* ---- picking + inspect/fly mode ---- */
const raycaster = new THREE.Raycaster();
const pointer = new THREE.Vector2();

function pickCell() {
  pointer.set((pointerX / innerWidth) * 2 - 1, -(pointerY / innerHeight) * 2 + 1);
  raycaster.setFromCamera(pointer, camera);
  const hit = raycaster.intersectObjects(pickMeshes, false)[0];
  if (!hit) return;
  const entry = [...chunks.values()].find((e) => e.mesh === hit.object || e.roof === hit.object);
  if (!entry) return;
  const cell = entry.data[hit.instanceId];
  inspectCell(cell.wx, cell.wz);
}

/* ---- collision + enter-by-key ---- */
function activeFloor() {
  return Math.max(0, Math.min(inspecting.floors.length - 1,
    Math.round((camera.position.y - FLY_EYE) / CFG.floorHeight)));
}
function interiorTile(f, x, z) {
  const { w, dep, ox, oz } = inspecting;
  const gx = Math.floor(x - ox);
  const gz = Math.floor(z - oz);
  if (gx < 0 || gx >= w || gz < 0 || gz >= dep) return -1; /* outside footprint = walled */
  return inspecting.floors[f].tiles[gz * w + gx];
}
function walkable(f, px, pz) {
  const r = 0.28;
  for (const [dx, dz] of [[0, 0], [r, 0], [-r, 0], [0, r], [0, -r]]) {
    const t = interiorTile(f, px + dx, pz + dz);
    if (t === -1 || t === TILE.WALL || t === TILE.CORE) return false;
  }
  return true;
}

/* "G" (or Enter) while looking at a building walks you inside. */
async function enterCenterBuilding() {
  if (transitionBusy || inspecting) return;
  pointer.set(0, 0);
  raycaster.setFromCamera(pointer, camera);
  const hit = raycaster.intersectObjects(pickMeshes, false)[0];
  if (!hit) return;
  const entry = [...chunks.values()].find((e) => e.mesh === hit.object || e.roof === hit.object);
  if (!entry) return;
  const cell = entry.data[hit.instanceId];
  inspectCell(cell.wx, cell.wz);
}

/* fly-in controls (while inspecting) — view is always level with the ground
   and floats at a pedestrian eye height unless R/F change storey. */
const FLY_EYE = 1.6;
const fly = { yaw: Math.PI, keys: new Set(), look: false, anchorY: FLY_EYE };
let pointerX = 0;
let pointerY = 0;

canvas.addEventListener("mousemove", (e) => {
  pointerX = e.clientX;
  pointerY = e.clientY;
  if (inspecting && fly.look && document.pointerLockElement === canvas) {
    fly.yaw -= e.movementX * 0.003;   /* horizontal only — see stays level */
  }
});

canvas.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  if (inspecting) {
    if (!fly.look) {
      canvas.requestPointerLock();
      fly.look = true;
      modeHint.textContent = "mouse look · WASD move · Q/E turn · R/F floor · G/Esc exit · Shift+Esc stop";
    }
    return;
  }
  /* Exterior: nothing on mousedown — entry is double-click so orbit drags
     that start on a building never teleport you inside. */
});

canvas.addEventListener("dblclick", (e) => {
  if (e.button !== 0 || inspecting) return;
  pointerX = e.clientX;
  pointerY = e.clientY;
  pickCell();
});

document.addEventListener("pointerlockchange", () => {
  fly.look = document.pointerLockElement === canvas;
  if (!fly.look) return;
  const dir = new THREE.Vector3();
  camera.getWorldDirection(dir);
  fly.yaw = Math.atan2(-dir.x, -dir.z);
});

const MOVE_SPEED = 14;
const TURN_SPEED = 2.2;   /* rad/s for Q/E */
function stepFly(dt) {
  const fwd = new THREE.Vector3();
  camera.getWorldDirection(fwd);
  const flat = new THREE.Vector3(fwd.x, 0, fwd.z).normalize();
  const right = new THREE.Vector3().crossVectors(flat, new THREE.Vector3(0, 1, 0)).normalize();
  const move = new THREE.Vector3();
  if (fly.keys.has("KeyW")) move.add(flat);
  if (fly.keys.has("KeyS")) move.sub(flat);
  if (fly.keys.has("KeyA")) move.sub(right);
  if (fly.keys.has("KeyD")) move.add(right);
  if (fly.keys.has("ArrowUp")) move.add(flat);
  if (fly.keys.has("ArrowDown")) move.sub(flat);
  if (fly.keys.has("ArrowLeft")) move.sub(right);
  if (fly.keys.has("ArrowRight")) move.add(right);
  if (move.lengthSq() > 0) {
    move.normalize().multiplyScalar(MOVE_SPEED * dt);
    if (inspecting) {
      const f = activeFloor();
      const px = camera.position.x + move.x;
      const pz = camera.position.z + move.z;
      if (walkable(f, px, pz)) camera.position.add(move);   /* walls block, no clipping out */
    } else {
      camera.position.add(move);
    }
  }
  if (fly.keys.has("KeyQ")) fly.yaw += TURN_SPEED * dt;
  if (fly.keys.has("KeyE")) fly.yaw -= TURN_SPEED * dt;
  camera.rotation.order = "YXZ";
  camera.rotation.y = fly.yaw;
  camera.rotation.x = 0;   /* forced level with the ground, no pitch/roll */
  camera.rotation.z = 0;
  const dy = fly.anchorY - camera.position.y;   /* pedestrian eye-level anchor */
  camera.position.y += dy * Math.min(1, dt * 6);
}

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && e.shiftKey) {
    if (confirm("Shut down the Urbix server and close the explorer?")) shutdownExplorer();
    return;
  }
  if (e.code === "Escape" && inspecting) { exitBuilding(); return; }
  if (inspecting) {
    if (e.code === "KeyG") { exitBuilding(); return; }
    if (e.code === "KeyR") setActiveStorey(inspecting.active + 1, true);
    if (e.code === "KeyF") setActiveStorey(inspecting.active - 1, true);
    fly.keys.add(e.code);
    return;
  }
  if (e.code === "KeyV") {
    walkMode = !walkMode;
    restyleChunks();
    modeHint.textContent = walkMode
      ? "walk overlay · V toggles · drag orbit · scroll zoom · double-click/G enter · T teleport"
      : "drag orbit · scroll zoom · double-click a building (or G) to enter · V walk overlay · T teleport";
    return;
  }
  if (e.code === "KeyT") {
    const ans = prompt("Teleport orbit target to world cell (wx,wz):", "0,0");
    if (ans) {
      const parts = ans.split(/[,\s]+/).map(Number);
      if (parts.length >= 2 && parts.every(Number.isFinite)) {
        controls.target.set(parts[0], 0, parts[1]);
        camera.position.set(parts[0] + 48, 90, parts[1] + 80);
        lastCenter = null;   /* force a stream refresh on the next frame */
      }
    }
    return;
  }
  if (e.code === "KeyG" || e.code === "Enter") enterCenterBuilding();
});
window.addEventListener("keyup", (e) => fly.keys.delete(e.code));

async function exitBuilding() {
  if (transitionBusy || !inspecting) return;
  transitionBusy = true;
  if (document.pointerLockElement) document.exitPointerLock();
  fly.keys.clear();
  const saved = exitState;
  await withFade(() => {
    interiorGroup.visible = false;
    clearInspect();
    setExteriorVisible(true);
    storeyBox.classList.add("hidden");
    popup.classList.add("hidden");
    controls.enabled = true;
    if (saved) {
      camera.position.copy(saved.pos);
      camera.quaternion.copy(saved.quat);
      controls.target.copy(saved.target);
    }
    controls.update();
  });
  transitionBusy = false;
  modeHint.textContent = walkMode
    ? "walk overlay · V toggles · drag orbit · scroll zoom · double-click/G enter · T teleport"
    : "drag orbit · scroll zoom · double-click a building (or G) to enter · V walk overlay · T teleport";
}

/* ---- main loop ---- */
const clock = new THREE.Clock();
let frameMs = 0;
let statusTick = 0;
renderer.setAnimationLoop(() => {
  const t0 = performance.now();
  const dt = Math.min(clock.getDelta(), 0.05);
  if (CFG.chunkSize && (inspecting || controls.enabled)) updateStream();
  if (inspecting) stepFly(dt);
  else controls.update();
  renderer.render(scene, camera);
  frameMs = frameMs * 0.9 + (performance.now() - t0) * 0.1;
  if (!inspecting && ++statusTick % 60 === 0) refreshStatus();
});

window.addEventListener("resize", () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
});