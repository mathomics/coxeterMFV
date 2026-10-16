#!/usr/bin/env node
/**
 * Time the heavy paths of the app in Node, phase by phase, so a decision about
 * what to move off the browser (native solver, parallel workers, a different
 * renderer) rests on measurements rather than guesses.
 *
 *   node tools/profile-heavy.mjs --case=core|iaf1260|ecoli4|gut9|zuniga
 *       [--full-map] [--fva=N|all] [--file=model.json] [--out=results.jsonl]
 *
 * Every row says how long a step took, how many LPs it solved and how much of
 * that time was spent inside GLPK itself, so the cost of building each LP in
 * JavaScript shows up as the difference. --full-map also runs the automatic
 * layout on everything, which takes minutes on a genome-scale model; run each
 * case in its own process so a slow one can be stopped without losing the rest.
 * The gut9 case without --full-map solves one Figure 4 point, which runs for
 * more than twenty minutes. The zuniga case downloads the study's source .mat
 * files, as the app does when that community is opened.
 */
import { appendFile, readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';
import factory from '../assets/vendor/glpk.js';
import { Store } from '../src/ui/store.js';
import { Model } from '../src/core/model.js';
import { FluxProblem } from '../src/solver/fba.js';
import { initGLPK } from '../src/solver/glpk.js';
import { renderMap } from '../src/render/mapview.js';
import { steadyCom, steadyComFVA, maxTotalBiomassAt } from '../src/core/steadycom.js';
import { serializeSteadyComSpec, restoreSteadyComSpec } from '../src/communities/steadycom-runtime.js';
import { figureTargets } from '../src/communities/gut9-figure4.js';
import { toSBML } from '../src/core/sbml-write.js';
import { parseCOBRASBML } from '../src/core/sbml.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = Object.fromEntries(process.argv.slice(2).map((a) => {
  const [k, v = 'true'] = a.replace(/^--/, '').split('=');
  return [k, v];
}));
const caseName = args.case ?? 'core';
const fullMap = args['full-map'] === 'true';
const fvaCount = args.fva === 'all' ? Infinity : Number(args.fva ?? 40);

// Count every LP and the time GLPK spends on it.
const lp = { calls: 0, ms: 0 };
globalThis.__GLPK_WASM__ = await readFile(resolve(root, 'assets/vendor/glpk.wasm'));
globalThis.__GLPK_FACTORY__ = async (options) => {
  const instance = await factory(options);
  const solve = instance.solve.bind(instance);
  // solve is a non-writable own property, so wrap a copy instead of the instance.
  return {
    ...instance,
    solve: (problem, opts) => {
      const t = performance.now();
      try { return solve(problem, opts); } finally { lp.calls++; lp.ms += performance.now() - t; }
    },
  };
};

// Before any FluxProblem exists: without it 'auto' quietly picks simplex.js.
if (!await initGLPK()) throw new Error('GLPK did not load');

// The Store fetches relative to its base; serve those from the checkout.
globalThis.fetch = async (url) => {
  try {
    const buffer = await readFile(resolve(root, String(url)));
    return {
      ok: true, status: 200,
      text: async () => buffer.toString('utf8'),
      json: async () => JSON.parse(buffer.toString('utf8')),
      arrayBuffer: async () => buffer,
    };
  } catch {
    return { ok: false, status: 404 };
  }
};

const round = (ms) => Math.round(ms * 10) / 10;
const heapMB = () => Math.round(process.memoryUsage().heapUsed / 1048576);

async function step(name, fn, describe = () => ({})) {
  const before = { ...lp };
  const t = performance.now();
  const value = await fn();
  const ms = performance.now() - t;
  const row = {
    case: caseName, step: name, ms: round(ms),
    lps: lp.calls - before.calls, glpkMs: round(lp.ms - before.ms),
    heapMB: heapMB(), ...describe(value),
  };
  const extra = Object.entries(row).filter(([k]) => !['case', 'step', 'ms', 'lps', 'glpkMs', 'heapMB'].includes(k))
    .map(([k, v]) => `${k}=${v}`).join(' ');
  console.log(`${name.padEnd(34)} ${String(row.ms).padStart(10)} ms  lps=${String(row.lps).padStart(5)}`
    + `  glpk=${String(row.glpkMs).padStart(9)} ms  heap=${row.heapMB} MB  ${extra}`);
  if (args.out) await appendFile(args.out, `${JSON.stringify(row)}\n`);
  return value;
}

/** What app.js hands the renderer, minus the parts only a browser has. */
function renderState(store) {
  const reactionColoring = store.reactionColoring();
  const ribbon = store.pathwayRibbonColoring();
  return {
    fluxes: store.displayFluxes(),
    solvedFluxes: store.solution?.status === 'optimal' ? store.solution.fluxes : new Map(),
    reactionWidthMode: store.isZunigaStudy ? 'flux' : 'constant',
    disabledReactionIds: new Set(),
    inactiveSpeciesIds: new Set(store.disabledDrawnSpeciesIds?.() ?? []),
    scale: store.scale,
    dark: false,
    reactionColorMode: store.view.reactionColorMode,
    reactionColors: reactionColoring.colors,
    omicsEnabled: false,
    omicsColors: new Map(),
    pathwayRibbonMode: store.view.pathwayRibbonMode,
    pathwayRibbonColors: ribbon.colors,
    pathwayRibbonGroups: ribbon.groups,
    pathwayRibbonWidth: store.view.pathwayRibbonWidth,
    pathwayRibbonOpacity: store.view.pathwayRibbonOpacity,
    selectedReaction: null,
    reactionEdit: false,
    selectedGeometry: null,
    geometryEdit: store.view.geometryEdit,
    multiSelection: store.multiSelection,
    fvaEnabled: store.fvaEnabled,
    fvaRanges: store.fvaRanges,
    fvaProgress: store.fvaProgress,
    labels: store.view.labels,
    metaboliteLabels: store.mapLabels('metabolite'),
    reactionLabels: store.mapLabels('reaction'),
    showCurrency: store.view.showCurrency,
    currencyLabels: store.view.currencyLabels,
    showCrossFeeding: store.view.showCrossFeeding,
    crossFeeding: store.crossFeeding,
    crossFeedingExplorer: { enabled: false, visibleEvents: [], selectedEvent: null },
    growth: store.growth,
    minHitRadius: 8,
  };
}

const muRange = (state) => ({
  min: state?.min?.toFixed?.(5), max: state?.max?.toFixed?.(5), error: state?.error ?? undefined,
});

const mapSize = (map) => ({
  reactions: map?.reactions?.size ?? 0,
  nodes: map?.nodes?.size ?? 0,
  canvas: map ? `${Math.round(map.width)}x${Math.round(map.height)}` : '-',
});

async function profileMap(store, label) {
  if (!store.map) return;
  const state = renderState(store);
  let svg = '';
  for (let i = 0; i < 3; i++) {
    svg = await step(`${label}: renderMap #${i + 1}`, () => renderMap(store.layout, state), (s) => ({
      svgKB: Math.round(s.length / 1024), elements: (s.match(/<[a-zA-Z]/g) ?? []).length, ...mapSize(store.map),
    }));
  }
  await step(`${label}: map.toJSON + stringify`, () => JSON.stringify(store.map.toJSON()), (s) => ({ jsonKB: Math.round(s.length / 1024) }));
  return svg;
}

/** Same work the SteadyCom Worker does, including the structured-clone round trip. */
function inlineSteadyCom(store) {
  store.runSteadyComTask = async (spec, request) => {
    const snapshot = await step('steadycom: serialize spec', () => structuredClone(serializeSteadyComSpec(spec)));
    const restored = restoreSteadyComSpec(snapshot);
    return request.type === 'range'
      ? steadyCom(restored, request.options ?? {})
      : maxTotalBiomassAt({ ...restored, biomassCap: null }, request.mu, { ...request.options, biomassCap: null });
  };
}

async function profileFBA(problem, label) {
  const ids = [...problem.rxnIds];
  await step(`${label}: FBA (GLPK) #1`, () => problem.optimize(), (r) => ({ status: r.status, objective: r.objectiveValue?.toFixed?.(4), rows: problem.metIds.length, cols: ids.length }));
  await step(`${label}: FBA (GLPK) #2`, () => problem.optimize(), (r) => ({ status: r.status }));
  await step(`${label}: pFBA (GLPK)`, () => problem.parsimonious(), (r) => ({ status: r.status }));
  const sample = Number.isFinite(fvaCount) ? ids.slice(0, fvaCount) : ids;
  await step(`${label}: FVA ${sample.length}/${ids.length} rxns`, () => problem.variability(sample),
    (r) => ({ ranges: r.size }));
}

async function profileSimplex(model, label, timeoutNote) {
  const problem = new FluxProblem(model, { backend: 'simplex' });
  await step(`${label}: FBA (simplex.js) ${timeoutNote}`, () => problem.optimize(), (r) => ({ status: r.status, objective: r.objectiveValue?.toFixed?.(4) }));
}

async function newStore() {
  const store = new Store('.');
  await step('store.init (GLPK + catalog)', () => store.init());
  return store;
}

console.log(`case ${caseName} · node ${process.version} · ${new Date().toISOString()}`);

if (caseName === 'core') {
  const store = await newStore();
  for (const scenario of store.scenarios.slice(0, 3)) {
    await step(`open "${scenario.id}" (load+layout+solve)`, () => store.useScenario(scenario.id), () => ({
      layoutMs: store.status.layoutMs, solveMs: store.status.solveMs, ...mapSize(store.map),
    }));
    await step(`${scenario.id}: re-solve`, () => store.solve({ silent: true }), (r) => ({ status: r?.status }));
    await profileMap(store, scenario.id);
    await profileFBA(store.problem, scenario.id);
  }
}

if (caseName === 'iaf1260') {
  // --file profiles any other model document the same way.
  const file = resolve(root, args.file ?? 'models/published/steadycom-ecoli-auxotrophs/model-Ec1.json');
  const text = await step('read model JSON', () => readFile(file, 'utf8'), (t) => ({ KB: Math.round(t.length / 1024) }));
  const model = await step('parse + new Model', () => new Model(JSON.parse(text)), (m) => ({ reactions: m.reactions.size, metabolites: m.metabolites.size }));
  const sbml = await step('toSBML (export)', () => toSBML(model), (x) => ({ KB: Math.round(x.length / 1024) }));
  await step('parseCOBRASBML (import)', () => parseCOBRASBML(sbml), (d) => ({ reactions: d.reactions?.length }));
  const problem = await step('new FluxProblem', () => new FluxProblem(model));
  await profileFBA(problem, 'iAF1260');
  if (args.simplex === 'true') await profileSimplex(model, 'iAF1260', '(dense basis)');

  const store = await newStore();
  store.models.set('profile-Ec1', model);
  store.scenarios.push({ id: 'profile-iaf1260', name: 'iAF1260', species: [{ id: 'Ec1', model: 'profile-Ec1', abundance: 1 }], medium: {} });
  if (fullMap) {
    await step('open as scenario (full layout + solve)', () => store.useScenario('profile-iaf1260'), () => ({
      layoutMs: store.status.layoutMs, solveMs: store.status.solveMs, ...mapSize(store.map),
    }));
    await profileMap(store, 'iAF1260 full map');
  }
}

if (caseName === 'ecoli4') {
  const store = await newStore();
  inlineSteadyCom(store);
  await step('open steadycom-ecoli-auxotrophs', () => store.usePublishedCommunity('steadycom-ecoli-auxotrophs'), () => ({
    layoutMs: store.status.layoutMs, solveMs: store.status.solveMs, ...mapSize(store.map),
    merged: store.community?.merged.reactions.size,
  }));
  await step('re-solve (equal growth)', () => store.solve({ silent: true }), (r) => ({ status: r?.status }));
  await profileMap(store, 'core overview');
  await profileFBA(store.problem, 'merged 4×iAF1260');
  await step('SteadyCom range (bisection)', () => store.steadyComRange({ recompute: true }), muRange);
  if (fullMap) {
    await step('draw full map (showFullMap)', () => store.showFullMap(), () => mapSize(store.map));
    await profileMap(store, 'ecoli4 full map');
  }
}

if (caseName === 'gut9') {
  const store = await newStore();
  inlineSteadyCom(store);
  await step('open steadycom-gut-nine', () => store.usePublishedCommunity('steadycom-gut-nine'), () => ({
    merged: store.community?.merged.reactions.size, ...mapSize(store.map),
  }));
  await step('draw excerpt (previews)', () => store.drawPublishedPreview('steadycom-gut-nine'), () => mapSize(store.map));
  await profileMap(store, 'gut9 excerpt');
  if (fullMap) {
    // One Figure 4 point runs for tens of minutes, so --full-map measures drawing only.
    await step('draw full map (showFullMap)', () => store.showFullMap(), () => mapSize(store.map));
    await profileMap(store, 'gut9 full map');
  } else {
    await step('SteadyCom range (bisection)', () => store.steadyComRange({ recompute: true }), muRange);
    store.setSteadyComFiber(30);
    const spec = await step('Figure 4 spec (fiber 30)', () => store.steadyComProblem({ recompute: true }), (s) => ({ reactions: s.reactions.size }));
    // The paper's eleven targets: nine biomasses plus B. theta's fiber and carbohydrate uptake.
    const targets = figureTargets({
      species: spec.members.map((m) => ({ id: m.id, biomassReaction: m.biomassReaction })),
      reactions: spec.reactions,
      speciesExchange: spec.metadata.speciesExchange,
    });
    await step(`Figure 4 point: steadyComFVA ×${targets.length} targets`, () => steadyComFVA(spec, {
      targets, upperGuess: 0.1, tolerance: 1e-6, biomassCap: 2,
    }), (r) => ({ status: r.status, growth: r.growthRate?.toFixed?.(5), ranges: r.ranges?.length }));
  }
}

if (caseName === 'zuniga') {
  const store = await newStore();
  await step('open zuniga-2020-phea-rescue', () => store.usePublishedCommunity('zuniga-2020-phea-rescue'), () => ({
    solveMs: store.status.solveMs, ...mapSize(store.map),
  }));
  await step('re-solve', () => store.solve({ silent: true }), (r) => ({ status: r?.status }));
  await profileMap(store, 'zuniga map');
  await profileFBA(store.problem, 'zuniga');
}

console.log(`done · ${lp.calls} LPs · ${round(lp.ms)} ms in GLPK · peak heap ${heapMB()} MB`);
