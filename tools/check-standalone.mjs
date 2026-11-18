#!/usr/bin/env node
/**
 * Smoke-check the single-file build.
 *
 * coxeter_mfv.html is the way most people will open this project, and nothing
 * else verifies it: `npm run check:browser` drives tools/browser-check.html,
 * which imports the modules from src/ over the dev server. The standalone inlines
 * everything instead, over file://, including GLPK, its WASM and the complete
 * SteadyCom models. Those are different enough conditions
 * that the build can break while every other check passes.
 *
 *   node tools/check-standalone.mjs [--file=coxeter_mfv.html] [--port=9334]
 */
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { closeBrowser } from './lib/chromium.mjs';

/**
 * How many times, 250 ms apart, to ask Chromium for its first page: 30 s in all.
 * A cold Chromium on a shared CI runner has taken more than 10 s to open it.
 */
const CHROME_START_POLLS = 120;

const CHROME_CANDIDATES = [
  process.env.CHROME_PATH,
  process.env.EDGE_PATH,
  ...(process.platform === 'win32' ? [
    join(process.env.PROGRAMFILES || 'C:\\Program Files', 'Google', 'Chrome', 'Application', 'chrome.exe'),
    join(process.env['PROGRAMFILES(X86)'] || 'C:\\Program Files (x86)', 'Microsoft', 'Edge', 'Application', 'msedge.exe'),
  ] : []),
  '/opt/pw-browsers/chromium',
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser',
  '/usr/bin/google-chrome',
];

const args = Object.fromEntries(process.argv.slice(2).map((a) => {
  const [k, v = 'true'] = a.replace(/^--/, '').split('=');
  return [k, v];
}));

const file = resolve(args.file ?? 'coxeter_mfv.html');
const port = Number(args.port ?? 9334);
const chrome = CHROME_CANDIDATES.find((p) => p && existsSync(p));

if (!existsSync(file)) {
  console.error(`${file} does not exist. Build it first: npm run build:standalone`);
  process.exit(2);
}
if (!chrome) {
  console.error('No Chromium found. Set CHROME_PATH to a browser binary.');
  process.exit(2);
}

// --allow-file-access-from-files is what lets the inlined module graph load over
// file://, which is the whole point of the single-file build.
const cacheRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '.cache');
await mkdir(cacheRoot, { recursive: true });
const profile = await mkdtemp(join(cacheRoot, 'standalone-check-'));
const browser = spawn(chrome, [
  '--headless=new', '--disable-gpu', '--no-sandbox', '--disable-dev-shm-usage',
  '--no-first-run', '--no-default-browser-check', `--user-data-dir=${profile}`,
  '--allow-file-access-from-files',
  `--remote-debugging-port=${port}`, '--remote-allow-origins=*',
  'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'], windowsHide: true });

let failures = 0;
const check = (name, ok, detail = '') => {
  if (!ok) failures += 1;
  console.log(`${ok ? 'OK  ' : 'FAIL'}  ${name}${detail ? `  — ${detail}` : ''}`);
};

try {
  const target = await openTarget(port);
  const result = await drive(target.webSocketDebuggerUrl, pathToFileURL(file).href);

  check('the single file starts the application', result.started,
    result.started ? `${result.nodes} nodes, µ = ${result.growth}` : result.error ?? 'timed out');
  if (!result.started) {
    for (const line of result.pageLog.slice(0, 6)) console.log(`      ${line}`);
  }
  check('it solves without a server, with the embedded GLPK', result.status === 'optimal' && result.backend === 'glpk',
    `backend ${result.backend}, status ${result.status}`);
  check('the single card offers Scenario and Objective', result.mergedCard,
    result.mergedCard ? 'two columns' : 'no .sc-grid with both columns');
  check('it bundles the four published communities', result.publishedCount === 4,
    `${result.publishedCount}`);
  check('the case studies head the scenario list', result.caseStudiesFirst,
    result.optionGroups);
  check('it shows the reproduced SteadyCom µ', result.showsGrowthRate, '0.73599');
  // The single-file build embeds each member's map excerpt. If that embedding is
  // missing the detail still renders and only the map is empty, so it is checked
  // separately from the text.
  check('it draws the published community map from the embedded excerpts',
    result.publishedNodes > 0, `${result.publishedNodes} nodes`);
  // The model buttons must be enabled in this build too, or nothing can be
  // downloaded from the single file at all.
  const modelPath = result.modelHref.startsWith('file://')
    ? fileURLToPath(result.modelHref) : '';
  check('the JSON downloads use the complete embedded models',
    result.jsonButtons === result.speciesCount && result.fullModels === 13,
    `${result.jsonButtons} buttons, ${result.fullModels} complete models`);
  check('it includes the CSV files and the SteadyCom worker for offline use',
    result.textInputs === 6 && result.workerEmbedded && result.csvButtons === 2,
    `${result.textInputs} CSV, ${result.csvButtons} downloads`);
  check('each species also offers its model in SBML',
    result.sbmlButtons > 0 && result.sbmlButtons === result.speciesCount,
    `${result.sbmlButtons} buttons for ${result.speciesCount} species`);
  check('Scenario → Return to map keeps the selected community',
    result.returnedToMap && result.returnedScenario && result.returnedNodes > 0,
    `returned=${result.returnedToMap}, scenario=${result.returnedScenario}, nodes=${result.returnedNodes}`);
} catch (error) {
  failures += 1;
  console.error(`EXCEPTION: ${error.message}`);
} finally {
  await closeBrowser(port);
  browser.kill();
  await Promise.race([
    browser.exitCode !== null ? Promise.resolve() : new Promise(done => browser.once('exit', done)),
    delay(3000),
  ]);
  if (resolve(profile).startsWith(`${cacheRoot}${sep}`)) {
    await rm(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 150 })
      .catch(() => console.error(`Temporary browser profile could not be removed: ${profile}`));
  }
}

console.log(failures ? `\n${failures} check(s) failed` : '\nAll checks passed');
process.exit(failures ? 1 : 0);

async function openTarget(debugPort) {
  for (let i = 0; i < CHROME_START_POLLS; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${debugPort}/json/list`);
      const list = await res.json();
      const page = list.find((t) => t.type === 'page');
      if (page) return page;
    } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error('Chromium never exposed a page over the DevTools protocol');
}

function drive(wsUrl, url) {
  return new Promise((resolve_, reject) => {
    const socket = new WebSocket(wsUrl);
    const pending = new Map();
    let id = 0;
    const timer = setTimeout(() => { socket.close(); reject(new Error('timed out')); }, 120000);

    const send = (method, params = {}) => new Promise((ok, fail) => {
      id += 1;
      pending.set(id, { ok, fail });
      socket.send(JSON.stringify({ id, method, params }));
    });

    // Keep whatever the page complains about. A single-file build that fails to
    // start says nothing useful through its own API, so the console is the only
    // place the reason shows up.
    const pageLog = [];
    socket.addEventListener('message', (event) => {
      const message = JSON.parse(event.data);
      if (message.method === 'Runtime.exceptionThrown') {
        const d = message.params.exceptionDetails;
        pageLog.push(`exception: ${d.exception?.description ?? d.text}`);
        return;
      }
      if (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') {
        pageLog.push(`console: ${message.params.args.map((a) => a.value ?? a.description).join(' ')}`);
        return;
      }
      const waiter = message.id && pending.get(message.id);
      if (!waiter) return;
      pending.delete(message.id);
      if (message.error) waiter.fail(new Error(message.error.message));
      else waiter.ok(message.result);
    });
    socket.addEventListener('error', () => reject(new Error('websocket error')));

    socket.addEventListener('open', async () => {
      try {
        await send('Runtime.enable');
        await send('Page.enable');
        await send('Page.navigate', { url });
        const res = await send('Runtime.evaluate', {
          expression: probeSource(),
          awaitPromise: true,
          returnByValue: true,
          timeout: 110000,
        });
        clearTimeout(timer);
        socket.close();
        if (res.exceptionDetails) {
          reject(new Error(res.exceptionDetails.exception?.description ?? 'page threw'));
          return;
        }
        resolve_({ ...res.result.value, pageLog });
      } catch (error) {
        clearTimeout(timer);
        socket.close();
        reject(error);
      }
    });
  });
}

/** Runs inside the page: wait for the app, then report what the merged scenario card shows. */
function probeSource() {
  return `(async () => {
  const until = async (fn, ms) => {
    const deadline = Date.now() + ms;
    while (Date.now() < deadline) {
      try { const v = fn(); if (v) return v; } catch {}
      await new Promise((r) => setTimeout(r, 120));
    }
    return null;
  };
  const app = await until(() => window.escherMCMC?.store?.layout && window.escherMCMC?.store?.solution && window.escherMCMC, 60000);
  if (!app) return { started: false, error: 'window.escherMCMC never appeared' };

  const store = app.store;
  document.querySelector('[data-action="left-tab"][data-tab="model"]')?.click();
  document.querySelector('#task-panel [data-workspace="scenario"]')?.click();
  await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
  const nodes = store.layout.nodes.size;
  const growth = store.solution?.objectiveValue?.toFixed(5) ?? null;
  const status = store.solution?.status ?? null;
  const backend = store.problem?.backend?.() ?? 'unknown';

  const card = () => document.querySelector('.card.sc-card');
  const select = () => card()?.querySelector('select[data-action="scenario"]');
  const groups = [...(select()?.querySelectorAll('optgroup') ?? [])].map((g) => g.label);

  // Scenario and Objective now share one card; the published communities head the
  // same list as the teaching ones.
  const mergedCard = Boolean(card()?.querySelector('.sc-grid .sc-scenario h2')
    && card()?.querySelector('.sc-grid .sc-objective'));

  const target = (store.publishedCommunities ?? [])
    .find((c) => c.reproduction?.expected?.growthRate);
  let showsGrowthRate = false;
  let publishedNodes = 0;
  let modelHref = '';
  let sbmlButtons = 0;
  let speciesCount = 0;
  let jsonButtons = 0;
  let csvButtons = 0;
  let returnedToMap = false;
  let returnedScenario = false;
  let returnedNodes = 0;
  if (target && select()) {
    // Re-query after each repaint: the card is rebuilt and the old handle is detached.
    select().value = 'pub:' + target.id;
    select().dispatchEvent(new Event('change', { bubbles: true }));
    await until(() => store.scenario?.published === target.id
      && card()?.querySelector('.pc-detail'), 60000);
    await until(() => store.layout?.nodes?.size > 0, 60000);
    await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
    showsGrowthRate = Boolean(card()?.textContent.includes('0.73599'));
    publishedNodes = store.layout?.nodes?.size ?? 0;
    // The download is a plain link on purpose: this build cannot fetch the
    // files beside it, but the browser can still follow a relative href to
    // one. What matters is that the href points next to the page.
    const link = card()?.querySelector('.pc-species a.pc-dl[download]');
    modelHref = link ? new URL(link.getAttribute('href'), document.baseURI).href : '';
    speciesCount = target.species?.length ?? 0;
    jsonButtons = card()?.querySelectorAll('button[data-action="download-member-json"]').length ?? 0;
    csvButtons = card()?.querySelectorAll('button[data-action="download-community-csv"]').length ?? 0;
    sbmlButtons = card()?.querySelectorAll('button[data-action="download-member-sbml"]').length ?? 0;

    // Reproduce the user path exactly: commit the selected community with
    // Return to map, then verify that the active map still belongs to it.
    document.querySelector('#workspace-editor [data-action="close-workspace"]')?.click();
    returnedToMap = Boolean(await until(
      () => document.querySelector('#workspace-editor')?.hidden === true,
      60000
    ));
    returnedScenario = store.scenario?.published === target.id;
    returnedNodes = store.layout?.nodes?.size ?? 0;
  }

  return {
    started: true,
    nodes,
    growth,
    status,
    backend,
    mergedCard,
    publishedCount: store.publishedCommunities?.length ?? 0,
    caseStudiesFirst: groups[0] === 'Explore Coxeter’s capabilities',
    optionGroups: groups.join(' | '),
    showsGrowthRate,
    publishedNodes,
    modelHref,
    sbmlButtons,
    speciesCount,
    jsonButtons,
    csvButtons,
    fullModels: Object.keys(globalThis.__ESCHER_MCMC_DATA__ ?? {}).filter(k => /steadycom-.+\\/model-/.test(k)).length,
    textInputs: Object.keys(globalThis.__ESCHER_MCMC_TEXT__ ?? {}).length,
    workerEmbedded: typeof globalThis.__COXETER_STEADYCOM_WORKER_SOURCE__ === 'string',
    returnedToMap,
    returnedScenario,
    returnedNodes,
  };
})()`;
}
