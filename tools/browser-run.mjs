#!/usr/bin/env node
/**
 * Run tools/browser-check.html in real Chromium and report the result.
 *
 * Headless screenshots with --virtual-time-budget capture whenever the budget
 * runs out, which is no use for a page that boots a second application in an
 * iframe. This drives the browser over the DevTools protocol instead, so it
 * waits for the page to actually finish and gets real timings back.
 *
 * Usage: node tools/browser-run.mjs [--url=http://localhost:8080] [--keep]
 */
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const CHROME_CANDIDATES = [
  process.env.CHROME_PATH,
  process.env.EDGE_PATH,
  process.env.BROWSER_PATH,
  ...(process.platform === 'win32' ? [
    join(process.env.PROGRAMFILES || 'C:\\Program Files', 'Google', 'Chrome', 'Application', 'chrome.exe'),
    join(process.env['PROGRAMFILES(X86)'] || 'C:\\Program Files (x86)', 'Google', 'Chrome', 'Application', 'chrome.exe'),
    join(process.env.LOCALAPPDATA || '', 'Google', 'Chrome', 'Application', 'chrome.exe'),
    join(process.env['PROGRAMFILES(X86)'] || 'C:\\Program Files (x86)', 'Microsoft', 'Edge', 'Application', 'msedge.exe'),
    join(process.env.PROGRAMFILES || 'C:\\Program Files', 'Microsoft', 'Edge', 'Application', 'msedge.exe'),
  ] : []),
  '/opt/pw-browsers/chromium-1194/chrome-linux/chrome',
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser',
  '/usr/bin/google-chrome',
];

const args = Object.fromEntries(
  process.argv.slice(2).map((a) => {
    const [k, v = 'true'] = a.replace(/^--/, '').split('=');
    return [k, v];
  }),
);

const base = args.url || 'http://localhost:8080';
const port = Number(args.port || 9333);
const chrome = CHROME_CANDIDATES.find((p) => p && existsSync(p));

if (!chrome) {
  console.error('No Chromium browser found. Set CHROME_PATH or EDGE_PATH to a browser binary.');
  process.exit(2);
}
if (typeof WebSocket === 'undefined') {
  console.error('This needs Node 22+ for the built-in WebSocket client.');
  process.exit(2);
}

// Fail fast and say why. Without this, Chromium happily navigates to a refused
// connection, the page never defines its result, and the run hangs for the full
// five-minute timeout before reporting something unhelpful.
try {
  const probe = await fetch(`${base}/models/index.json`, { signal: AbortSignal.timeout(4000) });
  if (!probe.ok) throw new Error(`HTTP ${probe.status}`);
} catch (error) {
  console.error(`No se pudo alcanzar ${base} (${error.message}).`);
  console.error('Arranca el servidor primero:  node tools/serve.mjs');
  process.exit(2);
}

// A separate profile makes this launch independent of an already open Chrome
// or Edge window. It also avoids writing into the user's browser profile.
const cacheRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '.cache');
await mkdir(cacheRoot, { recursive: true });
const profile = await mkdtemp(join(cacheRoot, 'browser-check-'));
let launchError = null;
const browser = spawn(chrome, [
  '--headless=new',
  '--no-first-run',
  '--no-default-browser-check',
  `--user-data-dir=${profile}`,
  '--window-size=1600,1000',
  '--disable-gpu',
  '--no-sandbox',
  '--disable-dev-shm-usage',
  `--remote-debugging-port=${port}`,
  '--remote-allow-origins=*',
  'about:blank',
], { stdio: ['ignore', 'ignore', 'ignore'], windowsHide: true });
browser.once('error', (error) => { launchError = error; });

let exitCode = 1;
try {
  const target = await openTarget(`${base}/tools/browser-check.html`);
  const result = await drive(target.webSocketDebuggerUrl);
  for (const line of result.lines) console.log(line);
  console.log('');
  console.log(result.failures ? `${result.failures} check(s) failed` : `All ${result.total} checks passed`);
  exitCode = result.failures ? 1 : 0;
} catch (error) {
  console.error(`browser-run failed: ${error.message}`);
  exitCode = 2;
} finally {
  if (!args.keep) {
    browser.kill('SIGTERM');
    // Windows may release the profile files a little after the process exits.
    await Promise.race([
      browser.exitCode !== null ? Promise.resolve() : new Promise((done) => browser.once('exit', done)),
      delay(3000),
    ]);
    if (resolve(profile).startsWith(`${cacheRoot}${sep}`)) {
      await rm(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 150 })
        .catch(() => console.error(`Temporary browser profile could not be removed: ${profile}`));
    }
  }
}
process.exit(exitCode);

// Helpers

async function openTarget(url) {
  // Wait for the debugging endpoint to come up.
  for (let attempt = 0; attempt < 100; attempt++) {
    if (launchError) throw launchError;
    if (browser.exitCode !== null) throw new Error(`Browser exited before opening its debugging port (${browser.exitCode}).`);
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent(url)}`, {
        method: 'PUT',
      });
      if (res.ok) return res.json();
    } catch { /* not listening yet */ }
    await delay(100);
  }
  throw new Error('Chromium never opened its debugging port');
}

function drive(wsUrl) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(wsUrl);
    let nextId = 1;
    const pending = new Map();
    const consoleErrors = [];
    // Raisable for diagnosis: a run that overruns reports nothing at all, so
    // being able to let it finish once is the difference between a timeout and
    // knowing which check is slow. COXETER_CHECK_TIMEOUT is in milliseconds.
    const budget = Number(process.env.COXETER_CHECK_TIMEOUT ?? 300000);
    const timer = setTimeout(() => {
      socket.close();
      reject(new Error(`timed out after ${Math.round(budget / 1000)}s`));
    }, budget);

    const send = (method, params = {}) => new Promise((ok, fail) => {
      const id = nextId++;
      pending.set(id, { ok, fail });
      socket.send(JSON.stringify({ id, method, params }));
    });

    socket.addEventListener('message', (event) => {
      const message = JSON.parse(event.data);
      if (message.method === 'Runtime.exceptionThrown') {
        consoleErrors.push(message.params?.exceptionDetails?.exception?.description
          ?? message.params?.exceptionDetails?.text
          ?? 'Uncaught browser exception');
      }
      if (message.method === 'Runtime.consoleAPICalled' && message.params?.type === 'error') {
        consoleErrors.push((message.params.args ?? []).map((arg) => arg.value ?? arg.description ?? '').join(' '));
      }
      if (message.id && pending.has(message.id)) {
        const { ok, fail } = pending.get(message.id);
        pending.delete(message.id);
        if (message.error) fail(new Error(message.error.message));
        else ok(message.result);
      }
    });

    socket.addEventListener('error', () => reject(new Error('websocket error')));

    socket.addEventListener('open', async () => {
      try {
        await send('Runtime.enable');
        // /json/new can expose the tab before its first navigation finishes.
        // Wait for the check page so its completion promise is not installed
        // into about:blank and destroyed by the following navigation.
        let pageReady = false;
        for (let attempt = 0; attempt < 100; attempt++) {
          try {
            const ready = await send('Runtime.evaluate', {
              expression: "location.pathname.endsWith('/tools/browser-check.html') && document.readyState !== 'loading'",
              returnByValue: true,
            });
            if (ready.result?.value === true) { pageReady = true; break; }
          } catch (error) {
            if (!/context.*destroyed|Cannot find context/i.test(error.message)) throw error;
          }
          await delay(100);
        }
        if (!pageReady) throw new Error('Browser check page did not finish loading');
        // The page signals completion by resolving window.__checkComplete.
        const res = await send('Runtime.evaluate', {
          expression: 'window.__checkComplete ?? new Promise((r) => { window.__onCheckComplete = r; })',
          awaitPromise: true,
          returnByValue: true,
          timeout: Math.max(budget - 20000, 30000),
        });
        clearTimeout(timer);
        socket.close();
        if (res.exceptionDetails) {
          reject(new Error(res.exceptionDetails.exception?.description ?? 'page threw'));
          return;
        }
        const value = res.result.value;
        value.total += 1;
        if (consoleErrors.length) {
          value.failures += 1;
          value.lines.push(`FAIL  consola del navegador sin errores  — ${consoleErrors.join(' | ')}`);
        } else {
          value.lines.push('OK    consola del navegador sin errores');
        }
        resolve(value);
      } catch (error) {
        clearTimeout(timer);
        socket.close();
        reject(error);
      }
    });
  });
}
