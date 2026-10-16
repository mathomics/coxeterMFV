#!/usr/bin/env node
// Reference outputs of the map stage of src/ui/app.js for the gesture tests
// of rust/crates/coxeter-canvas: the real application, in headless Firefox,
// driven with real pointer, wheel, touch and key input over WebDriver BiDi.
//
//   node rust/crates/coxeter-canvas/tools/fixtures-canvas.mjs
//
// Writes rust/crates/coxeter-canvas/tests/fixtures/gestures.json: the map the
// cases start from (the E. coli scenario's, with a free text added), what the
// renderer is told besides the map, the stage's box, and per case: the tool,
// the camera, every input step with the element the browser found under each
// press (`document.elementFromPoint`), and what the stage did — the map's
// change as a list of JSON edits, the selection, the multi-selection, the undo
// labels, the camera, the open menus, the status line, the text editor.
//
// The points pressed are chosen in the browser, on elements it really hits
// there, so the Rust test replays exactly these client coordinates.
//
// Needs Firefox (FIREFOX_PATH, or the usual install places) and Node 22+ for
// the built-in WebSocket. Nothing is shown on screen (--headless); the
// browser, its profile and the little HTTP server are removed at the end.
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { existsSync } from 'node:fs';
import { readFile, writeFile, mkdir, mkdtemp, rm, stat } from 'node:fs/promises';
import { dirname, extname, join, normalize } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import net from 'node:net';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '../../../..');
const outDir = join(here, '../tests/fixtures');

/* ------------------------------------------------------------- the server */

const TYPES = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.json': 'application/json; charset=utf-8', '.svg': 'image/svg+xml',
  '.wasm': 'application/wasm', '.png': 'image/png', '.xml': 'application/xml',
};
const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://localhost');
    let path = normalize(decodeURIComponent(url.pathname));
    if (path.endsWith('/') || path.endsWith('\\')) path += 'index.html';
    if (path.includes('..')) { res.writeHead(403).end(); return; }
    const file = join(root, path);
    if (!(await stat(file)).isFile()) { res.writeHead(404).end(); return; }
    res.writeHead(200, { 'Content-Type': TYPES[extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-cache' });
    res.end(await readFile(file));
  } catch { res.writeHead(404).end(); }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

/* ------------------------------------------------------------ the browser */

const FIREFOX = [
  process.env.FIREFOX_PATH,
  'C:/Program Files/Mozilla Firefox/firefox.exe',
  'C:/Program Files (x86)/Mozilla Firefox/firefox.exe',
  '/usr/bin/firefox', '/usr/lib/firefox/firefox', '/snap/bin/firefox',
  '/Applications/Firefox.app/Contents/MacOS/firefox',
].find((p) => p && existsSync(p));
if (!FIREFOX) { console.error('No Firefox found; set FIREFOX_PATH.'); server.close(); process.exit(2); }

const freePort = () => new Promise((r) => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(p)); }); });
const port = await freePort();
const profile = await mkdtemp(join(tmpdir(), 'coxeter-canvas-ff-'));
const browser = spawn(FIREFOX, ['--headless', `--remote-debugging-port=${port}`, '--profile', profile, '--no-remote', 'about:blank'], {
  stdio: 'ignore', env: { ...process.env, MOZ_HEADLESS: '1' },
});

let ws = null;
async function cleanup() {
  try { ws?.close(); } catch { /* closed */ }
  if (browser.exitCode === null) {
    if (process.platform === 'win32') {
      try { execFileSync('taskkill', ['/PID', String(browser.pid), '/T', '/F'], { stdio: 'ignore' }); } catch { /* gone */ }
    } else browser.kill('SIGKILL');
  }
  server.close();
  await new Promise((r) => setTimeout(r, 800));
  await rm(profile, { recursive: true, force: true }).catch(() => {});
}
process.on('SIGINT', async () => { await cleanup(); process.exit(130); });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
for (let k = 0; k < 120 && !ws; k++) {
  await sleep(500);
  try {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/session`);
    await new Promise((res, rej) => { socket.onopen = res; socket.onerror = rej; });
    ws = socket;
  } catch { /* not up yet */ }
}
if (!ws) { console.error('Firefox did not open its WebDriver BiDi port.'); await cleanup(); process.exit(2); }

let nextId = 0;
const pending = new Map();
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
};
async function send(method, params = {}) {
  const id = ++nextId;
  const reply = await new Promise((r) => { pending.set(id, r); ws.send(JSON.stringify({ id, method, params })); });
  if (reply.type === 'error') throw new Error(`${method}: ${reply.error} ${reply.message}`);
  return reply.result;
}

let context;
/** Evaluate a function body in the page (async allowed); its JSON result. */
async function page(body, arg = null) {
  const expression = `(async (arg) => { ${body} })(${JSON.stringify(arg)}).then((v) => JSON.stringify(v ?? null))`;
  const r = await send('script.evaluate', { expression, target: { context }, awaitPromise: true });
  if (r.type === 'exception') throw new Error(`page: ${r.exceptionDetails.text}\n${body.slice(0, 300)}`);
  return JSON.parse(r.result.value);
}
const frames = () => page('await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))); return true;');

/* --------------------------------------------------------------- the app */

const VIEW = { width: 1600, height: 1000 };

async function boot() {
  await send('session.new', { capabilities: {} });
  const tree = await send('browsingContext.getTree', {});
  context = tree.contexts[0].context;
  await send('browsingContext.setViewport', { context, viewport: VIEW });
  await send('browsingContext.navigate', { context, url: `${base}/index.html?scenario=solo-ecoli`, wait: 'complete' });
  for (let k = 0; k < 240; k++) {
    const ok = await page(`const s = window.escherMCMC?.store; return Boolean(s?.map && s.solution && document.querySelector('#stage svg'));`);
    if (ok) break;
    if (k === 239) throw new Error('the app did not boot');
    await sleep(250);
  }
  // The inspector's column stays open (its Layout tab), so the stage never
  // changes size under a gesture; every label is written, whatever the fluxes.
  await page(`
    document.querySelector('[data-action="right-tab"][data-tab="layout"]').click();
    const { store } = window.escherMCMC;
    store.setView({ labels: 'all' });
    store.addMapText('Free text', 300, 120, { fontSize: 22, fontFamily: 'sans' });
    store.mapHistory.clear();
    return true;`);
  await frames();
}

/** The JS side of every case: helpers installed once in the page. */
const HELPERS = `
  const E = window.escherMCMC; const store = E.store;
  const stage = document.getElementById('stage');
  const describe = (el) => {
    if (!el) return null;
    const data = {};
    for (const a of el.attributes ?? []) if (a.name.startsWith('data-')) data[a.name] = a.value;
    return { tag: el.tagName.toLowerCase(), cls: el.getAttribute?.('class') ?? '', data };
  };
  const centre = (el) => { const b = el.getBoundingClientRect(); return { x: Math.round(b.left + b.width / 2), y: Math.round(b.top + b.height / 2) }; };
  const inStage = (p) => { const b = stage.getBoundingClientRect(); return p.x > b.left + 4 && p.y > b.top + 4 && p.x < b.right - 4 && p.y < b.bottom - 4; };
  /** The first element of a selector whose point the browser hits as itself. */
  const pick = (selector, pointOf = centre, accept = (hit, el) => hit === el || el.contains(hit)) => {
    for (const el of document.querySelectorAll(selector)) {
      const p = pointOf(el);
      if (!p || !inStage(p)) continue;
      const hit = document.elementFromPoint(p.x, p.y);
      if (hit && accept(hit, el)) return { ...p, el };
    }
    return null;
  };
  window.__cx = { store, stage, describe, centre, pick };
`;

async function state() {
  return page(`
    const { store } = window.escherMCMC; const cam = window.escherMCMC.camera;
    const sel = store.multiSelection;
    const menus = [...document.querySelectorAll('#stage .boundary-menu, #stage .add-reaction-menu')].map((m) =>
      m.classList.contains('boundary-menu') ? 'boundary' : m.classList.contains('add-metabolite-menu') ? 'metabolite-add' : 'reaction-panel');
    const editor = document.querySelector('#stage .map-text-editor input[data-role="label"]');
    return {
      map: store.map.toJSON(),
      selectedReaction: store.selectedReaction ?? null,
      selectedEntity: store.selectedEntity ?? null,
      selectedGeometry: store.selectedGeometry ?? null,
      multiSelection: { nodes: [...sel.nodes], reactions: [...sel.reactions], texts: [...sel.texts] },
      undo: store.mapHistory.undoStack.map((e) => e.label),
      camera: { x: cam.x, y: cam.y, scale: cam.scale },
      menus,
      status: store.status?.message ?? null,
      editor: editor ? editor.value : null,
      view: { geometryEdit: store.view.geometryEdit, textTool: store.view.textTool, multiSelect: store.view.multiSelect },
    };`);
}

/** Edits that turn JSON `a` into `b`: `[path, value]`, value absent for a removal. */
function diff(a, b, path = [], out = []) {
  const isObj = (v) => v && typeof v === 'object';
  if (Array.isArray(a) && Array.isArray(b) && a.length === b.length) {
    a.forEach((v, i) => diff(v, b[i], [...path, i], out));
  } else if (isObj(a) && isObj(b) && !Array.isArray(a) && !Array.isArray(b)) {
    for (const k of Object.keys(a)) if (!(k in b)) out.push([[...path, k]]);
    for (const k of Object.keys(b)) {
      if (!(k in a)) out.push([[...path, k], b[k]]);
      else diff(a[k], b[k], [...path, k], out);
    }
  } else if (JSON.stringify(a) !== JSON.stringify(b)) {
    out.push([path, b]);
  }
  return out;
}

/* ------------------------------------------------------------------ input */

const mouse = (actions) => ({ type: 'pointer', id: 'mouse', parameters: { pointerType: 'mouse' }, actions });
async function act(sources) {
  await send('input.performActions', { context, actions: sources });
  await frames();
}

/** One case: reset, set up, run the steps, record. */
async function runCase(c, initial) {
  await send('input.releaseActions', { context });
  await page(`${HELPERS}
    const { EditableMap } = await import('/src/map/editable-map.js');
    for (const b of document.querySelectorAll('#stage [data-close-add-metabolite], #stage .add-reaction-close, #stage .boundary-menu-close')) b.click();
    document.querySelector('[data-action="tool-select"]').click();
    store.map = EditableMap.fromJSON(arg.map);
    store.mapHistory.clear();
    store.multiSelection = { nodes: new Set(), reactions: new Set(), texts: new Set() };
    store.selectedReaction = null; store.selectedEntity = null; store.selectedGeometry = null;
    store.status = { ...store.status, message: '' };
    document.querySelector('[data-action="tool-' + arg.tool + '"]').click();
    Object.assign(window.escherMCMC.camera, arg.camera, { fitted: true });
    store.emit('map');
    return true;`, { map: c.map ?? initial, tool: c.tool, camera: c.camera });
  await frames();
  if (c.setup) { await page(`${HELPERS} ${c.setup}`); await frames(); }
  const stageBox = await page(`const b = document.getElementById('stage').getBoundingClientRect(); return { left: b.left, top: b.top, width: b.width, height: b.height };`);
  const before = await state();
  const steps = [];
  let anchor = null;
  for (const step of c.steps) {
    let s = { ...step };
    if (s.at) {
      // A point the browser chooses, on the element named.
      const found = await page(`${HELPERS} const r = (${s.at})(window.__cx); return r ? { x: r.x, y: r.y } : null;`);
      if (!found) throw new Error(`${c.name}: no point for ${s.at}`);
      anchor = found;
      s.x = found.x; s.y = found.y;
      delete s.at;
    }
    if (s.dx !== undefined && s.type !== 'wheel') { s.x = anchor.x + s.dx; s.y = anchor.y + s.dy; }
    if (s.type === 'down' || s.type === 'up' || s.type === 'click') {
      s.target = await page(`return window.__cx.describe(document.elementFromPoint(arg.x, arg.y));`, { x: s.x, y: s.y });
    }
    const button = s.button ?? 0;
    const keys = s.shift ? [{ type: 'key', id: 'kbd', actions: [{ type: 'keyDown', value: '\uE008' }, { type: 'pause' }, { type: 'pause' }, { type: 'keyUp', value: '\uE008' }] }] : [];
    const hold = s.shift ? [{ type: 'key', id: 'kbd', actions: [{ type: 'keyDown', value: '' }, { type: 'pause' }] }] : [];
    const let_go = s.shift ? [{ type: 'key', id: 'kbd', actions: [{ type: 'pause' }, { type: 'pause' }, { type: 'keyUp', value: '' }] }] : [];
    if (s.type === 'down') {
      await act([mouse([{ type: 'pointerMove', x: s.x, y: s.y }, { type: 'pointerDown', button }]), ...hold]);
    } else if (s.type === 'move') {
      await act([mouse([{ type: 'pointerMove', x: s.x, y: s.y }])]);
    } else if (s.type === 'up') {
      await act([mouse([{ type: 'pointerMove', x: s.x, y: s.y }, { type: 'pointerUp', button }, { type: 'pause' }]), ...let_go]);
    } else if (s.type === 'click') {
      await act([mouse([{ type: 'pointerMove', x: s.x, y: s.y }, { type: 'pointerDown', button }, { type: 'pointerUp', button }, { type: 'pause' }]), ...keys]);
    } else if (s.type === 'wheel') {
      const at = anchor ?? { x: s.x, y: s.y };
      s.x = at.x; s.y = at.y;
      await act([{ type: 'wheel', id: 'wheel', actions: [{ type: 'scroll', x: s.x, y: s.y, deltaX: s.wx ?? 0, deltaY: s.wy, duration: 0 }] }]);
    } else if (s.type === 'key') {
      await act([{ type: 'key', id: 'kbd', actions: [{ type: 'keyDown', value: s.key }, { type: 'keyUp', value: s.key }] }]);
    } else if (s.type === 'type') {
      await act([{ type: 'key', id: 'kbd', actions: [...s.text].flatMap((ch) => [{ type: 'keyDown', value: ch }, { type: 'keyUp', value: ch }]) }]);
    } else if (s.type === 'ctrl') {
      await act([{ type: 'key', id: 'kbd', actions: [{ type: 'keyDown', value: '' }, { type: 'keyDown', value: s.key }, { type: 'keyUp', value: s.key }, { type: 'keyUp', value: '' }] }]);
    } else if (s.type === 'touch') {
      // One finger along a path.
      const [first, ...rest] = s.path.map((d) => ({ x: anchor.x + d[0], y: anchor.y + d[1] }));
      s.points = [first, ...rest];
      await act([{ type: 'pointer', id: 'finger', parameters: { pointerType: 'touch' }, actions: [
        { type: 'pointerMove', x: first.x, y: first.y }, { type: 'pointerDown', button: 0 },
        ...rest.map((p) => ({ type: 'pointerMove', x: p.x, y: p.y })), { type: 'pointerUp', button: 0 }] }]);
    } else if (s.type === 'pinch') {
      // Two fingers: down at a and b, then each moved by its own offset.
      const finger = (id, p, q) => ({ type: 'pointer', id, parameters: { pointerType: 'touch' }, actions: [
        { type: 'pointerMove', x: p.x, y: p.y }, { type: 'pointerDown', button: 0 },
        { type: 'pointerMove', x: q.x, y: q.y }, { type: 'pointerUp', button: 0 }] });
      await act([finger('f1', s.a, s.a2), finger('f2', s.b, s.b2)]);
    }
    steps.push(s);
  }
  await frames();
  const after = await state();
  const record = {
    name: c.name, tool: c.tool, camera: c.camera, stage: stageBox,
    setup: c.setupRust ?? null,
    start: c.map ? 'own' : 'initial',
    map: c.map ?? undefined,
    before: { ...before, map: undefined },
    steps,
    after: { ...after, map: undefined },
    mapEdits: diff(before.map, after.map),
  };
  if (c.setup) record.setupMapEdits = diff(c.map ?? initial, before.map);
  return record;
}

/* ------------------------------------------------------------------ cases */

const CAM = { x: 40, y: 30, scale: 0.9 };
/** The whole cell in view, its middle included. */
const WIDE = { x: -12, y: 8, scale: 0.27 };
/** The middle of the cell, where the internal reactions are. */
const MID = { x: -614, y: -429, scale: 0.6 };
const node = `({ pick }) => pick('.emc-node[data-node]', undefined, (hit, el) => (hit.closest('.emc-node[data-node], .emc-hit-node[data-node]')?.dataset.node === el.dataset.node) && window.escherMCMC.store.canMoveNode(el.dataset.node) && window.escherMCMC.store.map.nodes.get(el.dataset.node).kind === 'metabolite')`;
const marker = `({ pick }) => pick('.emc-marker[data-rxn]', undefined, (hit, el) => hit.closest('[data-rxn]')?.dataset.rxn === el.dataset.rxn && window.escherMCMC.store.canMoveReaction(el.dataset.rxn))`;
/** A reaction's marker, or the handle drawn over it in Edit. */
const markerEdit = `({ pick }) => pick('.emc-marker[data-rxn]', undefined, (hit, el) => (hit.closest('[data-reaction-handle]')?.dataset.reactionHandle ?? hit.closest('[data-rxn]')?.dataset.rxn) === el.dataset.rxn && window.escherMCMC.store.canMoveReaction(el.dataset.rxn))`;
const markerRotatable = markerEdit.replace('canMoveReaction(el.dataset.rxn))', 'canMoveReaction(el.dataset.rxn) && window.escherMCMC.store.canRotateReaction(el.dataset.rxn))');
const markerNth = (n) => `({ pick }) => { let k = 0; return pick('.emc-marker[data-rxn]', undefined, (hit, el) => hit.closest('[data-rxn]')?.dataset.rxn === el.dataset.rxn && window.escherMCMC.store.canMoveReaction(el.dataset.rxn) && k++ === ${n}); }`;
const label = `({ pick }) => pick('[data-metabolite-label]', undefined, (hit, el) => hit.closest('[data-metabolite-label]') === el)`;
const text = `({ pick }) => pick('[data-map-text]', undefined, (hit, el) => hit.closest('[data-map-text]') === el)`;
/** The first membrane band met going right from the stage's left edge, at mid height. */
const membraneLeft = `({ stage }) => {
  const st = stage.getBoundingClientRect(); const y = Math.round(st.top + st.height * 0.45);
  for (let x = Math.round(st.left) + 6; x < st.right - 6; x++) {
    const hit = document.elementFromPoint(x, y);
    if (hit?.closest('[data-boundary]') && hit.classList.contains('emc-boundary-hit') && !hit.closest('[data-rxn], [data-node]')) {
      // Into the band, a few pixels past its edge.
      const h2 = document.elementFromPoint(x + 5, y);
      if (h2 === hit) return { x: x + 5, y };
    }
  }
  return null; }`;
const handle = (h) => `({ pick }) => pick('[data-geometry-handle="${h}"]', undefined, (hit, el) => hit.closest('[data-geometry-handle]') === el)`;
const compLabel = `({ pick }) => pick('[data-compartment-label]', undefined, (hit, el) => hit.closest('[data-compartment-label]') === el)`;
/** On a canvas grip, inside the map's box (an SVG clips what overflows it). */
const resize = (edge) => `({ pick, stage }) => pick('[data-canvas-resize="${edge}"]', (el) => {
  const b = el.getBoundingClientRect(); const v = document.querySelector('#stage svg').getBoundingClientRect(); const st = stage.getBoundingClientRect();
  const l = Math.max(b.left, v.left, st.left), r = Math.min(b.right, v.right, st.right), t = Math.max(b.top, v.top, st.top), u = Math.min(b.bottom, v.bottom, st.bottom);
  return { x: Math.round((l + r) / 2), y: Math.round((t + u) / 2) }; },
  (hit) => hit.closest('[data-canvas-resize]')?.dataset.canvasResize === '${edge}')`;
const controlB1 = `({ pick }) => pick('[data-control-handle="b1"]', undefined, (hit, el) => hit === el)`;
const rotate = `({ pick }) => pick('[data-rotate-handle]', undefined, (hit, el) => hit === el)`;
/** A point inside a compartment where nothing else answers. */
const space = `({ stage }) => {
  const b = stage.getBoundingClientRect();
  for (let y = b.top + 60; y < b.bottom - 60; y += 9) for (let x = b.left + 60; x < b.right - 60; x += 9) {
    const hit = document.elementFromPoint(x, y);
    if (hit?.closest('[data-compartment-space]') && !hit.closest('[data-rxn], [data-node], [data-boundary], [data-metabolite-label], [data-map-text], [data-compartment-label], [data-cell-plate], .emc-hit')) return { x, y };
  }
  return null; }`;
/** A point on the canvas outside every cell (the shared medium). */
const medium = `({ stage }) => {
  const b = stage.getBoundingClientRect();
  for (let y = b.top + 50; y < b.bottom - 50; y += 7) for (let x = b.left + 50; x < b.right - 50; x += 7) {
    const hit = document.elementFromPoint(x, y);
    if (hit && hit.closest('#stage svg') && !hit.closest('[data-compartment-space], [data-rxn], [data-node], [data-boundary], [data-metabolite-label], [data-map-text], [data-compartment-label], [data-cell-plate], [data-canvas-resize], .emc-hit')) return { x, y };
  }
  return null; }`;

const drag = (at, moves, extra = {}) => [
  { type: 'down', at, ...extra },
  ...moves.map(([dx, dy]) => ({ type: 'move', dx, dy })),
  { type: 'up', dx: moves.at(-1)[0], dy: moves.at(-1)[1] },
];

const CASES = [
  { name: 'node-drag', tool: 'select', camera: CAM, steps: drag(node, [[6, 4], [24, 15], [41, 22]]) },
  { name: 'node-click-selects', tool: 'select', camera: CAM, steps: [{ type: 'click', at: node }] },
  { name: 'node-drag-edit-tool', tool: 'edit', camera: CAM, steps: drag(node, [[-30, 12], [-55, 40]]) },
  { name: 'reaction-marker-drag', tool: 'select', camera: CAM, steps: drag(marker, [[3, 1], [20, -18], [33, -40]]) },
  { name: 'reaction-marker-click', tool: 'select', camera: CAM, steps: [{ type: 'click', at: marker }] },
  { name: 'reaction-tremor-is-a-click', tool: 'select', camera: CAM, steps: drag(marker, [[2, 1], [3, 1]]) },
  { name: 'metabolite-label-drag', tool: 'select', camera: CAM, steps: drag(label, [[10, 5], [20, 30], [-15, 40]]) },
  { name: 'metabolite-label-click', tool: 'select', camera: CAM, steps: [{ type: 'click', at: label }] },
  { name: 'selection-rectangle', tool: 'select', camera: CAM, steps: drag(space, [[30, 30], [260, 210]]) },
  { name: 'background-click-deselects', tool: 'select', camera: CAM,
    setup: `store.select(Object.keys(Object.fromEntries(store.map.reactions))[0]); store.toggleMultiSelection('node', [...store.map.nodes.keys()][0]);`,
    setupRust: 'select-first-reaction-and-node', steps: [{ type: 'click', at: space }] },
  { name: 'shift-click-toggles', tool: 'select', camera: CAM, steps: [{ type: 'click', at: marker, shift: true }, { type: 'click', at: node, shift: true }] },
  { name: 'group-drag', tool: 'select', camera: CAM,
    steps: [{ type: 'click', at: markerNth(0), shift: true }, { type: 'click', at: markerNth(1), shift: true }, ...drag(markerNth(0), [[10, 10], [40, 25]])] },
  { name: 'membrane-drag-resizes', tool: 'select', camera: CAM, steps: drag(membraneLeft, [[-5, 0], [-40, 0]]) },
  { name: 'membrane-click-selects-compartment', tool: 'select', camera: CAM, steps: [{ type: 'click', at: membraneLeft }] },
  { name: 'compartment-click-selects', tool: 'select', camera: CAM, steps: [{ type: 'click', at: space }] },
  { name: 'geometry-handle-move', tool: 'select', camera: WIDE,
    steps: [{ type: 'click', at: membraneLeft }, ...drag(handle('move'), [[8, 6], [30, 22]])] },
  { name: 'geometry-handle-resize', tool: 'select', camera: CAM,
    steps: [{ type: 'click', at: membraneLeft }, ...drag(handle('nw'), [[10, 10], [45, 35]])] },
  { name: 'geometry-handle-resize-wide', tool: 'select', camera: WIDE,
    steps: [{ type: 'click', at: membraneLeft }, ...drag(handle('se'), [[-10, -10], [-45, -35]])] },
  { name: 'wide-node-drag', tool: 'select', camera: WIDE, steps: drag(node, [[6, 4], [24, 15]]) },
  { name: 'wide-selection-rectangle', tool: 'select', camera: WIDE, steps: drag(space, [[30, 30], [300, 240]]) },
  { name: 'compartment-label-drag', tool: 'select', camera: CAM, steps: drag(compLabel, [[10, 0], [80, 30]]) },
  { name: 'compartment-label-click', tool: 'select', camera: CAM, steps: [{ type: 'click', at: compLabel }] },
  { name: 'canvas-resize-right', tool: 'select', camera: WIDE, steps: drag(resize('right'), [[20, 0], [90, 0]]) },
  { name: 'canvas-resize-left', tool: 'select', camera: CAM, steps: drag(resize('left'), [[-20, 0], [-70, 5]]) },
  { name: 'canvas-resize-top', tool: 'select', camera: CAM, steps: drag(resize('top'), [[0, -20], [3, -60]]) },
  { name: 'canvas-resize-corner', tool: 'select', camera: WIDE, steps: drag(resize('corner'), [[20, 20], [50, 70]]) },
  { name: 'pan-tool', tool: 'pan', camera: CAM, steps: drag(node, [[10, 10], [120, 70]]) },
  { name: 'pan-tool-click-selects-nothing', tool: 'pan', camera: CAM, steps: [{ type: 'click', at: marker }] },
  { name: 'middle-button-pan', tool: 'select', camera: CAM, steps: drag(marker, [[-10, 5], [-90, 60]], { button: 1 }).map((s) => ({ ...s, button: 1 })) },
  { name: 'wheel-zoom', tool: 'select', camera: CAM, steps: [{ type: 'move', at: node }, { type: 'wheel', wy: 120 }, { type: 'wheel', wy: 240 }, { type: 'wheel', wy: -100 }] },
  { name: 'wheel-pan', tool: 'select', camera: CAM, steps: [{ type: 'move', at: node }, { type: 'wheel', wy: 12, wx: 0 }, { type: 'wheel', wy: 0, wx: -18 }] },
  { name: 'edit-control-point', tool: 'edit', camera: CAM, steps: [{ type: 'click', at: markerEdit }, ...drag(controlB1, [[5, 5], [25, -20]])] },
  { name: 'edit-rotate', tool: 'edit', camera: MID, steps: [{ type: 'click', at: markerRotatable },
  { type: 'down', at: rotate }, { type: 'move', dx: 5, dy: 5 }, { type: 'move', dx: 40, dy: 60 }, { type: 'up', dx: 40, dy: 60 }] },
  { name: 'mid-node-drag-merge-check', tool: 'select', camera: MID, steps: drag(node, [[6, 4], [24, 15], [41, 22]]) },
  { name: 'mid-reaction-drag', tool: 'select', camera: MID, steps: drag(marker, [[3, 1], [20, -18], [33, -40]]) },
  { name: 'edit-reaction-drag', tool: 'edit', camera: CAM, steps: [...drag(markerEdit, [[8, 0], [30, 25]])] },
  { name: 'edit-text-drag', tool: 'edit', camera: CAM, steps: drag(text, [[10, 10], [60, 40]]) },
  { name: 'select-text-click-opens-nothing', tool: 'select', camera: CAM, steps: [{ type: 'click', at: text }] },
  { name: 'edit-text-click-opens-editor', tool: 'edit', camera: CAM, steps: [{ type: 'click', at: text }, { type: 'type', text: ' more' }, { type: 'key', key: '\uE007' }] },
  { name: 'text-tool-adds-text', tool: 'text', camera: CAM, steps: [{ type: 'click', at: space }, { type: 'type', text: 'Hello' }, { type: 'key', key: '\uE007' }] },
  { name: 'text-tool-escape-drops', tool: 'text', camera: CAM, steps: [{ type: 'click', at: space }, { type: 'type', text: 'Gone' }, { type: 'key', key: '\uE00C' }] },
  { name: 'reaction-tool-node-opens-menu', tool: 'reaction', camera: CAM, steps: [{ type: 'click', at: node }] },
  { name: 'reaction-tool-compartment-menu', tool: 'reaction', camera: CAM, steps: [{ type: 'click', at: space }] },
  { name: 'reaction-tool-membrane-menu', tool: 'reaction', camera: CAM, steps: [{ type: 'click', at: membraneLeft }] },
  { name: 'reaction-tool-reaction-selects', tool: 'reaction', camera: CAM, steps: [{ type: 'click', at: marker }] },
  { name: 'metabolite-tool-compartment-menu', tool: 'metabolite', camera: CAM, steps: [{ type: 'click', at: space }] },
  { name: 'metabolite-tool-on-reaction', tool: 'metabolite', camera: CAM, steps: [{ type: 'click', at: marker }] },
  { name: 'delete-selected-reaction', tool: 'select', camera: CAM, steps: [{ type: 'click', at: marker }, { type: 'key', key: '\uE017' }] },
  { name: 'escape-clears', tool: 'select', camera: CAM, steps: [{ type: 'click', at: marker }, { type: 'key', key: '\uE00C' }] },
  { name: 'escape-cancels-label-drag', tool: 'select', camera: CAM,
    steps: [{ type: 'down', at: label }, { type: 'move', dx: 20, dy: 30 }, { type: 'key', key: '\uE00C' }, { type: 'up', dx: 20, dy: 30 }] },
  { name: 'undo-redo-keys', tool: 'select', camera: CAM, steps: [...drag(node, [[30, 30]]), { type: 'ctrl', key: 'z' }] },
  { name: 'edit-membrane-drag-shared-medium', tool: 'edit', camera: CAM, steps: drag(membraneLeft, [[-6, 0], [-30, 0]]) },
  { name: 'text-tool-click-elsewhere-commits', tool: 'text', camera: CAM,
    steps: [{ type: 'click', at: space }, { type: 'type', text: 'Kept' }, { type: 'click', dx: 0, dy: 90 }] },
  { name: 'delete-multi-selection', tool: 'select', camera: CAM,
    steps: [{ type: 'click', at: markerNth(0), shift: true }, { type: 'click', at: markerNth(1), shift: true }, { type: 'key', key: '' }] },
  { name: 'reaction-tool-label-pick', tool: 'reaction', camera: CAM, steps: [{ type: 'click', at: label }] },
  { name: 'shift-background-drag', tool: 'select', camera: CAM,
    steps: [{ type: 'down', at: space, shift: true }, { type: 'move', dx: 40, dy: 40 }, { type: 'move', dx: 240, dy: 200 }, { type: 'up', dx: 240, dy: 200, shift: true }] },
  { name: 'pinch-zoom', tool: 'select', camera: CAM, steps: [{ type: 'pinch', a: { x: 700, y: 500 }, b: { x: 800, y: 500 }, a2: { x: 650, y: 520 }, b2: { x: 870, y: 520 } }] },
  { name: 'touch-pan', tool: 'select', camera: CAM, steps: [{ type: 'move', at: space }, { type: 'touch', path: [[0, 0], [15, 10], [60, 45]] }] },
  { name: 'touch-tap-selects-reaction', tool: 'select', camera: CAM, steps: [{ type: 'move', at: marker }, { type: 'touch', path: [[0, 0]] }] },
];

/** Cases on a map with nothing drawn: the cell and compartment tools. */
const EMPTY_CASES = [
  { name: 'cell-draw', tool: 'cell', camera: { x: 20, y: 20, scale: 0.7 }, steps: drag(medium, [[30, 20], [400, 300]]) },
  { name: 'cell-click-default-size', tool: 'cell', camera: { x: 20, y: 20, scale: 0.7 }, steps: [{ type: 'click', at: medium }] },
];

/* -------------------------------------------------------------------- run */

try {
  await boot();
  const probe = process.argv.indexOf('--eval');
  if (probe > 0) {
    // Look at the page: `--eval '<function body>'` prints what it returns.
    await page(`Object.assign(window.escherMCMC.camera, { x: 40, y: 30, scale: 0.9, fitted: true }); window.escherMCMC.store.emit('map'); return true;`);
    await frames();
    console.log(JSON.stringify(await page(`${HELPERS} ${process.argv[probe + 1]}`), null, 1));
    throw 'done';
  }
  const initial = await page('return window.escherMCMC.store.map.toJSON();');
  const renderInfo = await page(`const s = window.escherMCMC.store; return {
    labels: s.view.labels, showCurrency: s.view.showCurrency, currencyLabels: s.view.currencyLabels,
    fluxes: Object.fromEntries(s.displayFluxes()), solvedFluxes: Object.fromEntries(s.solution?.fluxes ?? []),
    growth: Object.fromEntries(s.growth ?? []),
    metaboliteLabels: Object.fromEntries(s.mapLabels('metabolite')), reactionLabels: Object.fromEntries(s.mapLabels('reaction')),
    textStyle: s.view.textStyle ?? null,
  };`);
  const cases = [];
  for (const c of CASES.filter((c) => c.steps.length)) {
    process.stdout.write(`${c.name} … `);
    cases.push(await runCase(c, initial));
    console.log('ok');
  }
  // The shell: what the cell tool offers, on a blank canvas.
  const empty = await page(`const s = window.escherMCMC.store; const { EditableMap } = await import('/src/map/editable-map.js');
    const blank = EditableMap.blank({ width: s.map.width, height: s.map.height });
    const sp = s.species[0]; const model = s.models.get(sp.file);
    s.map = blank;
    const cells = s.availableMapCells().map((c) => ({ ...c, cellName: model.cell?.name ?? sp.id, organism: model.cell?.organism ?? '', note: model.cell?.note ?? '', color: model.cell?.color ?? '#64748b', abundance: sp.abundance }));
    return { map: blank.toJSON(), cells };`);
  for (const c of EMPTY_CASES) {
    process.stdout.write(`${c.name} … `);
    cases.push(await runCase({ ...c, map: empty.map }, initial));
    console.log('ok');
  }
  // The compartment tool, inside the cell just drawn.
  const shell = await page(`const s = window.escherMCMC.store; const { EditableMap } = await import('/src/map/editable-map.js');
    s.map = EditableMap.fromJSON(arg.map); const sp = s.species[0]; const model = s.models.get(sp.file);
    s.addMapCell(sp.id, { x: 120, y: 90, w: 900, h: 640 });
    const comps = s.availableMapCompartments().map((c) => ({ ...c, label: c.compartmentName, organism: model.cell?.organism ?? '', note: model.cell?.note ?? '', color: model.cell?.color ?? '#64748b', abundance: sp.abundance }));
    return { map: s.map.toJSON(), compartments: comps };`, { map: empty.map });
  const inParent = `({ stage }) => { const r = document.querySelector('.emc-compartment-parent-box').getBoundingClientRect(); return { x: Math.round(r.left + 60), y: Math.round(r.top + 70) }; }`;
  for (const c of [
    { name: 'compartment-draw', tool: 'compartment', camera: { x: 20, y: 20, scale: 0.7 }, steps: drag(inParent, [[30, 30], [300, 220]]) },
    { name: 'compartment-click-hints', tool: 'compartment', camera: { x: 20, y: 20, scale: 0.7 }, steps: [{ type: 'click', at: inParent }] },
    { name: 'cell-tool-membrane-selects-cell', tool: 'cell', camera: { x: 20, y: 20, scale: 0.7 }, steps: [{ type: 'click', at: membraneLeft }] },
    { name: 'cell-tool-geometry-edit-pan', tool: 'cell', camera: { x: 20, y: 20, scale: 0.7 }, steps: [{ type: 'click', at: space }, ...drag(space, [[10, 10], [50, 30]])] },
  ]) {
    process.stdout.write(`${c.name} … `);
    cases.push(await runCase({ ...c, map: shell.map }, initial));
    console.log('ok');
  }
  // Hit-testing: what the browser finds under a grid of points, at three
  // zooms, with a reaction selected (its handles drawn) in Edit.
  const hits = [];
  for (const [camera, tool] of [[CAM, 'select'], [WIDE, 'select'], [MID, 'edit']]) {
    await page(`${HELPERS}
      const { EditableMap } = await import('/src/map/editable-map.js');
      store.map = EditableMap.fromJSON(arg.map); store.mapHistory.clear();
      store.multiSelection = { nodes: new Set(), reactions: new Set(), texts: new Set() };
      document.querySelector('[data-action="tool-' + arg.tool + '"]').click();
      store.select(arg.tool === 'edit' ? 'eco@FBA' : null);
      Object.assign(window.escherMCMC.camera, arg.camera, { fitted: true });
      store.emit('map'); return true;`, { map: initial, tool, camera });
    await frames();
    const grid = await page(`${HELPERS}
      const b = stage.getBoundingClientRect(); const out = [];
      for (let y = Math.ceil(b.top) + 3; y < b.bottom - 3; y += 11) for (let x = Math.ceil(b.left) + 3; x < b.right - 3; x += 11) {
        out.push([x, y, describe(document.elementFromPoint(x, y))]);
      }
      return { stage: { left: b.left, top: b.top, width: b.width, height: b.height }, points: out };`);
    // Each element described once; the points name them by index.
    const seen = new Map(); const targets = [];
    const points = grid.points.map(([x, y, d]) => {
      const key = JSON.stringify(d);
      if (!seen.has(key)) { seen.set(key, targets.length); targets.push(d); }
      return [x, y, seen.get(key)];
    });
    hits.push({ camera, tool, selected: tool === 'edit' ? 'eco@FBA' : null, stage: grid.stage, targets, points });
    console.log(`hits at scale ${camera.scale}: ${grid.points.length} points`);
  }
  await mkdir(outDir, { recursive: true });
  const doc = { viewport: VIEW, initial, render: renderInfo, cells: empty.cells, compartments: shell.compartments, cases };
  await writeFile(join(outDir, 'hits.json'), JSON.stringify({ hits }));
  await writeFile(join(outDir, 'gestures.json'), JSON.stringify(doc));
  console.log(`wrote ${cases.length} cases`);
} catch (error) {
  if (error !== 'done') {
    console.error(error);
    process.exitCode = 1;
  }
} finally {
  await cleanup();
}
