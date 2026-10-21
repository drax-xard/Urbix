/* explorer-v2/src/app.js
 *
 * First-person 3D explorer for the Urbix procedural city engine.
 *
 * Runs inside the Tauri webview and talks to the Rust backend through
 * `invoke` (window.__TAURI__.core.invoke). It shares the rendering pipeline
 * with the v1 SDK viewer — mass merging, the procedural facade shader, the
 * paving overlay, and the interior mesh builder — but replaces the orbit
 * camera with a first-person controller and adds a main menu.
 *
 * Flow:
 *   menu  — seed / zone / coordinates / START
 *   start — start_world, find the nearest district of the chosen zone,
 *           spawn at ground level on a street
 *   walk  — WASD + pointer-lock mouse look; ENTER enters the building you
 *           are looking at (ground floor); ESC leaves it
 */
import * as THREE from "./vendor/three.module.js";

const invoke = window.__TAURI__.core.invoke;

const TILE = { VOID: 0, WALL: 1, DOOR: 2, CORE: 3, CORRIDOR: 4, ROOM: 5 };
const FLAG = {
  STREET: 1 << 0, PARK: 1 << 1, ARTERIAL: 1 << 2,
  PLAZA: 1 << 3, SIDEWALK: 1 << 4, GREENWAY: 1 << 5,
};
const ROOM_KIND_COLORS = [
  0x7fb3d5, 0xf5b041, 0x82e0aa, 0xaf7ac5, 0xf1948a, 0x85c1e9,
  0xabebc6, 0xf9e79f, 0xd7bde2, 0xfadbd8,
];
const FURN_COLORS = {
  1: 0x8e5a9e, 2: 0xb07a3f, 3: 0x9aa3ab, 4: 0x4f7fa8, 5: 0x6e5a3a, 6: 0xdde6ec,
};
const PAVE_COLORS = {
  arterial: 0x46536a, street: 0x272f3a, sidewalk: 0x707a88,
  plaza: 0xc9a06a, greenway: 0x3f7d4e, park: 0x4a8f5d,
};
const MAX_FLOORS = 48;
const TILE_COLORS = {
  [TILE.WALL]: 0x9a9a9a, [TILE.CORE]: 0x2f2f2f,
  [TILE.DOOR]: 0xc9a06a, [TILE.CORRIDOR]: 0xe6e2da,
};

/* ---- renderer ---- */
const canvas = document.getElementById("gl");
const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
renderer.setSize(innerWidth, innerHeight);
renderer.outputColorSpace = THREE.SRGBColorSpace;

function canvasTexture(size, draw) {
  const c = document.createElement("canvas");
  c.width = c.height = size;
  draw(c.getContext("2d"));
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  return t;
}

const skyTex = canvasTexture(8, (g) => {
  const grad = g.createLinearGradient(0, 0, 0, 8);
  grad.addColorStop(0, "#0a2440");
  grad.addColorStop(0.45, "#2d578a");
  grad.addColorStop(0.72, "#7ea3c4");
  grad.addColorStop(1, "#c9d6df");
  g.fillStyle = grad;
  g.fillRect(0, 0, 8, 8);
});

const HAZE = new THREE.Color(0x8aa6bd);
const scene = new THREE.Scene();
scene.background = skyTex;
scene.fog = new THREE.Fog(HAZE, 160, 560);

/* Near plane sits just ahead of the eye so walls never clip when up close. */
const camera = new THREE.PerspectiveCamera(72, innerWidth / innerHeight, 0.08, 1200);
camera.rotation.order = "YXZ";

const hemi = new THREE.HemisphereLight(0xcfe4ff, 0x27344a, 0.75);
const sun = new THREE.DirectionalLight(0xffe3b8, 1.5);
sun.position.set(90, 150, -70);
const fill = new THREE.DirectionalLight(0x9fc4e8, 0.35);
fill.position.set(-90, 40, 80);
scene.add(hemi, sun, fill);

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

const UNIT_BOX = new THREE.BoxGeometry(1, 1, 1);

/* Facade windows painted in the fragment shader (see v1 for the full story):
 * columns per cell (1 unit), rows per storey (4 units), bare top metre, and
 * deterministic sparse lit rooms; fwidth() LOD fade removes far moiré. */
function facadeMaterial() {
  const mat = new THREE.MeshLambertMaterial({ color: 0xffffff });
  mat.extensions = { derivatives: true };
  mat.customProgramCacheKey = () => "urbix-facade-v1";
  mat.onBeforeCompile = (shader) => {
    const varyings = `
      varying vec2 vFacadeUv;
      varying vec3 vFacadeScale;
      varying vec3 vFacadeNormal;
      varying vec3 vFacadeOrigin;`;
    shader.vertexShader = shader.vertexShader
      .replace("#include <common>", `#include <common>${varyings}`)
      .replace(
        "#include <begin_vertex>",
        `#include <begin_vertex>
        vFacadeUv = uv;
        vFacadeNormal = normal;
        #ifdef USE_INSTANCING
          vFacadeScale = vec3(
            length(instanceMatrix[0].xyz),
            length(instanceMatrix[1].xyz),
            length(instanceMatrix[2].xyz));
          vFacadeOrigin = (modelMatrix * instanceMatrix[3]).xyz;
        #else
          vFacadeScale = vec3(1.0);
          vFacadeOrigin = vec3(0.0);
        #endif`
      );
    shader.fragmentShader = shader.fragmentShader
      .replace("#include <common>", `#include <common>${varyings}`)
      .replace(
        "#include <color_fragment>",
        `#include <color_fragment>
        if (abs(vFacadeNormal.y) < 0.5) {
          float horiz = (abs(vFacadeNormal.x) > 0.5) ? vFacadeScale.z : vFacadeScale.x;
          float vert = vFacadeScale.y;
          float tall = smoothstep(6.0, 8.0, vert);
          if (tall > 0.001) {
            float u = vFacadeUv.x * horiz;
            float v = vFacadeUv.y * vert;
            float lodFade = 1.0
              - smoothstep(0.25, 0.75, max(fwidth(u), fwidth(v) * 0.25));
            float cu = floor(u);
            float rv = floor(v / 4.0);
            vec2 f = vec2(fract(u), fract(v / 4.0));
            float wx = smoothstep(0.22, 0.28, f.x)
              * (1.0 - smoothstep(0.72, 0.78, f.x));
            float wy = smoothstep(0.42, 0.47, f.y)
              * (1.0 - smoothstep(0.86, 0.91, f.y));
            float top = 1.0 - smoothstep(vert - 1.0, vert - 0.5, v);
            float win = wx * wy * top * tall * lodFade;
            if (win > 0.001) {
              float lit = abs(fract(sin(dot(
                vec3(vFacadeOrigin.x + cu, vFacadeOrigin.z, rv),
                vec3(12.9898, 78.233, 37.719))) * 43758.5453));
              float isLit = (lit < 0.22) ? 1.0 : 0.0;
              vec3 wc = mix(vec3(0.018, 0.031, 0.048),
                            vec3(1.0, 0.789, 0.352), isLit);
              diffuseColor.rgb = mix(diffuseColor.rgb, wc, win);
              totalEmissiveRadiance += vec3(1.0, 0.789, 0.352) * (win * isLit) * 0.5;
            }
          }
        }`
      );
  };
  return mat;
}

/* Ground cover: asphalt with a faint block grid + grain, tiles every 4 units. */
const groundTex = canvasTexture(64, (g) => {
  g.fillStyle = "#141d27";
  g.fillRect(0, 0, 64, 64);
  g.fillStyle = "#1b2633";
  for (let i = 0; i < 8; i++) {
    g.fillRect(0, i * 8, 64, 1);
    g.fillRect(i * 8, 0, 1, 64);
  }
  for (let i = 0; i < 640; i++) {
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
ground.visible = false; // shown once the world starts
scene.add(ground);

/* ---- engine config (from the backend) ---- */
const CFG = {
  chunkSize: 32, zoneHues: [], zoneNames: [], floorHeight: 4.0, seed: 0,
  sdk: "", cellMeters: 4, eyeHeight: 1.7, drawDistance: 8,
};
let started = false;

function groundKind(flags, h) {
  if (flags & FLAG.ARTERIAL) return "arterial";
  if (flags & FLAG.PLAZA) return "plaza";
  if (flags & FLAG.SIDEWALK) return "sidewalk";
  if (flags & FLAG.GREENWAY) return "greenway";
  if (flags & FLAG.STREET) return "street";
  if ((flags & FLAG.PARK) && h <= 0) return "park";
  return null;
}

const menuEl = document.getElementById("menu");
const menuForm = document.getElementById("menu-form");
const menuStatus = document.getElementById("menu-status");
const startBtn = document.getElementById("start-btn");
const seedInput = document.getElementById("seed-input");
const zoneInput = document.getElementById("zone-input");
const wxInput = document.getElementById("wx-input");
const wzInput = document.getElementById("wz-input");
const hudEl = document.getElementById("hud");
const statusEl = document.getElementById("status");
const seedLabel = document.getElementById("seed-label");
const modeHint = document.getElementById("mode-hint");
const crosshair = document.getElementById("crosshair");
const popup = document.getElementById("popup");
const storeyBox = document.getElementById("storey");
const storeySlider = document.getElementById("storey-slider");
const fadeEl = document.getElementById("fade");

/* ---- chunk streaming around the player ---- */
function divEuclid(n, d) {
  const r = n % d;
  return r >= 0 ? Math.floor(n / d) : Math.floor(n / d) - 1;
}

const STREAM_RADIUS = 3;
const chunks = new Map();
const pickMeshes = [];

function cellColor(zone, pal, h) {
  const hue = CFG.zoneHues[zone] || [150, 150, 150];
  const v = 0.64 + 0.12 * ((pal % 5) / 4) + Math.min(h / 460, 0.13);
  return new THREE.Color(hue[0] * v / 255, hue[1] * v / 255, hue[2] * v / 255);
}

function paveColor(kind) {
  return new THREE.Color(PAVE_COLORS[kind] ?? 0x272f3a);
}

/* Batch fetch: one invoke for the whole (2r+1)^2 square around (cx,cy). */
let fetches = 0;
let lastBatchMs = 0;
async function loadSquare(cx, cy, r) {
  const t0 = performance.now();
  fetches++;
  let list;
  try {
    list = await invoke("get_chunks", { cx, cy, r });
  } catch (e) {
    statusEl.textContent = "chunk fetch failed: " + e;
    return;
  }
  lastBatchMs = performance.now() - t0;
  for (const c of list) spawnChunk(c);
}

/* Merge adjacent built cells into building masses (see v1 for rationale). */
const MASS_NEIGHBORS = [[1, 0], [-1, 0], [0, 1], [0, -1]];
function mergeMasses(cells) {
  const byKey = new Map();
  for (const cell of cells) byKey.set(cell.x + "," + cell.z, cell);
  const masses = [];
  for (const seed of cells) {
    const sk = seed.x + "," + seed.z;
    if (!byKey.has(sk)) continue;
    byKey.delete(sk);
    let x0 = seed.x, x1 = seed.x, z0 = seed.z, z1 = seed.z, h = seed.h;
    const stack = [seed];
    while (stack.length) {
      const cur = stack.pop();
      x0 = Math.min(x0, cur.x); x1 = Math.max(x1, cur.x);
      z0 = Math.min(z0, cur.z); z1 = Math.max(z1, cur.z);
      h = Math.max(h, cur.h);
      const tol = Math.max(2, h * 0.25);
      for (const [dx, dz] of MASS_NEIGHBORS) {
        const k = (cur.x + dx) + "," + (cur.z + dz);
        const n = byKey.get(k);
        if (!n || n.zone !== seed.zone || n.pal !== seed.pal) continue;
        if (Math.abs(n.h - h) > tol) continue;
        byKey.delete(k);
        stack.push(n);
      }
    }
    masses.push({ x0, z0, x1, z1, h, zone: seed.zone, pal: seed.pal });
  }
  return masses;
}

function spawnChunk(c) {
  const key = c.cx + ":" + c.cy;
  if (chunks.has(key)) return;

  const built = c.cells.filter((cell) => cell.h > 0);
  const masses = mergeMasses(built);
  const paved = c.cells.filter((cell) => groundKind(cell.flags, cell.h) !== null);
  const group = new THREE.Group();
  const data = [];
  let mesh = null, roof = null, pave = null;

  if (masses.length > 0) {
    mesh = new THREE.InstancedMesh(UNIT_BOX, facadeMaterial(), masses.length);
    roof = new THREE.InstancedMesh(
      UNIT_BOX, new THREE.MeshLambertMaterial({ color: 0xffffff }), masses.length
    );
    const m = new THREE.Matrix4(), q = new THREE.Quaternion();
    const p = new THREE.Vector3(), s = new THREE.Vector3();
    const WHITE = new THREE.Color(0xffffff);
    masses.forEach((mass, i) => {
      const h = Math.max(mass.h, 0.05);
      const w = mass.x1 - mass.x0 + 1;
      const d = mass.z1 - mass.z0 + 1;
      const tint = cellColor(mass.zone, mass.pal, h);
      p.set(mass.x0 + w / 2, h / 2, mass.z0 + d / 2);
      s.set(w, h, d);
      mesh.setMatrixAt(i, m.compose(p, q, s));
      mesh.setColorAt(i, tint);
      p.set(mass.x0 + w / 2, h + 0.09, mass.z0 + d / 2);
      s.set(Math.max(w - 0.08, 0.2), 0.18, Math.max(d - 0.08, 0.2));
      roof.setMatrixAt(i, m.compose(p, q, s));
      roof.setColorAt(i, tint.clone().lerp(WHITE, 0.55));
      data.push({
        wx: mass.x0 + Math.floor(w / 2), wz: mass.z0 + Math.floor(d / 2),
        zone: mass.zone, height: h, i, x0: mass.x0, z0: mass.z0,
        x1: mass.x1, z1: mass.z1,
      });
    });
    mesh.instanceMatrix.needsUpdate = true;
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true;
    roof.instanceMatrix.needsUpdate = true;
    if (roof.instanceColor) roof.instanceColor.needsUpdate = true;
    group.add(mesh, roof);
    pickMeshes.push(mesh, roof);
  }

  if (paved.length > 0) {
    pave = new THREE.InstancedMesh(
      UNIT_BOX, new THREE.MeshLambertMaterial({ color: 0xffffff }), paved.length
    );
    const m = new THREE.Matrix4(), q = new THREE.Quaternion();
    const p = new THREE.Vector3(), s = new THREE.Vector3();
    paved.forEach((cell, i) => {
      p.set(cell.x, 0.03, cell.z);
      s.set(1, 0.06, 1);
      pave.setMatrixAt(i, m.compose(p, q, s));
      pave.setColorAt(i, paveColor(groundKind(cell.flags, cell.h)));
    });
    pave.instanceMatrix.needsUpdate = true;
    if (pave.instanceColor) pave.instanceColor.needsUpdate = true;
    group.add(pave);
  }

  // Index cells by local coords so collision can look them up cheaply.
  const n = CFG.chunkSize;
  const grid = new Array(n * n).fill(null);
  for (const cell of c.cells) {
    const lx = ((cell.x % n) + n) % n;
    const lz = ((cell.z % n) + n) % n;
    grid[lz * n + lx] = cell;
  }

  chunks.set(key, { group, data, grid, mesh, roof, pave, cx: c.cx, cy: c.cy });
  scene.add(group);
}

function centerChunk() {
  return {
    cx: divEuclid(Math.floor(camera.position.x), CFG.chunkSize),
    cy: divEuclid(Math.floor(camera.position.z), CFG.chunkSize),
  };
}

let lastCenter = null;
function updateStream() {
  const c = centerChunk();
  if (lastCenter && lastCenter.cx === c.cx && lastCenter.cy === c.cy) return;
  lastCenter = c;

  loadSquare(c.cx, c.cy, STREAM_RADIUS);

  for (const [key, entry] of chunks) {
    const d = Math.max(Math.abs(entry.cx - c.cx), Math.abs(entry.cy - c.cy));
    if (d > STREAM_RADIUS) {
      scene.remove(entry.group);
      for (const em of [entry.mesh, entry.roof, entry.pave]) {
        if (!em) continue;
        const idx = pickMeshes.indexOf(em);
        if (idx >= 0) pickMeshes.splice(idx, 1);
        if (em.material) em.material.dispose();
      }
      chunks.delete(key);
    }
  }
  refreshStatus();
}

function refreshStatus() {
  if (!started) return;
  const x = camera.position.x.toFixed(1);
  const z = camera.position.z.toFixed(1);
  statusEl.textContent =
    `${mode === "inside" ? "interior" : "street"} · (${x}, ${z}) · ` +
    `${chunks.size} chunk(s) · ${frameMs.toFixed(1)}ms/frame · ${fetches} fetches`;
}

/* ---- first-person controller ---- */
const PLAYER = {
  pos: new THREE.Vector3(0, 1.7, 0),  // eye position
  vel: new THREE.Vector3(),
  yaw: 0,
  pitch: 0,
  keys: new Set(),
  locked: false,
  anchorY: 1.7,                        // ground/floor eye height target
};
const MOVE_SPEED = 11;
const RUN_SPEED = 20;
const MOUSE_SENS = 0.0022;
const GRAVITY = 26;

let mode = "menu";                    // menu | street | inside
let inspecting = null;
const interiorGroup = new THREE.Group();
interiorGroup.visible = false;
scene.add(interiorGroup);
const INTERIOR_BG = new THREE.Color(0x0a0e14);
const interior = { w: 0, dep: 0, ox: 0, oz: 0, wx: 0, wz: 0, floors: [], active: 0 };
let transitionBusy = false;

/* Collision: exterior blocks built cells; interior uses the tile grid. */
function blockedAt(x, z) {
  if (mode === "inside") return !walkableInterior(interior.active, x, z);
  const cx = Math.floor(x), cz = Math.floor(z);
  const key = divEuclid(cx, CFG.chunkSize) + ":" + divEuclid(cz, CFG.chunkSize);
  const entry = chunks.get(key);
  if (!entry) return false; // unknown chunk: don't trap the player
  const n = CFG.chunkSize;
  const lx = ((cx % n) + n) % n;
  const lz = ((cz % n) + n) % n;
  const cell = entry.grid ? entry.grid[lz * n + lx] : null;
  if (!cell) return false;
  return cell.h > 0; // built cells are solid at street level
}
/* A 4-sample radius probe so the player can't clip through corners. */
function tryMove(dx, dz) {
  const r = 0.32;
  const px = PLAYER.pos.x, pz = PLAYER.pos.z;
  if (!blockedAt(px + dx + Math.sign(dx) * r, pz + dz)) {
    if (!blockedAt(px + dx, pz + dz + Math.sign(dz) * r)) {
      PLAYER.pos.x += dx;
      PLAYER.pos.z += dz;
    }
  }
}

function stepPlayer(dt) {
  const sinY = Math.sin(PLAYER.yaw), cosY = Math.cos(PLAYER.yaw);
  const fwd = new THREE.Vector3(-sinY, 0, -cosY);
  const right = new THREE.Vector3(cosY, 0, -sinY);
  const move = new THREE.Vector3();
  if (PLAYER.keys.has("KeyW") || PLAYER.keys.has("ArrowUp")) move.add(fwd);
  if (PLAYER.keys.has("KeyS") || PLAYER.keys.has("ArrowDown")) move.sub(fwd);
  if (PLAYER.keys.has("KeyD") || PLAYER.keys.has("ArrowRight")) move.add(right);
  if (PLAYER.keys.has("KeyA") || PLAYER.keys.has("ArrowLeft")) move.sub(right);
  if (PLAYER.keys.has("KeyQ")) PLAYER.yaw += 2.2 * dt;
  if (PLAYER.keys.has("KeyE")) PLAYER.yaw -= 2.2 * dt;

  if (move.lengthSq() > 0) {
    move.normalize();
    const speed = PLAYER.keys.has("ShiftLeft") || PLAYER.keys.has("ShiftRight")
      ? RUN_SPEED : MOVE_SPEED;
    const step = speed * dt;
    tryMove(move.x * step, move.z * step);
  }

  // Vertical: ease toward the current floor/ground eye height.
  const dy = PLAYER.anchorY - PLAYER.pos.y;
  PLAYER.pos.y += dy * Math.min(1, dt * 8);

  camera.position.copy(PLAYER.pos);
  camera.rotation.set(PLAYER.pitch, PLAYER.yaw, 0, "YXZ");
}

/* ---- pointer lock ---- */
canvas.addEventListener("mousedown", (e) => {
  if (mode === "menu") return;
  if (e.button !== 0) return;
  if (!PLAYER.locked) canvas.requestPointerLock();
});
document.addEventListener("pointerlockchange", () => {
  PLAYER.locked = document.pointerLockElement === canvas;
});
document.addEventListener("mousemove", (e) => {
  if (!PLAYER.locked) return;
  PLAYER.yaw -= e.movementX * MOUSE_SENS;
  PLAYER.pitch -= e.movementY * MOUSE_SENS;
  const lim = Math.PI / 2 - 0.02;
  PLAYER.pitch = Math.max(-lim, Math.min(lim, PLAYER.pitch));
});

/* ---- interaction: ENTER enters the building under the crosshair ---- */
const raycaster = new THREE.Raycaster();
const CENTER = new THREE.Vector2(0, 0);

function centerBuilding() {
  raycaster.setFromCamera(CENTER, camera);
  const hits = raycaster.intersectObjects(pickMeshes, false);
  for (const hit of hits) {
    const entry = [...chunks.values()].find(
      (e) => e.mesh === hit.object || e.roof === hit.object
    );
    if (!entry) continue;
    const cell = entry.data[hit.instanceId];
    if (cell) return cell;
  }
  return null;
}

async function enterBuilding(cellData) {
  if (transitionBusy || mode === "inside") return;
  transitionBusy = true;
  let d, rooms = null;
  try {
    d = await invoke("get_interior", { wx: cellData.wx, wz: cellData.wz });
    if (d && d.floor_count) {
      rooms = await invoke("get_rooms", { wx: cellData.wx, wz: cellData.wz }).catch(() => null);
    }
  } catch (e) {
    showPopup("interior lookup failed: " + e, 2500);
    transitionBusy = false;
    return;
  }
  if (!d || !d.floor_count || !d.floors.length) {
    showPopup(`(${cellData.wx}, ${cellData.wz}) — no way inside`, 1800);
    transitionBusy = false;
    return;
  }

  const floors = d.floors.slice(0, MAX_FLOORS);
  const w = d.footprint_w, dep = d.footprint_d;
  const ox = cellData.wx - Math.floor(w / 2);
  const oz = cellData.wz - Math.floor(dep / 2);
  const door = findDoor(floors, w, dep);
  const pass = door || firstPassable(floors, w, dep);
  if (!pass) {
    showPopup(`(${cellData.wx}, ${cellData.wz}) — sealed`, 1800);
    transitionBusy = false;
    return;
  }

  if (document.pointerLockElement) document.exitPointerLock();
  buildInteriorMesh(d, floors);

  const inward = door ? doorInward(door, dep)
    : { x: Math.sign(cellData.wx - ox - pass.x - 0.5) || 1,
        z: Math.sign(cellData.wz - oz - pass.z - 0.5) || 1 };
  const footX = ox + pass.x + 0.5;
  const footZ = oz + pass.z + 0.5;

  await fade(true);
  setExteriorVisible(false);
  interiorGroup.visible = true;
  mode = "inside";
  interior.active = 0;
  PLAYER.pos.set(footX + inward.x * 0.6, CFG.floorHeight * 0 + 1.6, footZ + inward.z * 0.6);
  PLAYER.anchorY = 1.6;
  PLAYER.yaw = Math.atan2(-inward.x, -inward.z);
  PLAYER.pitch = 0;
  setActiveStorey(0, true);
  crosshair.classList.remove("hidden");
  storeyBox.classList.remove("hidden");
  refreshModeHint();
  if (rooms && rooms.length) {
    const units = new Set(rooms.map((r) => r.unit)).size;
    showPopup(
      `inside (${cellData.wx}, ${cellData.wz}) · ${CFG.zoneNames[d.zone] || "?"} · ` +
      `${d.floor_count} storeys · ${rooms.length} rooms · ${units} units`, 3200
    );
  }
  await fade(false);
  transitionBusy = false;
}

async function exitBuilding() {
  if (transitionBusy || mode !== "inside") return;
  transitionBusy = true;
  if (document.pointerLockElement) document.exitPointerLock();
  PLAYER.keys.clear();
  await fade(true);
  interiorGroup.visible = false;
  clearInterior();
  setExteriorVisible(true);
  mode = "street";
  storeyBox.classList.add("hidden");
  // Nudge the player to the nearest street cell just outside the lot so they
  // don't spawn inside the mass they entered.
  PLAYER.anchorY = 1.7;
  spawnOutside();
  refreshModeHint();
  await fade(false);
  transitionBusy = false;
}

/* Place the player on a walkable cell next to the last building's footprint. */
function spawnOutside() {
  const { wx, wz, w, dep, ox, oz } = interior;
  const candidates = [];
  for (let x = ox - 1; x <= ox + w; x++) {
    candidates.push([x, oz - 1], [x, oz + dep]);
  }
  for (let z = oz - 1; z <= oz + dep; z++) {
    candidates.push([ox - 1, z], [ox + w, z]);
  }
  const mid = { x: wx, z: wz };
  candidates.sort((a, b) =>
    (Math.abs(a[0] - mid.x) + Math.abs(a[1] - mid.z)) -
    (Math.abs(b[0] - mid.x) + Math.abs(b[1] - mid.z)));
  for (const [cx, cz] of candidates) {
    if (!blockedAt(cx + 0.5, cz + 0.5)) {
      PLAYER.pos.set(cx + 0.5, 1.7, cz + 0.5);
      return;
    }
  }
  PLAYER.pos.set(ox + w / 2, 1.7, oz - 2);
}

function findDoor(floors, w, dep) {
  const g = floors[0].tiles;
  for (let tz = 0; tz < dep; tz++)
    for (let tx = 0; tx < w; tx++)
      if (g[tz * w + tx] === TILE.DOOR &&
          (tx === 0 || tx === w - 1 || tz === 0 || tz === dep - 1))
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
function doorInward(door, dep) {
  if (door.z === 0) return { x: 0, z: 1 };
  if (door.z === dep - 1) return { x: 0, z: -1 };
  if (door.x === 0) return { x: 1, z: 0 };
  return { x: -1, z: 0 };
}

/* Interior collision (radius probe over the tile grid). */
function walkableInterior(f, x, z) {
  const r = 0.28;
  for (const [dx, dz] of [[0, 0], [r, 0], [-r, 0], [0, r], [0, -r]]) {
    const t = interiorTile(f, x + dx, z + dz);
    if (t === -1 || t === TILE.WALL || t === TILE.CORE) return false;
  }
  return true;
}
function interiorTile(f, x, z) {
  const gx = Math.floor(x - interior.ox);
  const gz = Math.floor(z - interior.oz);
  if (gx < 0 || gx >= interior.w || gz < 0 || gz >= interior.dep) return -1;
  const floor = interior.floors[f];
  if (!floor) return -1;
  return floor.tiles[gz * interior.w + gx];
}

function windowCells(tiles, w, dep) {
  const out = [];
  const edge = (x, z) => { if (tiles[z * w + x] === TILE.WALL) out.push({ x, z }); };
  for (let z = 1; z < dep - 1; z++) { edge(0, z); edge(w - 1, z); }
  for (let x = 1; x < w - 1; x++) { edge(x, 0); edge(x, dep - 1); }
  return out;
}

function clearInterior() {
  for (const child of [...interiorGroup.children]) {
    if (child.geometry && child.geometry !== UNIT_BOX) child.geometry.dispose();
    if (child.material) child.material.dispose();
    interiorGroup.remove(child);
  }
  inspecting = null;
}

function buildInteriorMesh(d, floors) {
  clearInterior();
  const fh = CFG.floorHeight;
  const w = d.footprint_w, dep = d.footprint_d;
  const ox = d.wx - Math.floor(w / 2);
  const oz = d.wz - Math.floor(dep / 2);
  interior.w = w; interior.dep = dep; interior.ox = ox; interior.oz = oz;
  interior.wx = d.wx; interior.wz = d.wz; interior.floors = floors;
  inspecting = interior;

  const WALL_C = new THREE.Color(0xa9b2bc);
  const CORE_C = new THREE.Color(0x272d36);
  const DOOR_C = new THREE.Color(0xc3a47a);
  const boxList = [];
  for (let f = 0; f < floors.length; f++) {
    const y0 = f * fh;
    for (let tz = 0; tz < dep; tz++) {
      for (let tx = 0; tx < w; tx++) {
        const t = floors[f].tiles[tz * w + tx];
        if (t === TILE.WALL) {
          boxList.push([ox + tx, oz + tz, y0, fh,
            WALL_C.clone().multiplyScalar(0.94 + 0.06 * ((tx * 7 + tz * 13) % 5) / 4)]);
        } else if (t === TILE.CORE) {
          boxList.push([ox + tx, oz + tz, y0, fh, CORE_C.clone()]);
        } else if (t === TILE.DOOR) {
          boxList.push([ox + tx, oz + tz, y0 + fh * 0.84, fh * 0.16, DOOR_C.clone()]);
        }
      }
    }
  }
  const boxMesh = new THREE.InstancedMesh(
    UNIT_BOX, new THREE.MeshLambertMaterial({ color: 0xffffff }), boxList.length || 1
  );
  const m = new THREE.Matrix4(), q = new THREE.Quaternion();
  const p = new THREE.Vector3(), s = new THREE.Vector3(1, 1, 1);
  boxList.forEach(([x, z, y, h, c], i) => {
    p.set(x + 0.5, y + h / 2, z + 0.5);
    s.set(1, h, 1);
    boxMesh.setMatrixAt(i, m.compose(p, q, s));
    boxMesh.setColorAt(i, c);
  });
  if (boxList.length) {
    boxMesh.instanceMatrix.needsUpdate = true;
    if (boxMesh.instanceColor) boxMesh.instanceColor.needsUpdate = true;
  }

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
      UNIT_BOX, new THREE.MeshLambertMaterial({ color: 0xffffff }), furnList.length
    );
    const mm = new THREE.Matrix4(), qq = new THREE.Quaternion();
    const pp = new THREE.Vector3(), ss = new THREE.Vector3();
    furnList.forEach((it, i) => {
      pp.set(it.x + 0.5, it.y + 0.3, it.z + 0.5);
      ss.set(0.55, 0.6, 0.55);
      furnMesh.setMatrixAt(i, mm.compose(pp, qq, ss));
      furnMesh.setColorAt(i, new THREE.Color(it.c));
    });
    furnMesh.instanceMatrix.needsUpdate = true;
    if (furnMesh.instanceColor) furnMesh.instanceColor.needsUpdate = true;
  }

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
      UNIT_BOX, new THREE.MeshBasicMaterial({ color: 0xffe9b8 }), winList.length
    );
    const mm = new THREE.Matrix4(), pp = new THREE.Vector3();
    const ss = new THREE.Vector3(0.8, 1.1, 0.06);
    const e = new THREE.Euler();
    winList.forEach((it, i) => {
      pp.set(it.x, it.y + fh * 0.55, it.z);
      e.set(0, it.ry, 0);
      winMesh.setMatrixAt(i, mm.compose(pp, new THREE.Quaternion().setFromEuler(e), ss));
    });
    winMesh.instanceMatrix.needsUpdate = true;
  }

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
        else if (t === TILE.ROOM)
          c = new THREE.Color(ROOM_KIND_COLORS[(floors[f].kinds[i] || 0) % ROOM_KIND_COLORS.length]);
        else if (t === TILE.DOOR) c = new THREE.Color(DOORMAT);
        if (c) {
          addQuad(ox + tx, oz + tz, f * fh + 0.01, c);
          addQuad(ox + tx, oz + tz, (f + 1) * fh - 0.02, c);
        }
      }
    }
  }
  const floorGeom = new THREE.BufferGeometry();
  floorGeom.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  floorGeom.setAttribute("color", new THREE.Float32BufferAttribute(colors, 3));
  const floorMesh = new THREE.Mesh(
    floorGeom, new THREE.MeshLambertMaterial({ vertexColors: true, side: THREE.DoubleSide })
  );

  const frame = new THREE.LineSegments(
    new THREE.EdgesGeometry(new THREE.BoxGeometry(w + 0.05, fh, dep + 0.05)),
    new THREE.LineBasicMaterial({ color: 0x6ea8ff, transparent: true, opacity: 0.9 })
  );
  frame.position.set(d.wx, fh / 2, d.wz);

  interiorGroup.add(boxMesh, floorMesh, frame);
  if (furnMesh) interiorGroup.add(furnMesh);
  if (winMesh) interiorGroup.add(winMesh);
  interiorGroup.userData.fh = fh;

  storeySlider.max = String(floors.length - 1);
  storeySlider.value = "0";
  document.getElementById("storey-n").textContent = "0";
  document.getElementById("storey-total").textContent = String(floors.length);
}

function setActiveStorey(f, moveCamera) {
  if (!interior.floors.length) return;
  const n = interior.floors.length;
  interior.active = ((f % n) + n) % n;
  storeySlider.value = String(interior.active);
  document.getElementById("storey-n").textContent = String(interior.active);
  const frame = interiorGroup.children.find((c) => c.isLineSegments);
  const fh = CFG.floorHeight;
  if (frame) frame.position.set(interior.wx, interior.active * fh + fh / 2, interior.wz);
  if (moveCamera) PLAYER.anchorY = interior.active * fh + 1.6;
}
storeySlider.addEventListener("input", () => setActiveStorey(+storeySlider.value, true));

/* Hide/show everything outside the building. */
function setExteriorVisible(show) {
  for (const entry of chunks.values()) entry.group.visible = show;
  ground.visible = show;
  scene.background = show ? skyTex : INTERIOR_BG;
}

/* ---- fade + popup helpers ---- */
function fade(on) {
  return new Promise((resolve) => {
    fadeEl.classList.toggle("on", on);
    setTimeout(resolve, 430);
  });
}
let popupTimer = null;
function showPopup(text, ms = 2200) {
  popup.textContent = text;
  popup.classList.remove("hidden");
  if (popupTimer) clearTimeout(popupTimer);
  if (ms > 0) popupTimer = setTimeout(() => popup.classList.add("hidden"), ms);
}

function refreshModeHint() {
  if (mode === "inside") {
    modeHint.textContent = "WASD move · mouse look · R/F storey · ESC exit";
  } else {
    modeHint.textContent = "WASD move · mouse look (click) · ENTER enter · Shift run";
  }
}

/* ---- input ---- */
window.addEventListener("keydown", (e) => {
  if (mode === "menu") return;
  if (e.code === "Escape") {
    if (mode === "inside") { exitBuilding(); return; }
    return;
  }
  if (mode === "inside") {
    if (e.code === "KeyR") { setActiveStorey(interior.active + 1, true); return; }
    if (e.code === "KeyF") { setActiveStorey(interior.active - 1, true); return; }
  }
  if (e.code === "Enter") {
    if (mode === "street" && !transitionBusy) {
      const cell = centerBuilding();
      if (cell) enterBuilding(cell);
    }
    return;
  }
  PLAYER.keys.add(e.code);
});
window.addEventListener("keyup", (e) => PLAYER.keys.delete(e.code));

/* ---- world start ---- */
async function loadConfig() {
  const cfg = await invoke("get_config");
  Object.assign(CFG, {
    seed: cfg.seed,
    chunkSize: cfg.chunk_size || 32,
    zoneHues: cfg.zone_hues,
    zoneNames: cfg.zone_names,
    floorHeight: cfg.floor_height || 4.0,
    sdk: cfg.sdk || "",
    cellMeters: cfg.cell_meters || 4,
    eyeHeight: cfg.eye_height || 1.7,
    drawDistance: cfg.draw_distance || 8,
  });
  zoneInput.innerHTML = "";
  CFG.zoneNames.forEach((name, i) => {
    const opt = document.createElement("option");
    opt.value = String(i);
    opt.textContent = name.charAt(0).toUpperCase() + name.slice(1);
    zoneInput.appendChild(opt);
  });
  PLAYER.anchorY = CFG.eyeHeight;
}

function parseIntField(input, fallback = 0) {
  const raw = (input.value || "").trim();
  if (/^[+-]?\d+$/.test(raw)) return parseInt(raw, 10);
  return fallback;
}
function parseSeedField(input) {
  const raw = (input.value || "").trim();
  if (/^\d+$/.test(raw)) return raw.replace(/^0+(?=\d)/, "");
  return String(Math.floor(Math.random() * 1e9));
}
menuForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  startBtn.disabled = true;
  menuStatus.textContent = "generating world…";
  try {
    const seed = parseSeedField(seedInput);
    const zone = parseInt(zoneInput.value, 10) || 0;
    const wx = parseIntField(wxInput, 0);
    const wz = parseIntField(wzInput, 0);
    const cfg = await invoke("start_world", { seed });
    Object.assign(CFG, {
      seed: cfg.seed, chunkSize: cfg.chunk_size, zoneHues: cfg.zone_hues,
      zoneNames: cfg.zone_names, floorHeight: cfg.floor_height,
    });
    menuStatus.textContent = "locating district…";
    const spawn = await invoke("find_zone_start", { zone, wx, wz });
    await beginAt(spawn, seed);
  } catch (err) {
    menuStatus.textContent = "error: " + err;
    startBtn.disabled = false;
  }
});

async function beginAt(spawn, seed) {
  // Reset the streaming state for the new world.
  for (const [key, entry] of chunks) {
    scene.remove(entry.group);
    chunks.delete(key);
  }
  pickMeshes.length = 0;
  lastCenter = null;
  fetches = 0;

  PLAYER.pos.set(spawn.x, spawn.y, spawn.z);
  PLAYER.anchorY = spawn.y;
  PLAYER.yaw = Math.PI * 0.25;
  PLAYER.pitch = 0;
  PLAYER.keys.clear();

  seedLabel.textContent = `seed ${seed} · sdk ${CFG.sdk}`;
  ground.visible = true;

  // Prime the first ring so the ground isn't empty on the first frame.
  const ccx = divEuclid(Math.floor(spawn.x), CFG.chunkSize);
  const ccz = divEuclid(Math.floor(spawn.z), CFG.chunkSize);
  await loadSquare(ccx, ccz, STREAM_RADIUS);
  // Mark this column as current so the next loop tick doesn't refetch it.
  lastCenter = { cx: ccx, cy: ccz };

  mode = "street";
  menuEl.classList.add("hidden");
  hudEl.classList.remove("hidden");
  crosshair.classList.remove("hidden");
  refreshModeHint();
  showPopup(
    `spawned in ${CFG.zoneNames[spawn.zone]} · ` +
    `(${spawn.wx}, ${spawn.wz}) · ${spawn.distance} cells out`, 3200
  );
}

/* ---- main loop ---- */
const clock = new THREE.Clock();
let frameMs = 0;
let tick = 0;
renderer.setAnimationLoop(() => {
  const t0 = performance.now();
  const dt = Math.min(clock.getDelta(), 0.05);
  if (mode !== "menu") {
    updateStream();
    if (!transitionBusy) stepPlayer(dt);
  }
  renderer.render(scene, camera);
  frameMs = frameMs * 0.9 + (performance.now() - t0) * 0.1;
  if (++tick % 30 === 0) refreshStatus();
});

window.addEventListener("resize", () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
});

/* ---- boot ---- */
loadConfig()
  .then(() => { started = true; statusEl.textContent = "ready"; })
  .catch((e) => {
    menuStatus.textContent = "config failed: " + e;
  });
