#!/usr/bin/env node
/**
 * Measure what the browser spends showing big maps: the first paint, a full
 * repaint, the frames of a node drag and of a zoom. Companion to
 * profile-heavy.mjs, which times the same models in Node without a DOM.
 *
 *   node tools/serve.mjs            # in another terminal
 *   node tools/profile-render.mjs [--url=http://localhost:8080] [--cases=core,iaf1260,ecoli4,ecoli4-full]
 *       [--angle=d3d11|metal|vulkan]
 *
 * CHROME_PATH picks the browser; otherwise Playwright's headless shell is used
 * when installed. ecoli4-full runs the automatic layout on 9,831 reactions in the
 * page, which takes about five minutes.
 *
 * Drives headless Chromium over the DevTools protocol. Real mouse and wheel
 * events go through the app's own handlers; frame times come from a
 * requestAnimationFrame loop in the page and main-thread time from
 * Performance.getMetrics.
 */
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const args = Object.fromEntries(process.argv.slice(2).map((a) => {
  const [k, v = 'true'] = a.replace(/^--/, '').split('=');
  return [k, v];
}));
const base = args.url || 'http://localhost:8080';
const port = Number(args.port || 9334);
const cases = (args.cases || 'core,iaf1260,ecoli4').split(',');
const chrome = [
  process.env.CHROME_PATH,
  join(process.env.LOCALAPPDATA || '', 'ms-playwright', 'chromium_headless_shell-1234', 'chrome-headless-shell-win64', 'chrome-headless-shell.exe'),
  '/usr/bin/chromium',
].find((p) => p && existsSync(p));
if (!chrome) throw new Error('No Chromium found; set CHROME_PATH.');

const cacheRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '.cache');
await mkdir(cacheRoot, { recursive: true });
const profile = await mkdtemp(join(cacheRoot, 'profile-render-'));
// Headless Chromium falls back to software rendering (SwiftShader) unless told
// which GPU backend to use. The report prints which one it got.
const angle = args.angle ?? { win32: 'd3d11', darwin: 'metal' }[process.platform];
const browser = spawn(chrome, [
  '--headless=new', '--no-first-run', '--no-default-browser-check',
  `--user-data-dir=${profile}`, '--window-size=1600,1000',
  '--enable-gpu-rasterization', '--ignore-gpu-blocklist',
  ...(angle ? ['--use-gl=angle', `--use-angle=${angle}`] : []),
  `--remote-debugging-port=${port}`, '--remote-allow-origins=*', 'about:blank',
], { stdio: 'ignore', windowsHide: true });

// What runs in the page: a frame recorder and a repaint timer.
const PAGE_HELPERS = `(() => {
  const app = window.escherMCMC;
  const store = app.store;
  const nextFrame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
  const median = (xs) => { const s = [...xs].sort((a, b) => a - b); return s.length ? s[Math.floor(s.length / 2)] : 0; };
  const pct = (xs, p) => { const s = [...xs].sort((a, b) => a - b); return s.length ? s[Math.min(s.length - 1, Math.floor(s.length * p))] : 0; };
  const r1 = (x) => Math.round(x * 10) / 10;
  let rec = null;
  window.__prof = {
    nextFrame,
    async load(kind) {
      app.camera.fitted = false;
      const t0 = performance.now();
      if (kind === 'core') await store.useScenario('consorcio-anaerobio');
      if (kind === 'iaf1260') {
        const { Model } = await import('/src/core/model.js');
        const doc = await (await fetch('/models/published/steadycom-ecoli-auxotrophs/model-Ec1.json')).json();
        store.models.set('profile-Ec1', new Model(doc));
        if (!store.scenarios.some((s) => s.id === 'profile-iaf1260')) {
          store.scenarios.push({ id: 'profile-iaf1260', name: 'iAF1260', species: [{ id: 'Ec1', model: 'profile-Ec1', abundance: 1 }], medium: {} });
        }
        await store.useScenario('profile-iaf1260');
      }
      if (kind === 'ecoli4') await store.usePublishedCommunity('steadycom-ecoli-auxotrophs');
      if (kind === 'ecoli4-full') { store.showFullMap(); }
      const t1 = performance.now();
      await nextFrame();
      return { loadMs: r1(t1 - t0), firstFrameMs: r1(performance.now() - t0), layoutMs: store.status.layoutMs, solveMs: store.status.solveMs, ...this.stats() };
    },
    stats() {
      const svg = document.querySelector('#stage svg');
      return {
        reactions: store.map?.reactions.size ?? 0,
        svgElements: svg ? svg.getElementsByTagName('*').length : 0,
        svgKB: svg ? Math.round(svg.outerHTML.length / 1024) : 0,
        canvas: store.map ? Math.round(store.map.width) + 'x' + Math.round(store.map.height) : '-',
        zoom: r1(app.camera.scale * 1000) / 1000,
      };
    },
    async repaint(reason, n) {
      const sync = [], frame = [];
      for (let i = 0; i < n; i++) {
        const t0 = performance.now();
        store.emit(reason);
        sync.push(performance.now() - t0);
        await nextFrame();
        frame.push(performance.now() - t0);
      }
      return { syncMs: r1(median(sync)), frameMs: r1(median(frame)) };
    },
    startFrames() {
      rec = { t: [], on: true };
      const loop = (ts) => { if (!rec.on) return; rec.t.push(ts); requestAnimationFrame(loop); };
      requestAnimationFrame(loop);
    },
    stopFrames() {
      rec.on = false;
      const d = [];
      for (let i = 1; i < rec.t.length; i++) d.push(rec.t[i] - rec.t[i - 1]);
      return { frames: d.length, medianMs: r1(median(d)), p95Ms: r1(pct(d, 0.95)), maxMs: r1(Math.max(0, ...d)), fps: r1(d.length ? 1000 / median(d) : 0) };
    },
    async nodeOnScreen() {
      // Drag at 100 % zoom, as one would: centre the camera on a drawn metabolite,
      // then take a node in view whose hit circle is what the pointer would land on.
      const box = document.getElementById('stage').getBoundingClientRect();
      const target = [...store.layout.nodes.values()].find((n) => n.kind !== 'secondary' && !n.secondary);
      if (!target) return null;
      Object.assign(app.camera, { scale: 1, x: box.width / 2 - target.x, y: box.height / 2 - target.y, fitted: true });
      store.emit('selection');
      await nextFrame();
      for (const el of document.querySelectorAll('#stage circle.emc-hit-node[data-node]')) {
        const r = el.getBoundingClientRect();
        const x = r.left + r.width / 2, y = r.top + r.height / 2;
        if (x < box.left + 100 || x > box.right - 200 || y < box.top + 100 || y > box.bottom - 150) continue;
        if (document.elementFromPoint(x, y)?.closest('[data-node]')?.dataset.node === el.dataset.node) {
          return { x: Math.round(x), y: Math.round(y), id: el.dataset.node };
        }
      }
      return null;
    },
    stageCenter() {
      const b = document.getElementById('stage').getBoundingClientRect();
      return { x: b.left + b.width / 2, y: b.top + b.height / 2 };
    },
    gesture() { const g = app.gestures(); return Object.entries(g).filter(([, v]) => v && typeof v === 'object').map(([k]) => k).join(',') || null; },
    gpu() {
      const gl = document.createElement('canvas').getContext('webgl');
      const ext = gl?.getExtension('WEBGL_debug_renderer_info');
      return ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : 'unknown';
    },
  };
  return true;
})()`;

let exitCode = 0;
try {
  const target = await openTarget(`${base}/`);
  const cdp = await connect(target.webSocketDebuggerUrl);
  await cdp.send('Runtime.enable');
  await cdp.send('Performance.enable');
  await waitFor(cdp, "Boolean(window.escherMCMC?.store?.map && document.querySelector('#stage svg'))", 60000);
  await evaluate(cdp, PAGE_HELPERS);
  console.log(`GPU: ${await evaluate(cdp, '__prof.gpu()')} · ${await evaluate(cdp, 'navigator.userAgent.match(/Chrome\\/[\\d.]+/)?.[0]')}`);

  for (const kind of cases) {
    console.log(`\n== ${kind}`);
    const before = await metrics(cdp);
    const loaded = await evaluate(cdp, `__prof.load(${JSON.stringify(kind)})`, 1800000);
    const after = await metrics(cdp);
    print('open/draw', { ...loaded, ...mainThread(before, after) });

    print("repaint (store.emit 'map')", await evaluate(cdp, "__prof.repaint('map', 3)", 600000));
    print("repaint (store.emit 'selection')", await evaluate(cdp, "__prof.repaint('selection', 3)", 600000));

    // A node drag: every move schedules a full paintMap in the next frame.
    const node = await evaluate(cdp, '__prof.nodeOnScreen()');
    if (node) print('  zoomed to 100 % on', node);
    else print('  no node under the pointer', {});
    if (node) {
      const m0 = await metrics(cdp);
      await evaluate(cdp, '__prof.startFrames()');
      await mouse(cdp, 'mousePressed', node.x, node.y, 1);
      for (let i = 1; i <= 30; i++) {
        await mouse(cdp, 'mouseMoved', node.x + i * 3, node.y + i * 2, 1);
        if (i === 3) print('  drag gesture', { gesture: await evaluate(cdp, '__prof.gesture()') });
        await delay(16);
      }
      await mouse(cdp, 'mouseReleased', node.x + 90, node.y + 60, 0);
      await delay(300);
      const frames = await evaluate(cdp, '__prof.stopFrames()');
      print('drag a node (30 moves)', { ...frames, ...mainThread(m0, await metrics(cdp)) });
      // void: the undo result is the whole map, far too much to send back.
      await evaluate(cdp, 'void escherMCMC.store.undoMap?.()');
      await evaluate(cdp, "(escherMCMC.camera.fitted = false, escherMCMC.store.emit('selection'), __prof.nextFrame())");
    } else {
      print('drag a node', { skipped: 'no node in view' });
    }

    // Zoom: the camera is a CSS transform, but the label scale variable restyles every label.
    const c = await evaluate(cdp, '__prof.stageCenter()');
    const z0 = await metrics(cdp);
    await evaluate(cdp, '__prof.startFrames()');
    for (let i = 0; i < 12; i++) {
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: c.x, y: c.y, deltaX: 0, deltaY: i < 6 ? -120 : 120 });
      await delay(40);
    }
    await delay(300);
    print('zoom (12 wheel ticks)', { ...await evaluate(cdp, '__prof.stopFrames()'), ...mainThread(z0, await metrics(cdp)) });
  }
  cdp.close();
} catch (error) {
  console.error(`profile-render failed: ${error.stack ?? error.message}`);
  exitCode = 2;
} finally {
  browser.kill('SIGTERM');
  await delay(1000);
  await rm(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 150 }).catch(() => {});
}
process.exit(exitCode);

// Helpers

function print(label, row) {
  console.log(`${label.padEnd(36)} ${Object.entries(row).map(([k, v]) => `${k}=${v}`).join('  ')}`);
}

async function metrics(cdp) {
  const { metrics: list } = await cdp.send('Performance.getMetrics');
  return Object.fromEntries(list.map(({ name, value }) => [name, value]));
}

function mainThread(a, b) {
  const ms = (k) => Math.round((b[k] - a[k]) * 1000);
  return { script: ms('ScriptDuration'), style: ms('RecalcStyleDuration'), layout: ms('LayoutDuration'), task: ms('TaskDuration') };
}

function mouse(cdp, type, x, y, buttons) {
  return cdp.send('Input.dispatchMouseEvent', { type, x, y, button: 'left', buttons, clickCount: type === 'mouseMoved' ? 0 : 1 });
}

async function evaluate(cdp, expression, timeout = 120000) {
  const res = await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, timeout });
  if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description ?? res.exceptionDetails.text);
  return res.result.value;
}

async function waitFor(cdp, expression, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try { if (await evaluate(cdp, expression)) return; } catch { /* page still loading */ }
    await delay(200);
  }
  throw new Error(`timed out waiting for ${expression}`);
}

async function openTarget(url) {
  for (let attempt = 0; attempt < 100; attempt++) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent(url)}`, { method: 'PUT' });
      if (res.ok) return res.json();
    } catch { /* not listening yet */ }
    await delay(100);
  }
  throw new Error('Chromium never opened its debugging port');
}

function connect(wsUrl) {
  return new Promise((resolveSocket, reject) => {
    const socket = new WebSocket(wsUrl);
    let nextId = 1;
    const pending = new Map();
    socket.addEventListener('message', (event) => {
      const message = JSON.parse(event.data);
      if (message.id && pending.has(message.id)) {
        const { ok, fail } = pending.get(message.id);
        pending.delete(message.id);
        if (message.error) fail(new Error(message.error.message));
        else ok(message.result);
      }
    });
    socket.addEventListener('error', () => reject(new Error('websocket error')));
    socket.addEventListener('open', () => resolveSocket({
      send: (method, params = {}) => new Promise((ok, fail) => {
        const id = nextId++;
        pending.set(id, { ok, fail });
        socket.send(JSON.stringify({ id, method, params }));
      }),
      close: () => socket.close(),
    }));
  });
}
