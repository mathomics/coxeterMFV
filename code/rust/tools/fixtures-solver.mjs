// Reference outputs of src/solver, src/core/steadycom.js and the SteadyCom
// runtime for the Rust port's parity tests.
//
//   node rust/tools/fixtures-solver.mjs            small fixtures (committed)
//   node rust/tools/fixtures-solver.mjs --large    also genome-scale ones (git-ignored)
//
// Writes rust/fixtures/solver/ and, with --large, rust/fixtures/large/solver/.
//
// The JS app uses GLPK (WebAssembly) when it loads and its bundled simplex
// otherwise. Node has no node_modules here, so the vendored copy in
// assets/vendor/ is loaded the way the tests load it. FBA, pFBA and FVA come
// from GLPK. Duals and reduced costs come from the bundled simplex, because the
// GLPK path in fba.js reports row duals only.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { performance } from 'node:perf_hooks';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const out = join(root, 'rust/fixtures/solver');
const outLarge = join(root, 'rust/fixtures/large/solver');
const large = process.argv.includes('--large');
const onlyLarge = process.argv.includes('--only-large');
await mkdir(out, { recursive: true });
if (large || onlyLarge) await mkdir(outLarge, { recursive: true });

globalThis.crypto ??= (await import('node:crypto')).webcrypto;
const factory = (await import(pathToFileURL(join(root, 'assets/vendor/glpk.js')).href)).default;
globalThis.__GLPK_FACTORY__ = factory;
globalThis.__GLPK_WASM__ = await readFile(join(root, 'assets/vendor/glpk.wasm'));

const { initGLPK } = await import(pathToFileURL(join(root, 'src/solver/glpk.js')).href);
if (!await initGLPK()) throw new Error('GLPK did not load');
const { Model } = await import(pathToFileURL(join(root, 'src/core/model.js')).href);
const { CommunityModel } = await import(pathToFileURL(join(root, 'src/core/community.js')).href);
const { FluxProblem } = await import(pathToFileURL(join(root, 'src/solver/fba.js')).href);
const { SparseColumns, solveLP } = await import(pathToFileURL(join(root, 'src/solver/simplex.js')).href);
const { solveBinaryMILP } = await import(pathToFileURL(join(root, 'src/solver/milp.js')).href);
const { steadyCom, maxTotalBiomassAt, parsimoniousAt, steadyComFVA } = await import(pathToFileURL(join(root, 'src/core/steadycom.js')).href);
const runtime = await import(pathToFileURL(join(root, 'src/communities/steadycom-runtime.js')).href);
const figure4 = await import(pathToFileURL(join(root, 'src/communities/gut9-figure4.js')).href);

const mapObj = (m) => Object.fromEntries(m ?? []);
const write = (dir, name, value) => writeFile(join(dir, name), JSON.stringify(value));
const timed = (fn) => { const t = performance.now(); const value = fn(); return [value, performance.now() - t]; };

// FBA family

function fbaCase(model) {
  const glpk = new FluxProblem(model, { backend: 'glpk' });
  const dense = new FluxProblem(model, { backend: 'simplex' });
  const ids = [...glpk.rxnIds];

  const fba = glpk.optimize();
  const fbaDense = dense.optimize();
  const pfba = glpk.parsimonious();
  const pfba90 = glpk.parsimonious({ fraction: 0.9 });
  const loopless = glpk.loopless();
  const fva = mapObj([...glpk.variability(ids, { fraction: 1 })].map(([id, r]) => [id, [r.min, r.max]]));
  const fva90 = mapObj([...glpk.variability(ids, { fraction: 0.9 })].map(([id, r]) => [id, [r.min, r.max]]));
  const min = glpk.optimize({ maximize: false });
  return {
    reactions: ids.length,
    metabolites: glpk.metIds.length,
    objective: glpk.objective,
    fba: { status: fba.status, objectiveValue: fba.objectiveValue, fluxes: mapObj(fba.fluxes) },
    fbaMin: { status: min.status, objectiveValue: min.objectiveValue },
    simplex: {
      status: fbaDense.status, objectiveValue: fbaDense.objectiveValue,
      shadowPrices: mapObj(fbaDense.shadowPrices), reducedCosts: mapObj(fbaDense.reducedCosts),
      reducedCostScale: fbaDense.reducedCostScale,
    },
    pfba: { status: pfba.status, objectiveValue: pfba.objectiveValue, totalFlux: pfba.totalFlux ?? null },
    pfba90: { status: pfba90.status, objectiveValue: pfba90.objectiveValue, totalFlux: pfba90.totalFlux ?? null },
    loopless: {
      status: loopless.status, objectiveValue: loopless.objectiveValue, provenOptimal: loopless.provenOptimal,
      binaryVariables: loopless.binaryVariables, looplessInternalReactions: loopless.looplessInternalReactions,
    },
    fva,
    fva90,
  };
}

const models = new Map();
const modelIds = ['cyano_core_mc', 'ecoli_core_mc', 'lactobacillus_core_mc', 'methanogen_core_mc', 'yeast_core_mc'];
for (const id of modelIds) {
  models.set(id, new Model(JSON.parse(await readFile(join(root, 'models', `${id}.json`), 'utf8'))));
}

const cases = [];
if (!onlyLarge) {
  for (const [id, model] of models) {
    const [value, ms] = timed(() => fbaCase(model));
    await write(out, `fba-model-${id}.json`, { kind: 'model', model: id, ...value, ms });
    cases.push(`model-${id}`);
    console.log(`model ${id}: ${Math.round(ms)} ms`);
  }

  const { scenarios } = JSON.parse(await readFile(join(root, 'models/scenarios.json'), 'utf8'));
  for (const s of scenarios) {
    if (s.loader) continue; // fetched from the internet (Yeast9)
    const medium = {};
    for (const [base, limit] of Object.entries(s.medium ?? {})) medium[base] = { lower: -Math.abs(Number(limit) || 0), upper: 1000 };
    const objectives = [['equal-growth', null], ['sum-biomass', null]];
    for (const [objective, productTarget] of [...objectives, ['product', 'ac']]) {
      const spec = {
        name: s.name, objective, productTarget, medium,
        species: s.species.map((sp) => ({ id: sp.id, model: models.get(sp.model), abundance: sp.abundance ?? 1 / s.species.length, enabled: sp.enabled !== false })),
      };
      const c = new CommunityModel(spec);
      // A product objective is only defined where the medium carries it.
      if (objective === 'product' && !c.merged.reactions.has(`EX_${productTarget}_s`)) continue;
      const [value, ms] = timed(() => fbaCase(c.merged));
      await write(out, `fba-scenario-${s.id}-${objective}.json`, {
        kind: 'scenario', scenario: s.id,
        spec: { ...spec, species: s.species.map((sp, i) => ({ ...spec.species[i], model: sp.model })) },
        ...value, ms,
      });
      cases.push(`scenario-${s.id}-${objective}`);
      console.log(`scenario ${s.id} ${objective}: ${Math.round(ms)} ms`);
    }
  }
}

// The JS unit-test problems

function lpCases() {
  const out = [];
  const add = (name, rows, cols, entries, b, c, lb, ub, maximize = true) => {
    const A = new SparseColumns(rows, cols);
    for (const [i, j, v] of entries) A.set(i, j, v);
    const res = solveLP({ A, b, c, lb, ub, maximize });
    out.push({
      name, rows, cols, entries, b, c,
      lb: lb.map((v) => (Number.isFinite(v) ? v : v > 0 ? 'inf' : '-inf')),
      ub: ub.map((v) => (Number.isFinite(v) ? v : v > 0 ? 'inf' : '-inf')),
      maximize, status: res.status, objective: res.objective,
      x: res.x ? Array.from(res.x) : null,
      duals: res.duals ? Array.from(res.duals) : null,
      reducedCosts: res.reducedCosts ? Array.from(res.reducedCosts) : null,
    });
  };
  add('bounded', 1, 2, [[0, 0, 1], [0, 1, 1]], [4], [3, 2], [0, 0], [3, 3]);
  add('minimise', 1, 2, [[0, 0, 1], [0, 1, 1]], [4], [3, 2], [0, 0], [3, 3], false);
  add('infeasible', 1, 2, [[0, 0, 1], [0, 1, 1]], [10], [1, 1], [0, 0], [3, 3]);
  add('unbounded', 1, 2, [[0, 0, 1], [0, 1, -1]], [0], [1, 1], [0, 0], [Infinity, Infinity]);
  add('free', 1, 2, [[0, 0, 2], [0, 1, 1]], [1], [1, -1], [-5, -Infinity], [5, Infinity]);
  add('duals', 1, 2, [[0, 0, 1], [0, 1, 1]], [1], [1, 0], [0, 0], [10, 10]);
  add('duals-min', 1, 2, [[0, 0, 1], [0, 1, 1]], [1], [1, 2], [0, 0], [10, 10], false);
  // Random 2-D problems, as test/simplex.test.mjs draws them.
  let seed = 7;
  const rand = () => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed / 0x7fffffff; };
  for (let trial = 0; trial < 60; trial++) {
    const a0 = rand() * 4 - 2, a1 = rand() * 4 - 2;
    if (Math.abs(a1) < 0.2 || Math.abs(a0) < 0.2) continue;
    const bb = rand() * 4 - 2, c0 = rand() * 4 - 2, c1 = rand() * 4 - 2;
    add(`random-${trial}`, 1, 2, [[0, 0, a0], [0, 1, a1]], [bb], [c0, c1], [-3, -3], [3, 3]);
  }
  const milp = (name, entries, cols, b, c, ub, binaries) => {
    const A = new SparseColumns(1, cols);
    for (const [i, j, v] of entries) A.add(i, j, v);
    const res = solveBinaryMILP({ A, b: Float64Array.from(b), c: Float64Array.from(c), lb: new Float64Array(cols),
      ub: Float64Array.from(ub), maximize: true }, binaries, { nodeLimit: 100, timeLimitMs: 1000 });
    return { name, entries, cols, b, c, ub: ub.map((v) => (Number.isFinite(v) ? v : 'inf')), binaries,
      status: res.status, objective: res.objective, provenOptimal: res.provenOptimal };
  };
  return {
    lp: out,
    milp: [
      milp('no-binary-point', [[0, 0, 1], [0, 1, 2]], 2, [2.5], [1, 1], [1, 1], [0, 1]),
      milp('integer-optimum', [[0, 0, 1], [0, 1, 1], [0, 2, 1]], 3, [1.5], [1, 1, 0], [1, 1, Infinity], [0, 1]),
    ],
  };
}
if (!onlyLarge) await write(out, 'lp-cases.json', lpCases());

// SteadyCom, small

const GLUCOSE_BUDGET = 10;
function memberDoc(id, makes, needs) {
  return {
    id, name: id,
    compartments: { e: 'Extracelular', c: 'Citosol' },
    metabolites: [
      { id: 'glc_e', name: 'glucose', compartment: 'e' }, { id: 'glc_c', name: 'glucose', compartment: 'c' },
      { id: `${makes}_c`, name: makes, compartment: 'c' }, { id: `${makes}_e`, name: makes, compartment: 'e' },
      { id: `${needs}_c`, name: needs, compartment: 'c' }, { id: `${needs}_e`, name: needs, compartment: 'e' },
    ],
    reactions: [
      { id: 'EX_glc_e', metabolites: { glc_e: -1 }, lower_bound: -GLUCOSE_BUDGET, upper_bound: 1000 },
      { id: 'GLCt', metabolites: { glc_e: -1, glc_c: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: 'BIOMASS', metabolites: { glc_c: -1, [`${needs}_c`]: -1 }, lower_bound: 0, upper_bound: 100 },
      { id: `MK_${makes}`, metabolites: { glc_c: -1, [`${makes}_c`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `T_${makes}`, metabolites: { [`${makes}_c`]: -1, [`${makes}_e`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `EX_${makes}_e`, metabolites: { [`${makes}_e`]: -1 }, lower_bound: 0, upper_bound: 1000 },
      { id: `T_${needs}`, metabolites: { [`${needs}_e`]: -1, [`${needs}_c`]: 1 }, lower_bound: 0, upper_bound: 100 },
      { id: `EX_${needs}_e`, metabolites: { [`${needs}_e`]: -1 }, lower_bound: -100, upper_bound: 1000 },
    ],
    objective: { BIOMASS: 1 },
  };
}
const steadyResult = (r) => ({
  status: r.status, growthRate: r.growthRate, rangeLower: r.rangeLower ?? null, rangeUpper: r.rangeUpper ?? null,
  upperBound: r.upperBound ?? null, resolution: r.resolution ?? null, zeroFeasible: r.zeroFeasible ?? null,
  iterations: r.iterations, abundances: mapObj(r.abundances), message: r.message,
});
const atResult = (r) => ({
  status: r.status, feasible: r.feasible, total: r.total, abundances: mapObj(r.abundances),
  totalFlux: r.totalFlux ?? null, objective: r.objective ?? null,
});

if (!onlyLarge) {
  const pair = () => [
    { id: 'A', model: new Model(memberDoc('A', 'x', 'y')) },
    { id: 'B', model: new Model(memberDoc('B', 'y', 'x')) },
  ];
  const spec = runtime.steadyComSpec(pair(), { glc: GLUCOSE_BUDGET }, 1);
  const small = {
    members: { A: memberDoc('A', 'x', 'y'), B: memberDoc('B', 'y', 'x') },
    exchangeFor: spec.members.map((m) => m.exchangeFor),
    fixed: steadyResult(steadyCom(spec, { upperGuess: 20 })),
    capped: steadyResult(steadyCom(spec, { upperGuess: 20, biomassCap: 2 })),
    appOptions: steadyResult(steadyCom(spec, { upperGuess: 1, tolerance: 1e-5, biomassCap: 2, timeLimitSeconds: 30 })),
    at: [0.5, 1, 2, 4, 5, 10].map((mu) => ({ mu, fixed: atResult(maxTotalBiomassAt(spec, mu)), capped: atResult(maxTotalBiomassAt(spec, mu, { biomassCap: 2 })) })),
    parsimonious: [1, 2.5, 5].map((mu) => ({ mu, ...atResult(parsimoniousAt(spec, mu)) })),
    fva: (() => {
      const r = steadyComFVA(spec, { upperGuess: 20, targets: [
        { name: 'bio_A', reactions: { 'A\u0000BIOMASS': 1 } }, { name: 'X_B', biomass: { B: 1 } },
        { name: 'glc', reactions: { 'A\u0000EX_glc_e': 1, 'B\u0000EX_glc_e': 1 } },
      ] });
      return { status: r.status, growthRate: r.growthRate, ranges: r.ranges.map((x) => ({ name: x.name, min: x.min, max: x.max, minRelaxed: x.minRelaxed, maxRelaxed: x.maxRelaxed })), relaxations: r.relaxations };
    })(),
  };
  await write(out, 'steadycom-small.json', small);

  // The two-member joint community of test/steadycom-joint-adapter.test.mjs.
  const tinyDocs = ['A', 'B'].map((id) => ({
    id, compartments: { e: 'Extracellular', c: 'Cytosol', u: 'Shared medium' },
    metabolites: [
      { id: `a[e_${id}]`, compartment: 'e' }, { id: `a[c_${id}]`, compartment: 'c' }, { id: 'a[u]', compartment: 'u' },
    ],
    reactions: [
      { id: `EX_${id}`, metabolites: { [`a[e_${id}]`]: -1, 'a[u]': 1 }, lower_bound: -20, upper_bound: 1000 },
      { id: `T_${id}`, metabolites: { [`a[e_${id}]`]: -1, [`a[c_${id}]`]: 1 }, lower_bound: 0, upper_bound: 1000 },
      { id: `BIO_${id}`, metabolites: { [`a[c_${id}]`]: -1 }, lower_bound: 0, upper_bound: 1000 },
    ], objective: { [`BIO_${id}`]: 1 },
  }));
  const csv = [
    'reaction,species,lower_bound,upper_bound,objective_coefficient',
    ...['A', 'B'].flatMap((id) => [`EX_${id},${id},-20,1000,0`, `T_${id},${id},0,1000,0`, `BIO_${id},${id},0,1000,1`]),
    'UT_a(u),comm,0,10,0', 'EX_a(u),comm,0,10000,0',
  ].join('\n');
  const tiny = runtime.jointSteadyComSpec(tinyDocs.map((doc) => ({ id: doc.id, model: new Model(doc) })), csv);
  const projections = [];
  for (const [abundances, opts] of [[{ A: 0.25, B: 0.75 }, {}], [{ A: 1, B: 0 }, {}],
    [{ A: 0.25, B: 0.75 }, { objective: 'sum-biomass' }], [{ A: 0.25, B: 0.75 }, { objective: 'product', productTarget: 'a' }]]) {
    const p = runtime.jointCommunityProjection(tiny, abundances, opts);
    const r = new FluxProblem(p.model, { backend: 'glpk' }).optimize();
    projections.push({ abundances, opts, model: p.model.toJSON(), status: r.status, objectiveValue: r.objectiveValue,
      biomassIds: mapObj(p.biomassIds), reactionOwner: mapObj(p.reactionOwner), metaboliteOwner: mapObj(p.metaboliteOwner),
      transfersByBase: mapObj(p.transfersByBase), communityExchangeIds: mapObj(p.communityExchangeIds),
      activeSpecies: p.activeSpecies.map((m) => m.id), speciesCompartments: mapObj(p.speciesCompartments) });
  }
  await write(out, 'steadycom-joint-tiny.json', {
    docs: tinyDocs, csv,
    range: steadyResult(steadyCom(tiny, { upperGuess: 1, tolerance: 1e-5, biomassCap: 2 })),
    at: atResult(maxTotalBiomassAt(tiny, 0.5)),
    parsimonious: atResult(parsimoniousAt(tiny, 0.5)),
    projections,
  });

  // The published assets: deterministic transformations only (no LPs).
  const ecoliDir = join(root, 'models/published/steadycom-ecoli-auxotrophs');
  const ec1 = new Model(JSON.parse(await readFile(join(ecoliDir, 'model-Ec1.json'), 'utf8')));
  const media = await readFile(join(ecoliDir, 'media.csv'), 'utf8');
  const gutDir = join(root, 'models/published/steadycom-gut-nine');
  const gutManifest = JSON.parse(await readFile(join(gutDir, 'manifest.json'), 'utf8'));
  const gutMembers = await Promise.all(gutManifest.members.map(async ({ id, model }) => ({
    id, model: new Model(JSON.parse(await readFile(join(gutDir, model), 'utf8'))),
  })));
  const gutBounds = await readFile(join(gutDir, 'bounds.csv'), 'utf8');
  const joint = runtime.jointSteadyComSpec(gutMembers, gutBounds, { fiber: { bacteroides: 10, faecalibacterium: 5 } });
  const projection = runtime.jointCommunityProjection(joint, new Map(joint.members.map((m) => [m.id, 1 / 9])));
  const targets = figure4.figureTargets({
    species: joint.members.map((m) => ({ id: m.id, biomassReaction: m.biomassReaction })),
    reactions: joint.reactions, speciesExchange: joint.metadata.speciesExchange,
  });
  const reactionSample = ['ATPM_Bt', 'EX_12ethd[u]tr_Bt', 'UT_glc-D(u)', 'EX_glc-D(u)', 'glcIn_Er', 'Biomass_BT_v2_Bt'];
  await write(out, 'runtime-published.json', {
    ecoli: {
      exchangeIndex: runtime.exchangeIndex(ec1),
      media: runtime.parseMediaCSV(media, [{ id: 'Ec1', model: ec1 }]),
      mediaBare: runtime.parseMediaCSV(media),
    },
    gut9: {
      members: joint.members,
      reactions: joint.reactions.size,
      metabolites: joint.metabolites.size,
      reactionSample: Object.fromEntries(reactionSample.map((id) => [id, joint.reactions.get(id)])),
      metaboliteSample: Object.fromEntries(['glc-D[u]', 'ad[u]', '12ethd[e_Bt]'].map((id) => [id, joint.metabolites.get(id)])),
      constraints: joint.constraints,
      speciesExchangeCount: joint.metadata.speciesExchange.size,
      speciesExchangeSample: Object.fromEntries([...joint.metadata.speciesExchange].slice(0, 20)),
      bases: joint.metadata.bases.size,
      communityUptakeSample: Object.fromEntries([...joint.metadata.communityUptake].slice(0, 20)),
      communityExportSample: Object.fromEntries([...joint.metadata.communityExport].slice(0, 20)),
      targets,
      projection: {
        reactions: projection.model.reactions.size,
        metabolites: projection.model.metabolites.size,
        compartments: projection.model.compartmentIds(),
        transfers: projection.transfersByBase.size,
        transfersGlc: projection.transfersByBase.get('glc-D'),
        objective: projection.model.objectiveReactions(),
        growth: projection.model.reactions.get('COMMUNITY_GROWTH')?.toJSON(),
        sampleReactions: Object.fromEntries(['Er@glcIn_Er', 'Bt@EX_12ethd[u]tr_Bt', 'UT_glc-D(u)', 'Bt@Biomass_BT_v2_Bt']
          .map((id) => [id, projection.model.reactions.get(id)?.toJSON()])),
        reactionOwnerSample: Object.fromEntries(['Er@glcIn_Er', 'UT_glc-D(u)', 'Bt@Biomass_BT_v2_Bt', 'Bt@ATPM_Bt', 'COMMUNITY_GROWTH']
          .map((id) => [id, projection.reactionOwner.get(id)])),
        communityExchangeGlc: projection.communityExchangeIds.get('glc-D'),
      },
    },
  });
  console.log('wrote steadycom and runtime fixtures');
  await writeFile(join(out, 'index.json'), JSON.stringify({ cases }, null, 1));
  console.log(`wrote ${cases.length} FBA cases to ${out}`);
}

// Large, genome-scale fixtures

if (large || onlyLarge) {
  // iAF1260 from the COBRA Toolbox .mat, converted once to COBRA JSON.
  const { readMat5 } = await import(pathToFileURL(join(root, 'src/import/mat5.js')).href);
  const matPath = join(process.env.HOME, 'Documents/GitHub/cobratoolbox/test/models/mat/iAF1260.mat');
  let iaf = null;
  try {
    const vars = await readMat5(await readFile(matPath));
    const m = vars.iAF1260;
    const text = (v) => (Array.isArray(v) ? v.join('') : String(v ?? ''));
    const rxns = m.rxns.map(text), mets = m.mets.map(text);
    const rows = mets.length;
    const doc = {
      id: 'iAF1260', name: text(m.description) || 'iAF1260',
      metabolites: mets.map((id, i) => ({ id, name: text(m.metNames?.[i] ?? id), formula: text(m.metFormulas?.[i] ?? ''),
        charge: Number(m.metCharge?.[i] ?? 0) || 0, compartment: /_([a-z])$/.exec(id)?.[1] ?? 'c' })),
      reactions: rxns.map((id, j) => {
        const stoich = {};
        for (let i = 0; i < rows; i++) { const v = m.S[i + j * rows]; if (v) stoich[mets[i]] = v; }
        return { id, name: text(m.rxnNames?.[j] ?? id), metabolites: stoich, lower_bound: m.lb[j], upper_bound: m.ub[j],
          objective_coefficient: m.c[j], subsystem: text(m.subSystems?.[j] ?? ''), gene_reaction_rule: text(m.grRules?.[j] ?? '') };
      }),
    };
    await writeFile(join(root, 'rust/fixtures/large/iAF1260.json'), JSON.stringify(doc));
    iaf = new Model(doc);
    console.log(`converted iAF1260.mat: ${iaf.reactions.size} reactions`);
  } catch (error) {
    console.log(`iAF1260.mat not converted: ${error.message}`);
  }

  const bench = async (name, model) => {
    const problem = new FluxProblem(model, { backend: 'glpk' });
    const ids = [...problem.rxnIds];
    const [fba, fbaMs] = timed(() => problem.optimize());
    const [pfba, pfbaMs] = timed(() => problem.parsimonious());
    const [fva, fvaMs] = timed(() => problem.variability(ids));
    await write(outLarge, `fba-${name}.json`, {
      model: name, reactions: ids.length, metabolites: problem.metIds.length,
      fba: { status: fba.status, objectiveValue: fba.objectiveValue, fluxes: mapObj(fba.fluxes) },
      pfba: { status: pfba.status, objectiveValue: pfba.objectiveValue, totalFlux: pfba.totalFlux },
      fva: mapObj([...fva].map(([id, r]) => [id, [r.min, r.max]])),
      ms: { fba: fbaMs, pfba: pfbaMs, fva: fvaMs },
    });
    console.log(`${name}: FBA ${Math.round(fbaMs)} ms, pFBA ${Math.round(pfbaMs)} ms, FVA ${(fvaMs / 1000).toFixed(1)} s`);
  };
  const ecoliDir = join(root, 'models/published/steadycom-ecoli-auxotrophs');
  const manifest = JSON.parse(await readFile(join(ecoliDir, 'manifest.json'), 'utf8'));
  const members = await Promise.all(manifest.members.map(async ({ id, model }) => ({
    id, model: new Model(JSON.parse(await readFile(join(ecoliDir, model), 'utf8'))),
  })));
  if (!process.argv.includes('--skip-fva')) {
    if (iaf) await bench('iAF1260', iaf);
    await bench('Ec1', members[0].model);
  }

  // SteadyCom on the four auxotrophic E. coli, with the options the app uses.
  const medium = runtime.parseMediaCSV(await readFile(join(ecoliDir, 'media.csv'), 'utf8'), members);
  const spec = runtime.steadyComSpec(members, medium, 1);
  const [range, rangeMs] = timed(() => steadyCom(spec, { upperGuess: 1, tolerance: 1e-5, biomassCap: 2, timeLimitSeconds: 30 }));
  console.log(`ecoli4 range: ${range.status} mu=${range.rangeUpper} in ${(rangeMs / 1000).toFixed(1)} s, ${range.iterations} LPs`);
  const mu = 0.7;
  const [at, atMs] = timed(() => maxTotalBiomassAt({ ...spec, biomassCap: null }, mu, { biomassCap: null, timeLimitSeconds: 60 }));
  console.log(`ecoli4 fixed mu=${mu}: ${at.status} ${at.feasible} in ${(atMs / 1000).toFixed(1)} s`);
  const [lean, leanMs] = timed(() => parsimoniousAt(spec, mu, { timeLimitSeconds: 600 }));
  console.log(`ecoli4 parsimonious mu=${mu}: ${lean.status} total flux ${lean.totalFlux} in ${(leanMs / 1000).toFixed(1)} s`);
  await write(outLarge, 'steadycom-ecoli4.json', {
    medium,
    range: { ...steadyResult(range), fluxes: mapObj(range.fluxes) },
    mu,
    at: { ...atResult(at), fluxes: mapObj(at.fluxes) },
    parsimonious: { ...atResult(lean), fluxes: mapObj(lean.fluxes) },
    ms: { range: rangeMs, at: atMs, parsimonious: leanMs },
  });
}
