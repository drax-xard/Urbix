/* server/www/app.js
 *
 * Three.js viewer for the Urbix 3D-Explorer SDK.
 *
 * Feeds off the sibling micro-server (serve.c): /api/config for the engine's
 * deterministic settings, /api/chunk for streamed exterior cells, and
 * /api/interior for a selected building's storey grids (tiles + room kinds).
 *
 * Controls:
 *   orbit  — drag / scroll (damped), click a building to inspect its interior
 *   inside — WASD/arrows move, mouse-look (pointer lock on click), Q/E change
 *            storey, Esc exits back to orbit
 */
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";

const TILE = { VOID: 0, WALL: 1, DOOR: 2, CORE: 3, CORRIDOR: 4, ROOM: 5 };
const ROOM_KIND_COLORS = [
  0x7fb3d5, 0xf5b041, 0x82e0aa, 0xaf7ac5, 0xf1948a, 0x85c1e9,
  0xabebc6, 0xf9e79f, 0xd7bde2, 0xfadbd8,
];
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

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0e1622);
scene.fog = new THREE.Fog(0x0e1622, 180, 520);

const camera = new THREE.PerspectiveCamera(60, innerWidth / innerHeight, 0.1, 2000);
camera.position.set(64, 90, 96);

const hemi = new THREE.HemisphereLight(0xdfeaff, 0x3a4a5c, 0.9);
const sun = new THREE.DirectionalLight(0xfff2d8, 1.4);
sun.position.set(80, 160, -60);
scene.add(hemi, sun);

const controls = new OrbitControls(camera, canvas);
controls.target.set(16, 0, 16);
controls.enableDamping = true;
controls.maxPolarAngle = Math.PI / 2.02;
controls.minDistance = 2;
controls.maxDistance = 700;

/* Ground cover (roads live implicitly between boxes). */
const ground = new THREE.Mesh(
  new THREE.PlaneGeometry(6000, 6000),
  new THREE.MeshLambertMaterial({ color: 0x182434 })
);
ground.rotation.x = -Math.PI / 2;
ground.position.y = -0.02;
scene.add(ground);

/* ---- engine config (from the server, deterministic) ---- */
const CFG = { chunkSize: 32, zoneHues: [], zoneNames: [], floorHeight: 4.0, seed: 0 };
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
  });
  seedEl.textContent = `seed ${CFG.seed}`;
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
  const [r, g, b] = CFG.zoneHues[zone] || [150, 150, 150];
  const v = 0.62 + 0.10 * ((pal % 5) / 4) + Math.min(h / 600, 0.14);
  return new THREE.Color(r * v / 255, g * v / 255, b * v / 255);
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

function spawnChunk(c) {
  const key = c.cx + ":" + c.cy;
  if (chunks.has(key)) return;

  const built = c.cells.filter((cell) => cell.h > 0);
  const group = new THREE.Group();
  const data = [];
  let mesh = null;

  if (built.length > 0) {
    mesh = new THREE.InstancedMesh(
      new THREE.BoxGeometry(1, 1, 1),
      new THREE.MeshLambertMaterial({ color: 0xffffff }),
      built.length
    );
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    const m = new THREE.Matrix4();
    const q = new THREE.Quaternion();
    const p = new THREE.Vector3();
    const s = new THREE.Vector3();
    built.forEach((cell, i) => {
      const h = Math.max(cell.h, 0.05);
      p.set(cell.x, h / 2, cell.z);
      s.set(1, h, 1);
      mesh.setMatrixAt(i, m.compose(p, q, s));
      mesh.setColorAt(i, cellColor(cell.zone, cell.pal, h));
      data.push({ wx: cell.x, wz: cell.z, height: h, zone: cell.zone, i });
    });
    mesh.instanceMatrix.needsUpdate = true;
    const colors = mesh.instanceColor;
    if (colors) colors.needsUpdate = true;
    mesh.castShadow = false;
    group.add(mesh);
    pickMeshes.push(mesh);
  }

  group.userData = { cx: c.cx, cy: c.cy };
  chunks.set(key, { group, data, mesh, cx: c.cx, cy: c.cy });
  scene.add(group);
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

  for (let dy = -STREAM_RADIUS; dy <= STREAM_RADIUS; dy++)
    for (let dx = -STREAM_RADIUS; dx <= STREAM_RADIUS; dx++)
      loadChunk(c.cx + dx, c.cy + dy);

  for (const [key, entry] of chunks) {
    const d = Math.max(Math.abs(entry.cx - c.cx), Math.abs(entry.cy - c.cy));
    if (d > STREAM_RADIUS) {
      scene.remove(entry.group);
      pickMeshes = pickMeshes.filter((m) => m !== entry.mesh);
      entry.mesh?.dispose();
      entry.group.traverse((o) => o.geometry && (o.geometry.dispose(), o.material?.dispose()));
      chunks.delete(key);
    }
  }
  statusEl.textContent = `${chunks.size} chunk(s) · ${countBoxes()} boxes`;
}

function countBoxes() {
  let n = 0;
  for (const e of chunks.values()) n += e.data.length;
  return n;
}

/* ---- interior inspection (click a building) ---- */
let inspecting = null;  /* { wx, wz, zone, floors, boxMesh, floorMesh, active } */
const interiorGroup = new THREE.Group();
interiorGroup.visible = false;
scene.add(interiorGroup);

const popup = document.getElementById("popup");

async function inspectCell(wx, wz) {
  const r = await fetch(`/api/interior?wx=${wx}&wz=${wz}`);
  const d = await r.json();
  if (!d.floor_count || !d.floors.length) {
    popup.textContent = `(${wx}, ${wz}) — no interior`;
    popup.classList.remove("hidden");
    setTimeout(() => popup.classList.add("hidden"), 2000);
    return;
  }
  clearInspect();
  const floors = d.floors.slice(0, MAX_FLOORS);
  inspecting = { wx, wz, floors, zone: d.zone, active: 0 };
  buildInteriorMesh(d, floors);
  enterInspect(d);
}

function clearInspect() {
  interiorGroup.clear();
  for (const child of interiorGroup.children)
    child.geometry?.dispose(), child.material?.dispose();
  inspecting = null;
}

function buildInteriorMesh(d, floors) {
  const fh = CFG.floorHeight;
  const w = d.footprint_w;
  const dep = d.footprint_d;
  const ox = d.wx - Math.floor(w / 2);
  const oz = d.wz - Math.floor(dep / 2);

  /* Boxes: walls + core squares + doors, instanced. */
  const boxList = [];
  const boxColor = [];
  for (let f = 0; f < floors.length; f++) {
    const y0 = f * fh;
    for (let tz = 0; tz < dep; tz++) {
      for (let tx = 0; tx < w; tx++) {
        const t = floors[f].tiles[tz * w + tx];
        if (t === TILE.WALL || t === TILE.CORE || t === TILE.DOOR) {
          const h = t === TILE.CORE ? fh * 0.92 : t === TILE.WALL ? fh * 0.35 : fh * 0.25;
          boxList.push([ox + tx, y0 + h / 2, oz + tz, h]);
          boxColor.push(new THREE.Color(TILE_COLORS[t]));
        }
      }
    }
  }

  const boxMesh = new THREE.InstancedMesh(
    new THREE.BoxGeometry(1, 1, 1),
    new THREE.MeshLambertMaterial({ color: 0xffffff, transparent: true, opacity: 0.92 }),
    boxList.length || 1
  );
  const m = new THREE.Matrix4();
  const q = new THREE.Quaternion();
  const p = new THREE.Vector3();
  const s = new THREE.Vector3(1, 1, 1);
  boxList.forEach(([x, y, z, h], i) => {
    p.set(x, y, z);
    s.y = h;
    boxMesh.setMatrixAt(i, m.compose(p, q, s));
    boxMesh.setColorAt(i, boxColor[i]);
  });
  if (boxList.length) {
    boxMesh.instanceMatrix.needsUpdate = true;
    if (boxMesh.instanceColor) boxMesh.instanceColor.needsUpdate = true;
  }

  /* Floor surfaces: corridor + room tiles merged into one colored geometry. */
  const positions = [];
  const colors = [];
  const addQuad = (x, z, y, color) => {
    positions.push(x, y, z, x + 1, y, z, x + 1, y, z + 1, x, y, z, x + 1, y, z + 1, x, y, z + 1);
    for (let i = 0; i < 6; i++) colors.push(color.r, color.g, color.b);
  };
  for (let f = 0; f < floors.length; f++) {
    const y = (f + 1) * fh - 0.02; /* slab top, right under the next floor's walls */
    for (let tz = 0; tz < dep; tz++) {
      for (let tx = 0; tx < w; tx++) {
        const i = tz * w + tx;
        const t = floors[f].tiles[i];
        if (t === TILE.CORRIDOR) {
          addQuad(ox + tx, oz + tz, y, new THREE.Color(TILE_COLORS[TILE.CORRIDOR]));
        } else if (t === TILE.ROOM) {
          const k = (floors[f].kinds[i] || 0) % ROOM_KIND_COLORS.length;
          addQuad(ox + tx, oz + tz, y, new THREE.Color(ROOM_KIND_COLORS[k]));
        }
      }
    }
  }

  const floorGeom = new THREE.BufferGeometry();
  floorGeom.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  floorGeom.setAttribute("color", new THREE.Float32BufferAttribute(colors, 3));
  const floorMesh = new THREE.Mesh(
    floorGeom,
    new THREE.MeshLambertMaterial({ vertexColors: true, side: THREE.DoubleSide, transparent: true, opacity: 0.96 })
  );

  /* Translucent shell showing the exterior silhouette while inside. */
  const shell = new THREE.Mesh(
    new THREE.BoxGeometry(w, floors.length * fh, dep),
    new THREE.MeshLambertMaterial({ color: 0x8fa8c8, transparent: true, opacity: 0.08, depthWrite: false })
  );
  shell.position.set(d.wx, (floors.length * fh) / 2, d.wz);

  /* Active-storey highlight. */
  const frame = new THREE.LineSegments(
    new THREE.EdgesGeometry(new THREE.BoxGeometry(w + 0.05, 1, dep + 0.05)),
    new THREE.LineBasicMaterial({ color: 0x6ea8ff, transparent: true, opacity: 0.9 })
  );

  interiorGroup.add(boxMesh, floorMesh, shell, frame);
  interiorGroup.userData = { boxMesh, floorMesh, frame, fh };

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
  if (frame) frame.position.set(inspecting.wx, inspecting.active * fh - 0.35, inspecting.wz);
  if (moveCamera) {
    camera.position.y = inspecting.active * fh + fh / 2;
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
  const entry = [...chunks.values()].find((e) => e.mesh === hit.object);
  if (!entry) return;
  const cell = entry.data[hit.instanceId];
  inspectCell(cell.wx, cell.wz);
}

/* fly-in controls (while inspecting) */
const fly = { yaw: 0, pitch: -0.25, keys: new Set(), look: false };
let pointerX = 0;
let pointerY = 0;

canvas.addEventListener("mousemove", (e) => {
  pointerX = e.clientX;
  pointerY = e.clientY;
  if (inspecting && fly.look && document.pointerLockElement === canvas) {
    fly.yaw -= e.movementX * 0.003;
    fly.pitch = Math.max(-1.5, Math.min(1.5, fly.pitch - e.movementY * 0.003));
  }
});

canvas.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  if (inspecting) {
    if (!fly.look) {
      canvas.requestPointerLock();
      fly.look = true;
      modeHint.textContent = "WASD move · Q/E floor · Esc exit";
    }
    return;
  }
  pickCell();
});

document.addEventListener("pointerlockchange", () => {
  fly.look = document.pointerLockElement === canvas;
  if (!fly.look) return;
  const dir = new THREE.Vector3();
  camera.getWorldDirection(dir);
  fly.yaw = Math.atan2(-dir.x, -dir.z);
  fly.pitch = 0;
});

const MOVE_SPEED = 14;
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
    camera.position.add(move);
  }
  camera.rotation.order = "YXZ";
  camera.rotation.y = fly.yaw;
  camera.rotation.x = fly.pitch;
}

window.addEventListener("keydown", (e) => {
  if (e.code === "Escape" && inspecting) { exitInspect(); return; }
  if (inspecting) {
    if (e.code === "KeyQ") setActiveStorey(inspecting.active - 1, false);
    if (e.code === "KeyE") setActiveStorey(inspecting.active + 1, true);
    fly.keys.add(e.code);
  }
});
window.addEventListener("keyup", (e) => fly.keys.delete(e.code));

function enterInspect(d) {
  controls.enabled = false;
  interiorGroup.visible = true;
  modeHint.textContent = "click to grab mouse · WASD/arrows move · Q/E floor · Esc exit";
  popup.textContent =
    `(${d.wx}, ${d.wz}) · ${CFG.zoneNames[d.zone] || "?"} · ` +
    `${d.floors.length} storeys · ${d.footprint_w}×${d.footprint_d}`;
  popup.classList.remove("hidden");
  const fh = CFG.floorHeight;
  camera.rotation.order = "YXZ";
  camera.position.set(d.wx, fh * 0.6, d.wz - 3.2);
  fly.yaw = 0;
  fly.pitch = 0;
  setActiveStorey(0, false);
  updateStream(); /* pull in surrounding chunks so the street continues */
}

function exitInspect() {
  if (document.pointerLockElement) document.exitPointerLock();
  clearInspect();
  interiorGroup.visible = false;
  storeyBox.classList.add("hidden");
  popup.classList.add("hidden");
  controls.enabled = true;
  modeHint.textContent = "drag orbit · scroll zoom · click a building to enter";
}

/* ---- main loop ---- */
const clock = new THREE.Clock();
renderer.setAnimationLoop(() => {
  const dt = Math.min(clock.getDelta(), 0.05);
  if (CFG.chunkSize && (inspecting || controls.enabled)) updateStream();
  if (inspecting) stepFly(dt);
  else controls.update();
  renderer.render(scene, camera);
});

window.addEventListener("resize", () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
});