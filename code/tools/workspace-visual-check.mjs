#!/usr/bin/env node
/**
 * Browser regression checks for the workspace shell. Requires a running server
 * and Playwright, either installed locally or supplied with --playwright=PATH.
 *
 * node tools/workspace-visual-check.mjs --url=http://localhost:8087 \
 *   --playwright=/path/to/node_modules/playwright --out=.cache/workspace-visual
 *
 * This tests geometry and navigation in the real browser. Screenshots are
 * evidence for visual review, not automatically approved golden images.
 * Add --functional to also run the control, draft and first-click regressions
 * and the scientific-card integration checks.
 */
import { existsSync } from 'node:fs';
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { join, resolve } from 'node:path';

const require = createRequire(import.meta.url);
const args = Object.fromEntries(process.argv.slice(2).map((arg) => {
  const i = arg.indexOf('=');
  return i < 0 ? [arg.replace(/^--/, ''), 'true'] : [arg.slice(0, i).replace(/^--/, ''), arg.slice(i + 1)];
}));
const { chromium } = require(args.playwright || process.env.PLAYWRIGHT_PATH || 'playwright');
const base = args.url || 'http://localhost:8087';
const out = resolve(args.out || '.cache/workspace-visual');
const browserPath = [
  args.browser, process.env.CHROME_PATH, process.env.EDGE_PATH,
  ...(process.platform === 'win32' ? [
    join(process.env.PROGRAMFILES || 'C:\\Program Files', 'Google', 'Chrome', 'Application', 'chrome.exe'),
    join(process.env['PROGRAMFILES(X86)'] || 'C:\\Program Files (x86)', 'Microsoft', 'Edge', 'Application', 'msedge.exe'),
  ] : []),
].find((path) => path && existsSync(path));
const viewports = [
  { width: 1366, height: 768, name: '1366x768' },
  { width: 1280, height: 720, name: '1280x720' },
  { width: 1024, height: 768, name: '1024x768' },
  { width: 900, height: 600, name: '900x600' },
  { width: 390, height: 844, name: '390x844' },
  // Reflow equivalent of a 1366x768 window at 200% browser zoom. This catches
  // the layout constraint. It does not claim to emulate font-only zoom.
  { width: 683, height: 384, name: '1366x768-reflow200' },
].filter((v) => !args.only || args.only.split(',').includes(v.name));
const themes = (args.themes || 'light,dark').split(',');
const report = { startedAt: new Date().toISOString(), base, checks: [], screenshots: [], runtimeErrors: [] };
const check = (name, pass, details = null) => {
  report.checks.push({ name, pass: Boolean(pass), details });
  if (!pass) console.error(`FAIL ${name}${details ? `: ${JSON.stringify(details)}` : ''}`);
};
await mkdir(out, { recursive: true });
const response = await fetch(`${base}/models/index.json`, { signal: AbortSignal.timeout(5000) });
if (!response.ok) throw new Error(`Server is not ready at ${base} (HTTP ${response.status}).`);
const browser = await chromium.launch({ headless: true, ...(browserPath ? { executablePath: browserPath } : {}) });

try {
  for (const viewport of viewports) {
    for (const theme of themes) {
      const key = `${viewport.name}-${theme}`;
      const context = await browser.newContext({ viewport: { width: viewport.width, height: viewport.height }, reducedMotion: 'reduce' });
      const page = await context.newPage();
      page.setDefaultTimeout(10000);
      page.on('pageerror', (error) => report.runtimeErrors.push({ key, message: error.message }));
      page.on('console', (message) => { if (message.type() === 'error') report.runtimeErrors.push({ key, message: message.text() }); });
      try {
        await page.goto(`${base}/index.html?scenario=consorcio-anaerobio`, { waitUntil: 'networkidle', timeout: 45000 });
        await page.waitForFunction(() => window.escherMCMC?.store?.layout && window.escherMCMC.store.solution, null, { timeout: 30000 });
        await settle(page);
        const initialTheme = await page.evaluate(() => document.documentElement.dataset.theme);
        if (initialTheme !== theme) await action(page, 'toggle-theme');
        await settle(page);
        check(`${key}: theme updates HTML, Store and SVG together`, await page.evaluate((requested) => {
          return document.documentElement.dataset.theme === requested
            && window.escherMCMC.store.view.dark === (requested === 'dark')
            && document.querySelector('#stage > svg')?.classList.contains('emc-dark') === (requested === 'dark');
        }, theme));
        const scientificBefore = await scienceSnapshot(page);
        await audit(page, key, 'home');
        await screenshot(page, key, 'home');

        await ensureTasksVisible(page);
        await action(page, 'toggle-focus');
        await settle(page);
        check(`${key}: focus mode keeps Tools and Inspector controls available`, await page.locator('.canvas-toolbar [data-action="toggle-left"]').isVisible() && await page.locator('.canvas-toolbar [data-action="toggle-inspector"]').isVisible());
        await ensureTasksVisible(page);
        await audit(page, key, 'focus-tools', { contrast: false });
        await screenshot(page, key, 'focus-tools');
        await action(page, 'toggle-focus');
        await settle(page);

        // Contextual help can wrap differently for each editing tool. Test
        // all of them, including the model and structure routes they open.
        for (const tool of ['select', 'reaction', 'metabolite', 'text', 'compartment', 'cell']) {
          await ensureTasksVisible(page);
          await page.locator('#left-panel [data-action="left-tab"][data-tab="build"]').click();
          await page.locator(`#left-panel [data-tool="${tool}"]`).click();
          await settle(page);
          await ensureTasksVisible(page);
          await audit(page, key, `tool-${tool}`, { contrast: false });
          if (await page.locator('#workspace-editor').isVisible()) {
            // The drawer can cover the editor's close action on small screens.
            await page.keyboard.press('Escape');
            await settle(page);
            if (await page.locator('#workspace-editor').isVisible()) await action(page, 'close-workspace');
          }
        }
        await ensureTasksVisible(page);
        await page.locator('#left-panel [data-action="left-tab"][data-tab="build"]').click();
        await page.locator('#left-panel [data-tool="select"]').click();
        await settle(page);

        // Exercise every navigation tab and every reachable workspace route.
        // This catches an overflowing task hub hidden by overflow:hidden.
        await ensureTasksVisible(page);
        const taskTabs = await page.locator('#left-panel [data-action="left-tab"]').evaluateAll((nodes) => nodes.map((node) => node.dataset.tab));
        const workspaces = new Set();
        for (const tab of taskTabs) {
          await page.locator(`#left-panel [data-action="left-tab"][data-tab="${tab}"]`).click();
          await settle(page);
          check(`${key}: ${tab} navigation exposes its current state`, await page.locator(`#left-panel [data-action="left-tab"][data-tab="${tab}"]`).getAttribute('aria-current') === 'page');
          await audit(page, key, `tab-${tab}`, { contrast: false });
          const names = await page.locator('#task-panel [data-action="open-workspace"]').evaluateAll((nodes) => nodes.filter((node) => node.getClientRects().length).map((node) => node.dataset.workspace));
          names.forEach((name) => workspaces.add(name));
        }
        check(`${key}: all three task areas remain reachable`, ['build', 'model', 'analysis'].every((tab) => taskTabs.includes(tab)), taskTabs);
        for (const workspace of workspaces) {
          await ensureTasksVisible(page);
          const button = page.locator(`[data-action="open-workspace"][data-workspace="${workspace}"]`).filter({ visible: true }).first();
          if (!(await button.count())) {
            const tabs = page.locator('#left-panel [data-action="left-tab"]');
            for (let i = 0; i < await tabs.count(); i += 1) {
              await tabs.nth(i).click();
              await settle(page);
              if (await page.locator(`[data-action="open-workspace"][data-workspace="${workspace}"]`).filter({ visible: true }).count()) break;
            }
          }
          const route = page.locator(`[data-action="open-workspace"][data-workspace="${workspace}"]`).filter({ visible: true }).first();
          await route.click();
          await settle(page);
          check(`${key}: ${workspace} opens in the central editor`, await page.locator('#workspace-editor').isVisible());
          await audit(page, key, `editor-${workspace}`, { contrast: false });
          if (workspace === 'medium' || workspace === 'scenario') await screenshot(page, key, `editor-${workspace}`);
          if (workspace === 'model') {
            await action(page, 'new-reaction');
            await settle(page);
            await page.locator('#sidebar input[name="id"]').fill('QA_NEW_RXN');
            await page.locator('#sidebar textarea[name="stoichiometry"]').fill('pyr_c:-1, lac__L_c:1');
            await screenshot(page, key, 'editor-model');
          }
          await action(page, 'close-workspace');
          await settle(page);
          check(`${key}: ${workspace} returns to the map`, !(await page.locator('#workspace-editor').isVisible()));
          check(`${key}: ${workspace} returns keyboard focus to a visible control`, await page.evaluate(() => {
            const node = document.activeElement;
            return node && node !== document.body && node.isConnected && node.getClientRects().length > 0;
          }));
        }
        await ensureTasksVisible(page);
        await page.locator('#left-panel [data-action="left-tab"][data-tab="model"]').click();
        await page.locator('#task-panel [data-action="open-workspace"][data-workspace="model"]').click();
        await settle(page);
        check(`${key}: unfinished model form survives visiting other workspaces`, await page.locator('#sidebar input[name="id"]').inputValue() === 'QA_NEW_RXN'
          && await page.locator('#sidebar textarea[name="stoichiometry"]').inputValue() === 'pyr_c:-1, lac__L_c:1');
        await action(page, 'close-workspace');
        await settle(page);

        // Select a real result through the existing UI event route. The
        // selection itself is independent of the map's fitted zoom.
        const resultLink = page.locator('#results-content [data-action="select-result-reaction"]').first();
        if (await resultLink.count()) await resultLink.evaluate((node) => node.click());
        else await page.evaluate(() => {
          const store = window.escherMCMC.store;
          const rid = store.layout.reactions?.keys ? store.layout.reactions.keys().next().value : store.community.merged.reactions.keys().next().value;
          store.select(rid);
        });
        await settle(page);
        const boundInput = page.locator('#inspector [data-action="bound-upper-number"]');
        if (await boundInput.count()) await boundInput.scrollIntoViewIfNeeded();
        check(`${key}: selected reaction has an accessible inspector`, await page.locator('#inspector-panel').isVisible() && await boundInput.isVisible(), await page.evaluate(() => {
          const input = document.querySelector('#inspector [data-action="bound-upper-number"]');
          const panel = document.getElementById('inspector-panel');
          const box = input?.getBoundingClientRect();
          return { selected: window.escherMCMC.store.selectedReaction, panelWidth: panel.getBoundingClientRect().width, input: input ? { x: box.x, y: box.y, width: box.width, height: box.height, display: getComputedStyle(input).display, visibility: getComputedStyle(input).visibility } : null };
        }));
        await page.locator('#inspector').evaluate((node) => { node.scrollTop = 0; });
        await audit(page, key, 'inspector');
        await screenshot(page, key, 'inspector');
        await action(page, 'clear-selection');
        await settle(page);

        const view = page.locator('details.canvas-view');
        check(`${key}: View menu remains reachable`, await view.locator('summary').isVisible());
        await view.locator('summary').click();
        await settle(page);
        check(`${key}: View controls open`, await page.locator('#view-controls').isVisible());
        await audit(page, key, 'view');
        await screenshot(page, key, 'view');
        await view.locator('summary').click();
        await settle(page);

        const cameraBefore = await cameraSnapshot(page);
        await ensureTasksVisible(page);
        await page.locator('#left-panel [data-action="left-tab"][data-tab="analysis"]').click();
        await settle(page);
        await action(page, 'show-results');
        await settle(page);
        if (!(await page.locator('#app-shell').evaluate((node) => node.classList.contains('is-results-maximized')))) await action(page, 'maximize-results');
        await settle(page);
        await audit(page, key, 'results');
        await screenshot(page, key, 'results');
        await action(page, 'maximize-results');
        // Restore the same collapsed dock state used before this excursion.
        await action(page, 'toggle-results');
        await settle(page);
        const cameraAfter = await cameraSnapshot(page);
        check(`${key}: returning from results preserves camera zoom`, Math.abs(cameraBefore.scale - cameraAfter.scale) < 1e-8, { cameraBefore, cameraAfter });
        // Layout docking can translate the camera. The point at the stage's
        // centre should remain the same once its original size is restored.
        check(`${key}: returning from results preserves map centre`, Math.hypot(cameraBefore.centreX - cameraAfter.centreX, cameraBefore.centreY - cameraAfter.centreY) < 1, { cameraBefore, cameraAfter });
        check(`${key}: navigation does not modify scientific state`, JSON.stringify(scientificBefore) === JSON.stringify(await scienceSnapshot(page)));
        console.log(`Completed ${key}`);
      } catch (error) {
        check(`${key}: scenario completes`, false, error.message);
        await screenshot(page, key, 'failure').catch(() => {});
      } finally {
        await context.close();
      }
    }
  }
  if (args.functional) {
    await functionalSmoke();
    await integratedFeatureSmoke();
  }
} finally {
  await browser.close();
  check('browser console has no application errors', report.runtimeErrors.length === 0, report.runtimeErrors);
  report.completedAt = new Date().toISOString();
  report.failures = report.checks.filter((entry) => !entry.pass).length;
  await writeFile(join(out, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
}
console.log(`${report.checks.length - report.failures}/${report.checks.length} checks passed; ${report.screenshots.length} screenshots. Report: ${join(out, 'report.json')}`);
process.exitCode = report.failures ? 1 : 0;

async function settle(page) {
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
  await page.waitForTimeout(180);
}

async function action(page, name) {
  const controls = page.locator(`[data-action="${name}"]`);
  const visible = controls.filter({ visible: true });
  if (!(await visible.count())) throw new Error(`No visible control for ${name}`);
  await visible.first().click();
}

async function ensureTasksVisible(page) {
  if (!(await page.locator('#task-panel').isVisible())) await action(page, 'toggle-left');
  await settle(page);
}

async function screenshot(page, key, name) {
  const file = `${key}-${name}.png`;
  // Chromium may need extra compositor time for the first capture after a
  // theme change. Interaction checks keep their shorter timeout; evidence
  // capture gets its own budget. The context already reduces motion.
  await page.screenshot({ path: join(out, file), fullPage: false, timeout: 30000 });
  report.screenshots.push(file);
}

async function scienceSnapshot(page) {
  return page.evaluate(() => {
    const store = window.escherMCMC.store;
    return { objective: store.solution?.objectiveValue, status: store.solution?.status, mode: [store.parsimonious, store.loopless, store.fvaEnabled], species: store.species.map((sp) => [sp.id, sp.enabled, sp.abundance]), medium: store.medium, overrides: [...store.overrides], reactions: store.community.merged.reactions.size };
  });
}

async function cameraSnapshot(page) {
  return page.evaluate(() => {
    const { camera } = window.escherMCMC;
    const box = document.getElementById('stage').getBoundingClientRect();
    return { scale: camera.scale, centreX: (box.width / 2 - camera.x) / camera.scale, centreY: (box.height / 2 - camera.y) / camera.scale, width: box.width, height: box.height };
  });
}

async function audit(page, key, state, options = {}) {
  const data = await page.evaluate(({ contrast }) => {
    const visible = (node) => {
      if (!node?.getClientRects().length) return false;
      const closedDisclosure = node.closest('details:not([open])');
      if (closedDisclosure && !closedDisclosure.querySelector('summary')?.contains(node)) return false;
      if (typeof node.checkVisibility === 'function' && !node.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) return false;
      const box = node.getBoundingClientRect();
      const style = getComputedStyle(node);
      return box.width > 0 && box.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
    };
    const label = (node) => `${node.tagName.toLowerCase()}${node.id ? `#${node.id}` : ''}${node.dataset.action ? `[${node.dataset.action}]` : ''}: ${(node.textContent || node.getAttribute('aria-label') || '').trim().slice(0, 90)}`;
    const rect = (node) => { const r = node.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; };
    const clipFailures = [];
    const left = document.getElementById('left-panel');
    const task = document.getElementById('task-panel');
    if (visible(left)) {
      for (const control of left.querySelectorAll('button, input, select, summary, a[href]')) {
        if (!visible(control)) continue;
        const c = control.getBoundingClientRect();
        if (c.left < -1 || c.top < -1 || c.right > innerWidth + 1 || c.bottom > innerHeight + 1) clipFailures.push({ control: label(control), by: 'viewport', rect: rect(control) });
        let parent = control.parentElement;
        while (parent) {
          const style = getComputedStyle(parent);
          const box = parent.getBoundingClientRect();
          const clipX = /hidden|clip|auto|scroll/.test(style.overflowX);
          const clipY = /hidden|clip|auto|scroll/.test(style.overflowY);
          if ((clipX && (c.left < box.left - 1 || c.right > box.right + 1)) || (clipY && (c.top < box.top - 1 || c.bottom > box.bottom + 1))) {
            clipFailures.push({ control: label(control), by: label(parent), rect: rect(control) });
            break;
          }
          parent = parent.parentElement;
        }
      }
    }
    const panelGeometry = [left, task].filter(visible).map((node) => ({ id: node.id, clientHeight: node.clientHeight, scrollHeight: node.scrollHeight, clientWidth: node.clientWidth, scrollWidth: node.scrollWidth, overflowY: getComputedStyle(node).overflowY }));
    const controlTextFailures = [...document.querySelectorAll('#left-panel button, .canvas-toolbar button')].filter(visible)
      .filter((node) => node.scrollWidth > node.clientWidth + 1 || node.scrollHeight > node.clientHeight + 1)
      .map((node) => ({ control: label(node), width: node.clientWidth, textWidth: node.scrollWidth, height: node.clientHeight, textHeight: node.scrollHeight }));
    const controls = [...document.querySelectorAll('#task-panel button')].filter(visible);
    const entry = [...document.querySelectorAll('[data-action="toggle-left"], [data-action="open-workspace"]')].some(visible);
    const rootOverflow = { viewport: innerWidth, document: document.documentElement.scrollWidth };
    const canvas = document.querySelector('#stage [data-canvas-bg]');
    const background = document.querySelector('.stage');
    const surface = { svgFill: canvas?.getAttribute('fill') || null, stageBackground: background ? getComputedStyle(background).backgroundColor : null };
    const floatingFailures = [];
    const menu = document.getElementById('view-controls');
    if (visible(menu)) {
      const box = menu.getBoundingClientRect();
      if (box.left < -1 || box.top < -1 || box.right > innerWidth + 1 || box.bottom > innerHeight + 1) floatingFailures.push({ region: 'View menu', rect: rect(menu) });
    }
    const inspector = document.getElementById('inspector-panel');
    if (inspector && document.getElementById('app-shell').classList.contains('is-inspector-collapsed') && getComputedStyle(inspector).position === 'absolute' && inspector.getBoundingClientRect().width > 1 && getComputedStyle(inspector).pointerEvents !== 'none') {
      floatingFailures.push({ region: 'Collapsed inspector intercepts the map or results', rect: rect(inspector) });
    }
    const contrastFailures = [];
    let contrastSamples = 0;
    function color(value) {
      const parts = value?.match(/[\d.]+/g)?.map(Number);
      return parts?.length >= 3 ? [parts[0], parts[1], parts[2], parts[3] ?? 1] : null;
    }
    function composite(foreground, backgroundColor) {
      const alpha = foreground[3];
      return foreground.slice(0, 3).map((component, i) => component * alpha + backgroundColor[i] * (1 - alpha)).concat(1);
    }
    function bg(node) {
      const chain = [];
      while (node) { chain.push(color(getComputedStyle(node).backgroundColor)); node = node.parentElement; }
      return chain.reverse().reduce((base, entry) => entry ? composite(entry, base) : base, [255, 255, 255, 1]);
    }
    function luminance(c) {
      const linear = c.slice(0, 3).map((v) => { const s = v / 255; return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; });
      return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
    }
    if (contrast) {
      for (const node of document.querySelectorAll('.topbar *, #left-panel *, .canvas-toolbar *, #workspace-editor *, #inspector *, #results-panel *')) {
        if (!visible(node) || node.disabled || node.closest('[disabled]') || node.namespaceURI === 'http://www.w3.org/2000/svg') continue;
        const directText = [...node.childNodes].filter((child) => child.nodeType === Node.TEXT_NODE).map((child) => child.textContent).join('').trim();
        if (!/[\p{L}\p{N}]{2}/u.test(directText) || getComputedStyle(node).opacity < 1) continue;
        const style = getComputedStyle(node);
        const text = color(style.color);
        if (!text) continue;
        const backdrop = bg(node);
        const a = luminance(composite(text, backdrop));
        const b = luminance(backdrop);
        const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
        const size = parseFloat(style.fontSize);
        const target = size >= 24 || (size >= 18.666 && parseFloat(style.fontWeight) >= 700) ? 3 : 4.5;
        contrastSamples += 1;
        if (ratio + 0.02 < target) contrastFailures.push({ control: label(node), ratio: Number(ratio.toFixed(2)), target, color: style.color, background: backdrop });
      }
    }
    return { panelGeometry, clipFailures, taskControls: controls.length, entry, rootOverflow, surface, contrastFailures, contrastSamples, floatingFailures, controlTextFailures };
  }, { contrast: options.contrast !== false });
  const prefix = `${key}/${state}`;
  check(`${prefix}: left panels have no vertical or horizontal overflow`, data.panelGeometry.every((p) => p.scrollHeight <= p.clientHeight + 1 && p.scrollWidth <= p.clientWidth + 1), data.panelGeometry);
  check(`${prefix}: every visible left control fits without clipping`, data.clipFailures.length === 0, data.clipFailures);
  check(`${prefix}: tool and menu button labels fit their controls`, data.controlTextFailures.length === 0, data.controlTextFailures);
  check(`${prefix}: task controls or their opening control are available`, data.taskControls > 0 || data.entry, { taskControls: data.taskControls, entry: data.entry });
  check(`${prefix}: document has no horizontal overflow`, data.rootOverflow.document <= data.rootOverflow.viewport + 1, data.rootOverflow);
  check(`${prefix}: floating panels fit and closed panels do not intercept controls`, data.floatingFailures.length === 0, data.floatingFailures);
  if (options.contrast !== false) check(`${prefix}: enabled HTML text meets contrast thresholds`, data.contrastFailures.length === 0 && data.contrastSamples > 0, { failures: data.contrastFailures, samples: data.contrastSamples });
  if (key.endsWith('-dark')) check(`${prefix}: canvas uses a dark SVG surface`, data.surface.svgFill !== '#ffffff' && data.surface.svgFill !== '#fff' && Boolean(data.surface.svgFill), data.surface);
}

async function functionalSmoke() {
  const key = 'functional-1366x768-dark';
  const context = await browser.newContext({ viewport: { width: 1366, height: 768 }, reducedMotion: 'reduce' });
  const page = await context.newPage();
  page.setDefaultTimeout(10000);
  const errors = [];
  const result = [];
  page.on('pageerror', error => { errors.push(error.message); report.runtimeErrors.push({ key, message: error.message }); });
  try {
  await page.goto(`${base}/index.html?scenario=consorcio-anaerobio`, { waitUntil: 'networkidle' });
  await page.waitForFunction(() => window.escherMCMC?.store?.solution && window.escherMCMC.store.scenario);
  if (await page.evaluate(() => document.documentElement.dataset.theme) !== 'dark') await action(page, 'toggle-theme');
  await settle(page);
  const baseline = await page.evaluate(() => ({
    reactions: window.escherMCMC.store.community.merged.reactions.size,
    metabolites: window.escherMCMC.store.community.merged.metabolites.size,
  }));

  await page.locator('[data-action="tool-metabolite"]').click();
  await page.locator('.model-editor-form [name="id"]').fill('QA_DRAFT_METABOLITE');
  await page.locator('.model-editor-form [name="name"]').fill('Temporary draft retained');
  await page.locator('.header-search').click();
  await page.locator('[data-action="close-workspace"]').click();
  await page.locator('.task-link[data-workspace="model"]').click();
  assert.equal(await page.locator('.model-editor-form [name="id"]').inputValue(), 'QA_DRAFT_METABOLITE');
  assert.equal(await page.locator('.model-editor-form [name="name"]').inputValue(), 'Temporary draft retained');
  result.push('Metabolite draft survives library, map and model navigation.');

  const compartmentCounts = await page.evaluate(() => {
    const row = [...document.querySelectorAll('.model-overview-card .overview-list > div')]
      .find(item => item.querySelector('dt')?.textContent === 'Compartments');
    return { displayed: Number(row.querySelector('dd').textContent), actual: window.escherMCMC.store.map.stats().compartments };
  });
  assert.equal(compartmentCounts.displayed, compartmentCounts.actual);
  result.push('Overview displays the actual number of drawn compartments.');

  await page.locator('[data-action="cancel-model-editor"]').click();
  await page.locator('[data-action="new-metabolite"]').click();
  assert.equal(await page.locator('.model-editor-form [name="id"]').inputValue(), '');
  assert.equal(await page.locator('.model-editor-form [name="name"]').inputValue(), '');
  result.push('Cancel clears the metabolite draft.');
  await page.locator('[data-action="cancel-model-editor"]').click();

  await page.locator('[data-action="new-reaction"]').click();
  await page.locator('.model-editor-form [name="id"]').fill('QA_DRAFT_REACTION');
  await page.locator('.model-editor-form [name="stoichiometry"]').fill('pyr_c:-1, lac__L_c:1');
  await page.locator('.model-editor-form [name="addToMap"]').uncheck();
  await page.locator('.header-search').click();
  await page.locator('[data-action="close-workspace"]').click();
  await page.locator('.task-link[data-workspace="model"]').click();
  assert.equal(await page.locator('.model-editor-form [name="id"]').inputValue(), 'QA_DRAFT_REACTION');
  assert.equal(await page.locator('.model-editor-form [name="stoichiometry"]').inputValue(), 'pyr_c:-1, lac__L_c:1');
  assert.equal(await page.locator('.model-editor-form [name="addToMap"]').isChecked(), false);
  result.push('Reaction draft, textarea and checkbox survive navigation.');
  await page.locator('[data-action="cancel-model-editor"]').click();

  await page.locator('.task-link[data-workspace="medium"]').click();
  const input = page.locator('[data-action="medium-number"]').first();
  const nutrientBase = await input.getAttribute('data-base');
  const next = (Number(await input.inputValue()) + 1.5).toString();
  await input.fill(next);
  await input.blur();
  await page.waitForFunction(({base, value}) => window.escherMCMC.store.medium[base] === value, { base: nutrientBase, value: Number(next) });
  assert.equal(await page.locator(`[data-action="medium-number"][data-base="${nutrientBase}"]`).inputValue(), next);
  assert.deepEqual(await page.evaluate(() => ({
    reactions: window.escherMCMC.store.community.merged.reactions.size,
    metabolites: window.escherMCMC.store.community.merged.metabolites.size,
  })), baseline);
  result.push(`Medium supply ${nutrientBase} updates actual state and preserves model counts.`);

  await page.locator('.header-search').click();
  const lookup = await page.evaluate(() => {
    const s = window.escherMCMC.store;
    return [...s.community.reactionOwner].find(([id, owner]) => owner.speciesId && s.map.reactions.has(id));
  });
  await page.locator('#reaction-search-input').fill(lookup[1].originalId);
  const row = page.locator(`[data-action="search-select-reaction"][data-rxn="${lookup[0]}"]`);
  await row.click();
  assert.equal(await page.locator('#workspace-editor').isVisible(), false);
  assert.equal(await page.locator('#stage').evaluate(node => node.inert), false);
  assert.equal(await page.locator('#stage svg').isVisible(), true);
  assert.equal(await page.evaluate(() => window.escherMCMC.store.selectedReaction), lookup[0]);
  assert.equal(await page.locator('#inspector-panel').isVisible(), true);
  result.push('Reaction selection returns to the map and opens its inspector.');

  await page.locator('[data-action="left-tab"][data-tab="model"]').click();
  await page.locator('.task-link[data-workspace="community"]').click();
  const toggle = page.locator('[data-action="species-toggle"]').first();
  const speciesId = await toggle.getAttribute('data-species');
  const initialChecked = await toggle.isChecked();
  await toggle.setChecked(!initialChecked);
  await page.waitForFunction(({ id, enabled }) => window.escherMCMC.store.species.find(s => s.id === id).enabled === enabled, { id: speciesId, enabled: !initialChecked });
  await page.locator(`[data-action="species-toggle"][data-species="${speciesId}"]`).setChecked(initialChecked);
  result.push('Species toggle changes actual scientific state and can restore it.');
  await page.locator('[data-action="close-workspace"]').click();

  await page.locator('[data-action="left-tab"][data-tab="analysis"]').click();
  await page.locator('.task-link[data-workspace="crossfeeding"]').click();
  await page.locator('[data-action="open-crossfeeding-explorer"]').click();
  const eventButton = page.locator('[data-action="select-crossfeeding-event"]').first();
  if (await eventButton.count()) {
    const eventId = await eventButton.getAttribute('data-crossfeed-id');
    await eventButton.click();
    assert.equal(await page.evaluate(() => window.escherMCMC.store.crossFeedingExplorer.selectedId), eventId);
    result.push('Cross-feeding event selection updates real explorer state.');
  }
  await page.locator('[data-action="crossfeeding-query"]').click();
  await page.locator('[data-action="crossfeeding-query"]').pressSequentially('hydrogen');
  assert.equal(await page.evaluate(() => window.escherMCMC.store.crossFeedingExplorer.query), 'hydrogen');
  result.push('Cross-feeding search retains focus throughout typing.');
  const queryBase = await page.evaluate(() => window.escherMCMC.store.crossFeedingExplorer.index.events[0]?.base);
  if (queryBase) {
    await page.locator('[data-action="crossfeeding-query"]').fill('');
    await page.locator('[data-action="crossfeeding-query"]').pressSequentially(queryBase);
    const filteredEvent = page.locator('[data-action="select-crossfeeding-event"]').first();
    const filteredId = await filteredEvent.getAttribute('data-crossfeed-id');
    await filteredEvent.click();
    assert.equal(await page.evaluate(() => window.escherMCMC.store.crossFeedingExplorer.selectedId), filteredId);
    result.push('Filtered cross-feeding event responds to the first click without a search blur workaround.');
  }
  await page.locator('[data-action="close-workspace"]').click();

  await page.locator('[data-action="left-tab"][data-tab="build"]').click();
  await page.locator('.task-link[data-workspace="map"]').click();
  await page.locator('[data-action="map-empty"]').click();
  assert.equal(await page.locator('#workspace-editor').isVisible(), false);
  assert.equal(await page.evaluate(() => window.escherMCMC.store.map.cells.length), 0);
  assert.deepEqual(await page.evaluate(() => ({
    reactions: window.escherMCMC.store.community.merged.reactions.size,
    metabolites: window.escherMCMC.store.community.merged.metabolites.size,
  })), baseline);
  await page.locator('.task-link[data-workspace="structure"]').click();
  const cellSelect = page.locator('#cell-species-select');
  const cellOptions = await cellSelect.locator('option').evaluateAll(nodes => nodes.map(node => node.value));
  await cellSelect.selectOption(cellOptions.at(-1));
  const chosenSpecies = cellOptions.at(-1);
  await page.locator('[data-action="close-workspace"]').click();
  assert.equal(await page.evaluate(() => window.escherMCMC.store.cellPlacementSpeciesId), chosenSpecies);
  await page.locator('[data-action="tool-cell"]').click();
  assert.equal(await page.locator('#workspace-editor').isVisible(), false);
  await page.evaluate(id => window.escherMCMC.store.addMapCell(id, { x: 100, y: 100, w: 800, h: 600 }), chosenSpecies);
  await page.locator('.task-link[data-workspace="structure"]').click();
  const compSelect = page.locator('#map-compartment-select');
  await compSelect.selectOption(await compSelect.locator('option').first().getAttribute('value'));
  const chosenComp = await compSelect.inputValue();
  await page.locator('[data-action="close-workspace"]').click();
  assert.equal(await page.evaluate(() => window.escherMCMC.store.compartmentPlacementKey), chosenComp);
  await page.locator('.tool-button[data-action="tool-compartment"]').click();
  assert.equal(await page.locator('#workspace-editor').isVisible(), false);
  result.push('Clearing the drawing preserves the model; species and compartment placement survive workspace closure.');

  await page.locator('[data-action="left-tab"][data-tab="analysis"]').click();
  await page.locator('#task-panel [data-action="run-fba"]').click();
  await settle(page);
  assert.equal(await page.evaluate(() => document.activeElement?.dataset.action === 'run-fba'
    && Boolean(document.activeElement.closest('#task-panel'))
    && !document.activeElement.closest('details:not([open])')), true);
  result.push('Running analysis returns keyboard focus to its visible task button.');

  assert.deepEqual(errors, []);
  for (const message of result) check(`${key}: ${message}`, true);
  console.log(`Completed ${key}`);

  } catch (error) {
    for (const message of result) check(`${key}: ${message}`, true);
    check(`${key}: migrated controls complete their functional routes`, false, error.stack);
    await screenshot(page, key, 'failure').catch(() => {});
  } finally {
    await context.close();
  }
}

/** Regressions where the scientific cards (published communities, omics, GIMME, pathways) meet the workspace shell. */
async function integratedFeatureSmoke() {
  const key = 'integration-1366x768-dark';
  const context = await browser.newContext({ viewport: { width: 1366, height: 768 }, reducedMotion: 'reduce' });
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  page.on('pageerror', error => report.runtimeErrors.push({ key, message: error.message }));
  try {
    await page.goto(`${base}/index.html?scenario=consorcio-anaerobio`, { waitUntil: 'networkidle' });
    await page.waitForFunction(() => window.escherMCMC?.store?.solution && window.escherMCMC.store.scenario);
    if (await page.evaluate(() => document.documentElement.dataset.theme) !== 'dark') await action(page, 'toggle-theme');
    await settle(page);
    let baseline = await scienceSnapshot(page);

    await page.locator('#left-panel [data-action="left-tab"][data-tab="model"]').click();
    await page.locator('#task-panel [data-workspace="scenario"]').click();
    const published = page.locator('[data-action="published-community"]');
    const registry = await page.evaluate(() => window.escherMCMC.store.publishedCommunities);
    check(`${key}: Published Communities is reachable and exposes the complete registry`,
      await published.isVisible() && await published.locator('option').count() === registry.length && registry.length === 4);
    const gut9 = registry.find(entry => entry.species?.length === 9 && entry.computed?.sweep?.length);
    assert.ok(gut9, 'The reproduced nine-species Figure 4 community is missing');
    await published.selectOption(gut9.id);
    await settle(page);
    check(`${key}: the nine-species community exposes the complete Figure 4 sweep`,
      await page.locator('.pc-sweep tbody tr').count() === gut9.computed.sweep.length
        && await page.locator('.pc-ok').isVisible());
    check(`${key}: browsing published results preserves live scientific inputs and solution`,
      JSON.stringify(await scienceSnapshot(page)) === JSON.stringify(baseline));
    await audit(page, key, 'published-gut9');
    await screenshot(page, key, 'published-gut9');
    await action(page, 'close-workspace');

    // Bundled teaching models omit GPR annotations. Supply annotated copies as
    // a controlled fixture through the same model importer used by the app.
    // Their stoichiometry, bounds and biomass objective stay unchanged.
    const models = await page.evaluate(() => window.escherMCMC.store.species.map(species => {
      const raw = window.escherMCMC.store.models.get(species.file).toJSON();
      raw.id = species.id;
      raw.genes = raw.reactions.map((reaction, index) => {
        const id = `qa_gene_${index}`;
        reaction.gene_reaction_rule = id;
        return { id, name: id };
      });
      return { name: `${species.id}-with-gpr.json`, text: JSON.stringify(raw) };
    }));
    await page.locator('#model-file-input').setInputFiles(models.map(model => ({
      name: model.name, mimeType: 'application/json', buffer: Buffer.from(model.text),
    })));
    await page.waitForFunction(() => {
      const store = window.escherMCMC.store;
      return store.scenario?.uploaded && store.solution?.status === 'optimal' && store.status.state === 'ok';
    });
    await page.evaluate(async () => { await window.escherMCMC.store.rebuild({ relayout: true, contents: 'all' }); });
    baseline = await scienceSnapshot(page);
    check(`${key}: controlled GPR model import establishes an optimal baseline`,
      baseline.status === 'optimal' && Number.isFinite(baseline.objective));

    await page.locator('#task-panel [data-workspace="omics"]').click();
    const csv = await page.evaluate(() => {
      const store = window.escherMCMC.store;
      const rows = ['species,gene,Control,Treatment'];
      for (const species of store.species) {
        const model = store.models.get(species.file);
        const genes = new Set([...model.reactions.values()].flatMap(reaction =>
          reaction.geneReactionRule.match(/[A-Za-z0-9_.:-]+/g) ?? []).filter(gene => !/^(?:and|or)$/i.test(gene)));
        for (const [index, gene] of [...genes].entries()) rows.push(`${species.id},${gene},${index + 1},${(index + 1) * 2}`);
      }
      if (rows.length === 1) throw new Error('The scenario has no GPR genes for a real omics mapping test');
      return rows.join('\n');
    });
    await page.locator('#omics-file-input').setInputFiles({ name: 'integration-overlay.csv', mimeType: 'text/csv', buffer: Buffer.from(csv) });
    await page.waitForFunction(() => window.escherMCMC.store.omicsDataset?.source === 'integration-overlay.csv');
    await page.locator('[data-action="omics-mode"]').selectOption('single');
    await page.locator('[data-action="omics-condition-a"]').selectOption('Treatment');
    check(`${key}: visual omics controls update only the visual dataset settings`, await page.evaluate(() => {
      const store = window.escherMCMC.store;
      return store.omicsSettings.mode === 'single' && store.omicsSettings.conditionA === 'Treatment'
        && store.omics.dataset === null;
    }));
    await action(page, 'close-workspace');
    const painted = await page.evaluate(() => {
      const store = window.escherMCMC.store;
      const overlay = store.omicsOverlay();
      const sample = [...overlay.colors].find(([id]) => document.querySelector(`.emc-seg[data-rxn="${CSS.escape(id)}"]`));
      const segment = sample && document.querySelector(`.emc-seg[data-rxn="${CSS.escape(sample[0])}"]`);
      return { matched: overlay.matchedReactions, expected: sample?.[1], actual: segment?.getAttribute('stroke') };
    });
    check(`${key}: imported visual omics colors reach the actual SVG`, painted.matched > 0 && painted.actual === painted.expected, painted);

    await page.locator('#task-panel [data-workspace="omics"]').click();
    await page.locator('#metatranscriptomics-file-input').setInputFiles({ name: 'integration-community.csv', mimeType: 'text/csv', buffer: Buffer.from(csv) });
    await page.waitForFunction(() => window.escherMCMC.store.omics.dataset?.source === 'integration-community.csv');
    await page.locator('[data-action="omics-compare"]').click();
    await page.locator('[data-action="community-omics-condition-a"]').selectOption('Control');
    await page.locator('[data-action="community-omics-condition-b"]').selectOption('Treatment');
    await page.locator('[data-action="community-omics-metric"]').selectOption('difference');
    await page.locator('[data-action="community-omics-pseudocount"]').fill('3');
    await page.locator('[data-action="community-omics-pseudocount"]').blur();
    check(`${key}: GIMME expression controls leave the visual overlay settings independent`, await page.evaluate(() => {
      const store = window.escherMCMC.store;
      return store.omics.mode === 'compare' && store.omics.conditionA === 'Control'
        && store.omics.conditionB === 'Treatment' && store.omics.metric === 'difference' && store.omics.pseudocount === 3
        && store.omicsSettings.mode === 'single' && store.omicsSettings.conditionA === 'Treatment'
        && store.omicsSettings.metric === 'log2fc' && store.omicsSettings.pseudocount === 1;
    }));
    await page.locator('[data-action="omics-mode"]').selectOption('compare');
    await page.locator('[data-action="omics-pseudocount"]').fill('2');
    await page.locator('[data-action="omics-pseudocount"]').blur();
    await page.locator('[data-action="omics-metric"]').selectOption('difference');
    check(`${key}: visual comparison controls leave community expression parameters independent`, await page.evaluate(() => {
      const store = window.escherMCMC.store;
      return store.omicsSettings.mode === 'compare' && store.omicsSettings.metric === 'difference'
        && store.omicsSettings.pseudocount === 2 && store.omics.pseudocount === 3;
    }));
    await action(page, 'close-workspace');
    check(`${key}: overlapping datasets keep the selected visual overlay colors`, await page.evaluate(() => {
      const store = window.escherMCMC.store;
      const colors = store.omicsOverlay().colors;
      const sample = [...colors].find(([id]) => document.querySelector(`.emc-seg[data-rxn="${CSS.escape(id)}"]`));
      return sample && document.querySelector(`.emc-seg[data-rxn="${CSS.escape(sample[0])}"]`).getAttribute('stroke') === sample[1];
    }));
    await page.locator('#task-panel [data-workspace="omics"]').click();
    await page.locator('[data-action="omics-enabled"]').uncheck();
    await action(page, 'close-workspace');
    check(`${key}: community expression still colors the map when the visual overlay is disabled`, await page.evaluate(() => {
      const colors = window.escherMCMC.store.omics.colors;
      const sample = [...colors].find(([id]) => document.querySelector(`.emc-seg[data-rxn="${CSS.escape(id)}"]`));
      return sample && document.querySelector(`.emc-seg[data-rxn="${CSS.escape(sample[0])}"]`).getAttribute('stroke') === sample[1];
    }));
    await page.locator('#task-panel [data-workspace="omics"]').click();
    await page.locator('[data-action="gimme-objective-fraction"]').fill('80');
    await page.locator('[data-action="gimme-objective-fraction"]').blur();
    await page.locator('[data-action="run-gimme"]').click();
    await page.waitForFunction(() => window.escherMCMC.store.omics.gimmeResult?.status === 'optimal');
    check(`${key}: Run GIMME solves the real constrained community problem`, await page.evaluate(() => {
      const store = window.escherMCMC.store;
      return store.omics.gimmeEnabled && store.solution.gimme
        && store.solution.objectiveValue >= store.omics.gimmeResult.requiredObjective - 1e-7
        && store.omics.gimmeResult.objectiveFraction === 0.8;
    }));
    await audit(page, key, 'loaded-omics-and-gimme');
    await screenshot(page, key, 'loaded-omics-and-gimme');
    await page.locator('[data-action="disable-gimme"]').click();
    check(`${key}: Return FBA restores the original analysis after GIMME`, await page.evaluate(expected => {
      const store = window.escherMCMC.store;
      return !store.omics.gimmeEnabled && !store.solution.gimme && store.solution.status === 'optimal'
        && Math.abs(store.solution.objectiveValue - expected) < 1e-7;
    }, baseline.objective));
    await action(page, 'close-workspace');

    // Seed selection through the public store API, then exercise the manager's
    // create/edit/hide actions. The selection gesture itself has separate tests.
    await page.evaluate(() => {
      const store = window.escherMCMC.store;
      store.multiSelection.reactions = new Set(store.map.reactionIds().slice(0, 2));
      store.emit('multi-selection');
    });
    await page.locator('#left-panel [data-action="left-tab"][data-tab="build"]').click();
    await page.locator('#task-panel [data-workspace="map"]').click();
    await page.locator('.pathway-create [name="pathway-name"]').fill('Integration QA pathway');
    await page.locator('[data-action="pathway-create"]').click();
    const pathway = await page.evaluate(() => [...window.escherMCMC.store.manualPathways.values()].find(item => item.name === 'Integration QA pathway')?.id);
    assert.ok(pathway, 'Creating a manual pathway did not reach the store');
    await page.locator(`[data-action="pathway-color"][data-pathway-key="manual:${pathway}"]`).fill('#9333ea');
    await page.locator(`[data-action="pathway-color"][data-pathway-key="manual:${pathway}"]`).dispatchEvent('change');
    await page.locator(`[data-action="pathway-visible"][data-pathway-key="manual:${pathway}"]`).uncheck();
    check(`${key}: pathway controls retain their edit and visibility semantics`, await page.evaluate(id => {
      const store = window.escherMCMC.store;
      const item = store.manualPathways.get(id);
      return item.color === '#9333ea' && item.visible === false && item.reactionIds.size === 2;
    }, pathway));
    check(`${key}: pathway presentation changes preserve the model and objective`,
      JSON.stringify(await scienceSnapshot(page)) === JSON.stringify(baseline));
    await settle(page);
    const editorHeader = await page.evaluate(() => {
      const editor = document.getElementById('workspace-editor');
      const regions = [editor.querySelector('.workspace-editor-header'), editor.querySelector('[data-action="close-workspace"]')];
      const box = editor.getBoundingClientRect();
      return {
        scrollTop: editor.scrollTop,
        regions: regions.map(node => {
          const rect = node.getBoundingClientRect();
          const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
          return { y: rect.y, height: rect.height, fits: rect.top >= box.top - 1 && rect.bottom <= box.bottom + 1
            && rect.top >= 0 && rect.bottom <= innerHeight, receivesPointer: node === hit || node.contains(hit) };
        }),
      };
    });
    check(`${key}: lower pathway inputs keep the editor header and Return to map visible`,
      editorHeader.scrollTop === 0 && editorHeader.regions.every(region => region.fits && region.receivesPointer), editorHeader);
    await audit(page, key, 'pathways');
    await screenshot(page, key, 'pathways');
    console.log(`Completed ${key}`);
  } catch (error) {
    check(`${key}: integrated scientific cards complete their functional routes`, false, error.stack);
    await screenshot(page, key, 'failure').catch(() => {});
  } finally {
    await context.close();
  }
}
