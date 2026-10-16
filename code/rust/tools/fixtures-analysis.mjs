// Reference outputs of the omics, analysis and published-community modules for
// the Rust port's parity tests (crate coxeter-analysis).
//
//   node rust/tools/fixtures-analysis.mjs             all fixtures
//   node rust/tools/fixtures-analysis.mjs --catalog   also rewrite the crate's
//                                                     copy of the registry
//
// Writes rust/fixtures/analysis/ (small, committed):
//   overlay.json, community-omics.json   omics tables, GPR scores, colours and
//                                        mapping reports on the bundled scenario
//                                        communities (synthetic gene rules: the
//                                        bundled models carry none)
//   explorer.json                        the cross-feeding explorer on every
//                                        bundled scenario's FBA fluxes
//   gimme.json                           GIMME on toy, core and community models
//   published.json                       the registry, as the JS holds it
//   stolyar.json                         the CellNetAnalyzer reader on the JS
//                                        test's miniature, its errors, and FBA
//                                        on the bundled joint model
//   steadycom-ecoli.json (+ .xml)        buildCommunity on a miniature iAF1260
//   gut9.json (+ .xlsx)                  importGut9 on a miniature model9.xlsx
//   zuniga.json (+ toy .mat files)       the Zuñiga converter, scenarios and
//                                        loader on synthetic MAT files
//
// Writes rust/fixtures/large/analysis/ (git-ignored):
//   stolyar-reconstructed/               the four CellNetAnalyzer files rebuilt
//                                        from the bundled joint model, and what
//                                        the JS reads from them
//   yeast9-omics.json                    omics scores on yeast-GEM.xml, when the
//                                        user's copy is on disk
globalThis.crypto ??= (await import('node:crypto')).webcrypto;

import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { homedir } from 'node:os';
import { createHash } from 'node:crypto';
import { deflateRawSync } from 'node:zlib';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const out = join(root, 'rust/fixtures/analysis');
const large = join(root, 'rust/fixtures/large/analysis');
await mkdir(out, { recursive: true });
await mkdir(join(large, 'stolyar-reconstructed'), { recursive: true });

globalThis.__GLPK_FACTORY__ = (await import(pathToFileURL(join(root, 'assets/vendor/glpk.js')).href)).default;
globalThis.__GLPK_WASM__ = await readFile(join(root, 'assets/vendor/glpk.wasm'));
const src = (p) => import(pathToFileURL(join(root, 'src', p)).href);
const { initGLPK } = await src('solver/glpk.js');
if (!await initGLPK()) throw new Error('GLPK did not load');

const { Model } = await src('core/model.js');
const { CommunityModel } = await src('core/community.js');
const { FluxProblem } = await src('solver/fba.js');
const { toSBML } = await src('core/sbml-write.js');
const { parseCOBRASBML } = await src('core/sbml.js');
const { readMat5 } = await src('import/mat5.js');
const { steadyCom, maxTotalBiomassAt } = await src('core/steadycom.js');
const overlay = await src('omics/overlay.js');
const communityOmics = await src('omics/community-omics.js');
const { runGIMME } = await src('omics/gimme.js');
const explorer = await src('analysis/cross-feeding-explorer.js');
const published = await src('communities/published.js');
const stolyar = await src('communities/stolyar.js');
const ecoli = await src('communities/steadycom-ecoli.js');
const gut9 = await src('communities/gut9.js');
const zuniga = await src('communities/zuniga.js');

const write = (dir, name, value) => writeFile(join(dir, name), JSON.stringify(value));
const mapObj = (m) => Object.fromEntries(m ?? []);
const attempt = (fn) => {
  try { return { ok: fn() }; } catch (error) { return { error: error.message }; }
};
const attemptAsync = async (fn) => {
  try { return { ok: await fn() }; } catch (error) { return { error: error.message }; }
};
const modelJSON = (m) => JSON.parse(JSON.stringify(m.toJSON()));
/** A small deterministic generator, so the fixtures do not change between runs. */
function generator(seed) {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const bundled = new Map();
const loadModel = async (id) => {
  if (!bundled.has(id)) bundled.set(id, JSON.parse(await readFile(join(root, 'models', `${id}.json`), 'utf8')));
  return structuredClone(bundled.get(id));
};
const { scenarios } = JSON.parse(await readFile(join(root, 'models/scenarios.json'), 'utf8'));
const usable = scenarios.filter((s) => !s.loader);

// Synthetic gene rules

// The bundled models carry no gene rules, so each reaction gets one built from
// its position: no rule, one gene, AND, OR, nested, upper-case operators, and a
// gene shared by every model (the dataset's species column tells them apart).
function syntheticRules(modelId, doc) {
  const short = modelId.split('_')[0];
  const g = (k) => `${short}_g${k % 40}`;
  const rules = {};
  doc.reactions.forEach((r, i) => {
    const rule = [
      '',
      g(i),
      `${g(i)} and ${g(i + 1)}`,
      `${g(i)} or ${g(i + 3)}`,
      `(${g(i)} and ${g(i + 1)}) or ${g(i + 2)}`,
      `shared_${i % 5} AND ${g(i)}`,
      `(${g(i)} or (${g(i + 5)} and ${g(i + 6)}))`,
    ][i % 7];
    rules[r.id] = rule;
    r.gene_reaction_rule = rule;
  });
  return rules;
}

/** A wide omics table for a scenario's species: per-species and global rows. */
function syntheticCSV(species, rnd) {
  const lines = ['species,gene,C1,T2,3'];
  const value = () => (Math.round(rnd() * 100000) / 100).toString();
  for (const sp of species) {
    const short = sp.model.split('_')[0];
    for (let k = 0; k < 40; k += 1) {
      if (k % 13 === 12) continue; // unmeasured gene
      const who = k % 3 === 0 ? '' : sp.id;
      const t2 = k % 11 === 0 ? '' : (k % 17 === 0 ? 'NA' : value());
      lines.push(`${who},${short}_g${k},${value()},${t2},${value()}`);
    }
  }
  for (let k = 0; k < 5; k += 1) lines.push(`${k % 2 ? species[0].id : ''},shared_${k},${value()},${value()},${value()}`);
  lines.push(`ghost,${species[0].model.split('_')[0]}_g1,1,2,3`);
  return `${lines.join('\n')}\n`;
}

function datasetJSON(d) {
  if (!d) return null;
  return { ...overlay.serializeOmicsDataset(d), keys: [...d.values.keys()] };
}

function scaleJSON(s) {
  return { ...s, colors: mapObj(s.colors) };
}

async function scenarioCommunity(s, objective = 'equal-growth') {
  const medium = {};
  for (const [base, limit] of Object.entries(s.medium ?? {})) medium[base] = { lower: -Math.abs(Number(limit) || 0), upper: 1000 };
  const models = new Map();
  const gprs = {};
  for (const sp of s.species) {
    if (models.has(sp.model)) continue;
    const doc = await loadModel(sp.model);
    gprs[sp.model] = syntheticRules(sp.model, doc);
    models.set(sp.model, new Model(doc));
  }
  const spec = {
    name: s.name, objective, productTarget: null, medium,
    species: s.species.map((sp) => ({ id: sp.id, model: models.get(sp.model), abundance: sp.abundance ?? 1 / s.species.length, enabled: sp.enabled !== false })),
  };
  return {
    community: new CommunityModel(spec),
    models,
    gprs,
    speciesConfig: s.species.map((sp) => ({ id: sp.id, file: sp.model })),
    spec: { name: s.name, objective, productTarget: null, medium, species: s.species.map((sp, i) => ({ ...spec.species[i], model: sp.model })) },
  };
}

// overlay.js

const csvCases = [
  ['wide', 'species,gene,Control,Treatment\nA,g1,10,20\nA,g2,4,2\nB,g1,8,16\n', { dataType: 'gene' }],
  ['crlf-bom-quotes', '﻿Gene ID,"Cond, 1",Cond.2\r\n"g""1",1.5,\r\ng2,,3e2\r\n\r\n', { source: 'x.csv' }],
  ['no-id-header', 'name,a,b\nx,1,2\ny,z,3\n', {}],
  ['empty-header', 'id,,b\ng1,1,2\n', {}],
  ['numeric-headers', 'id,2,1,b\ng1,1,2,3\ng2,,5,\n', {}],
  ['duplicates', 'id,v\ng1,1\ng1,2\n g1 ,3\n', {}],
  ['species-blank', 'organism,locus_tag,v\n,g1,1\nB,g1,2\n', {}],
  ['js-numbers', 'id,v\na,0x10\nb,Infinity\nc,1e999\nd, 7 \ne, 5 \nf,-.5\ng,+3\nh,1_0\ni,0b11\nj,5.\n', {}],
  ['no-numeric', 'id,v\ng1,x\n', {}],
  ['one-row', 'id,v\n', {}],
  ['no-usable', 'id,v,w\n,1,2\n', {}],
  ['quoted-newline', 'id,v\n"g\n1",4\n', {}],
  ['protein', 'protein_id,MAG,c1\nP1,m1,3\n', { dataType: 'protein' }],
  ['whitespace-rows', 'id,v\n , \ng1,2\n', {}],
  ['dashes-and-dots', 'Locus-Tag,Genome.Id,x y\nl1,G,2\n', {}],
  ['no-trailing-newline', 'id,a\ng1,1', {}],
  ['tabs-are-not-separators', 'id\tv\ng1\t2\n', {}],
];
const parseCases = csvCases.map(([name, text, options]) => {
  const r = attempt(() => overlay.parseOmicsCSV(text, options));
  return { name, text, options, ...(r.error ? { error: r.error } : { dataset: datasetJSON(r.ok), rowCount: r.ok.rowCount }) };
});

const rawDatasets = [
  null, 5, [], { conditions: 'x' }, { conditions: [] }, { conditions: ['a'], rows: [] },
  { conditions: ['a', '2'], rows: [{ id: 'g1', values: { a: 1, 2: 3 } }, { id: '', values: { a: 1 } }, { id: 7, species: null, values: { a: 5 } }, { id: 'g1', species: ' S ', values: { a: 2 } }], source: 's.csv', speciesColumn: 'sp' },
  { conditions: ['c'], rows: [{ species: 'A', id: 'g', values: { c: 1 } }, { id: 'h' }, null, { id: 'k', values: { c: -2.5 } }], dataType: null, idColumn: 'gene' },
];
const deserializeCases = rawDatasets.map((raw) => ({ raw, dataset: datasetJSON(overlay.deserializeOmicsDataset(raw)) }));

const gprRules = ['(g1 and g2) or g3', 'g1 AND g2 Or g3', 'g1 and missing', 'g1 or missing', 'and g1', 'g1 and', '((g1)', ')',
  '', '   ', 'g1 g2', '(g1 or g2) and (g3 or (g4 and g5))', 'or or g1', 'g1 and (g2', 'g1 and ) g2', '(g1 and g2',
  'missing and missing2 or g4', 'g1 or g2 or g3 and g4', '()', 'g1 and or g2', 'g1 and\tg2', 'g1 and nan'];
const gprValues = { g1: 8, g2: 3, g3: 5, g4: 7, g5: 1, nan: NaN };
const gprCases = gprRules.map((rule) => {
  const ast = overlay.parseGPR(rule);
  return { rule, ast: ast ?? null, value: overlay.evaluateGPR(ast, (id) => gprValues[id]) };
});

const compareInputs = [
  [{ R1: 3, R2: 7, R3: -1, R4: 0 }, { R1: 7, R2: 3, R3: 4, R5: 1, R4: 0 }],
  [{ a: 0, b: -0.5, c: 1e-12 }, { a: 0, b: 2, c: 0 }],
];
const compareOptions = [{ metric: 'log2fc', pseudocount: 1 }, { metric: 'difference' }, { metric: 'log2fc', pseudocount: 0 },
  { metric: 'log2fc', pseudocount: -1 }, { metric: 'log2fc', pseudocount: 'x' }, { metric: 'other', pseudocount: 0.5 }];
const encodeOptions = (o) => ({ ...o, pseudocount: typeof o.pseudocount === 'string' ? 'NaN' : o.pseudocount });
const compareCases = compareInputs.flatMap(([a, b]) => compareOptions.map((options) => ({
  a, b, options: encodeOptions(options),
  result: mapObj(overlay.compareReactionScores(new Map(Object.entries(a)), new Map(Object.entries(b)), options)),
})));

const scaleInputs = [
  { R1: 1, R2: 10 }, { R1: -2, R2: 0, R3: 2 }, {}, { a: 5 }, { a: 3, b: 3, c: 3 },
  Object.fromEntries(Array.from({ length: 41 }, (_, i) => [`r${i}`, Math.round((i * 37 % 41 - 20) * 7.3) / 10])),
];
const encodeScores = (o) => Object.fromEntries(Object.entries(o).map(([k, v]) => [k, Number.isFinite(v) ? v : String(v)]));
const scaleCases = [];
for (const input of [...scaleInputs, { a: 1, b: NaN, c: Infinity, d: -Infinity, e: 4 }]) {
  const scores = new Map(Object.entries(input));
  for (const dark of [false, true]) {
    scaleCases.push({
      scores: encodeScores(input), dark,
      sequential: scaleJSON(overlay.sequentialOmicsColors(scores, { dark })),
      diverging: scaleJSON(overlay.divergingOmicsColors(scores, { dark })),
    });
  }
}

const omicsScenarios = [];
const rnd = generator(20261013);
for (const s of usable) {
  if (!['todas', 'consorcio-anaerobio', 'metanogenesis', 'solo-ecoli'].includes(s.id)) continue;
  const { community, models, gprs, speciesConfig, spec } = await scenarioCommunity(s);
  const csv = syntheticCSV(s.species, rnd);
  const dataset = overlay.parseOmicsCSV(csv, { source: `${s.id}.csv`, dataType: 'gene' });
  const scores = {};
  for (const condition of [...dataset.conditions, 'absent']) {
    scores[condition] = mapObj(overlay.reactionOmicsScores(community, speciesConfig, models, dataset, condition));
  }
  const a = overlay.reactionOmicsScores(community, speciesConfig, models, dataset, 'C1');
  const b = overlay.reactionOmicsScores(community, speciesConfig, models, dataset, 'T2');
  const fc = overlay.compareReactionScores(a, b, { metric: 'log2fc', pseudocount: 1 });
  const diff = overlay.compareReactionScores(a, b, { metric: 'difference' });
  omicsScenarios.push({
    scenario: s.id, spec, gprs, speciesConfig, csv,
    dataset: datasetJSON(dataset),
    scores,
    log2fc: mapObj(fc),
    difference: mapObj(diff),
    sequential: scaleJSON(overlay.sequentialOmicsColors(a)),
    sequentialDark: scaleJSON(overlay.sequentialOmicsColors(a, { dark: true })),
    diverging: scaleJSON(overlay.divergingOmicsColors(fc)),
    divergingDark: scaleJSON(overlay.divergingOmicsColors(diff, { dark: true })),
    report: communityOmics.communityOmicsMappingReport(community, speciesConfig, models, dataset),
  });
}

// The JS unit test's hand-built community (only `merged.reactions` and
// `reactionOwner`), so the scoring rule is checked on its own as well.
const handModel = new Model({
  id: 'm', compartments: { c: 'cytosol' },
  metabolites: [{ id: 'a_c', compartment: 'c' }, { id: 'b_c', compartment: 'c' }],
  reactions: [
    { id: 'R1', metabolites: { a_c: -1, b_c: 1 }, gene_reaction_rule: 'g1 and g2' },
    { id: 'R2', metabolites: { b_c: -1, a_c: 1 }, gene_reaction_rule: 'g3 or g4' },
  ],
});
const handData = overlay.parseOmicsCSV('species,protein,C1\nsp,g1,10\nsp,g2,4\nsp,g3,2\nsp,g4,7\n', { dataType: 'protein' });
const handCommunity = {
  merged: { reactions: new Map([['sp@R1', {}], ['sp@R2', {}]]) },
  reactionOwner: new Map([['sp@R1', { speciesId: 'sp', originalId: 'R1' }], ['sp@R2', { speciesId: 'sp', originalId: 'R2' }]]),
};

await write(out, 'overlay.json', {
  parse: parseCases,
  deserialize: deserializeCases,
  gpr: { values: encodeScores(gprValues), cases: gprCases },
  compare: compareCases,
  scales: scaleCases,
  hand: { scores: mapObj(overlay.reactionOmicsScores(handCommunity, [{ id: 'sp', file: 'model' }], new Map([['model', handModel]]), handData, 'C1')) },
  scenarios: omicsScenarios,
});
console.log(`overlay: ${parseCases.length} tables, ${gprCases.length} rules, ${omicsScenarios.length} communities`);

// community-omics.js

const longCases = [
  ['long', 'MAG,gene,condition,TPM\nMAG1,g1,Control,10\nMAG1,g1,Treatment,40\nMAG1,g2,Control,5\nMAG1,g2,Treatment,10\n'],
  ['tsv', 'species\tlocus_tag\tcondition\tcounts\nsp\tg1\tA\t7\n'],
  ['wide-fallback', 'species,gene,C1,C2\nA,g1,1,2\nB,g1,,4\n'],
  ['wide-tsv-fallback', 'species\tgene\tC1\nA\tg1\t1\n'],
  ['blank-value-is-zero', 'genome,gene_id,sample,value\nG,g1,S1,\nG,g2,S1,x\nG,g3,S2, 4 \n,g4,S1,1\nG,,S1,1\nG,g5,,1\n'],
  ['numeric-conditions', 'bin,id,group,cpm\nb,g,2,1\nb,g,1,2\nb,g,x,3\nc,g,2,4\n'],
  ['no-species', 'gene,condition,value\ng1,A,1\n'],
  ['no-gene', 'species,condition,value\nA,A,1\n'],
  ['header-only', 'species,gene,condition,value\n'],
  ['nothing-usable', 'species,gene,condition,value\nA,g,C,x\n'],
  ['bom-crlf-quotes', '﻿Species,Gene,Condition,Expression\r\n"A, the first","g""1",C,1.5\r\n'],
  ['tabs-and-commas', 'species\tgene,x\tcondition\tvalue\nA\tg,1\tC\t2\n'],
];
const metatranscriptomics = longCases.map(([name, text]) => {
  const r = attempt(() => communityOmics.parseMetatranscriptomics(text, { source: `${name}.csv` }));
  return { name, text, ...(r.error ? { error: r.error } : { format: r.ok.format, dataset: datasetJSON(r.ok.dataset) }) };
});
// The JS unit tests' report cases.
const reportModel = new Model({ id: 'm', compartments: { c: 'c' }, genes: [{ id: 'g1' }, { id: 'g2' }],
  metabolites: [{ id: 'a_c', compartment: 'c' }, { id: 'b_c', compartment: 'c' }],
  reactions: [{ id: 'R', metabolites: { a_c: -1, b_c: 1 }, gene_reaction_rule: 'g1 and g2' }] });
const reportCommunity = { merged: { reactions: new Map([['MAG1@R', {}]]) }, reactionOwner: new Map([['MAG1@R', { speciesId: 'MAG1', originalId: 'R' }]]) };
const reportData = communityOmics.parseMetatranscriptomics('MAG,gene,condition,expression\nMAG1,g1,A,8\nMAG1,g2,A,3\n').dataset;
await write(out, 'community-omics.json', {
  parse: metatranscriptomics,
  report: communityOmics.communityOmicsMappingReport(reportCommunity, [{ id: 'MAG1', file: 'm' }], new Map([['m', reportModel]]), reportData),
});
console.log(`community-omics: ${metatranscriptomics.length} tables`);

// cross-feeding-explorer.js

const linkJSON = (raw) => ({ links: raw.links, perMetabolite: raw.perMetabolite });
const explorerCases = [];
const filters = [{}, { metabolite: 'ac' }, { producer: 'eco' }, { consumer: 'sce' }, { query: 'AC' }, { query: '  o ' },
  { minFlux: 0.5 }, { minFlux: NaN }, { minFlux: -1 }, { producer: 'eco', consumer: 'mba', minFlux: 0.01 }, { query: 'zzz' }];
for (const s of usable) {
  for (const objective of ['equal-growth', 'sum-biomass']) {
    const { community, spec } = await scenarioCommunity(s, objective);
    const result = new FluxProblem(community.merged).optimize();
    const fluxes = result.fluxes;
    const thresholds = [1e-6, 0.05];
    explorerCases.push({
      scenario: s.id, objective, spec,
      status: result.status,
      fluxes: mapObj(fluxes),
      raw: linkJSON(community.crossFeeding(fluxes, 1e-6)),
      indexes: thresholds.map((threshold) => {
        const index = explorer.buildCrossFeedingIndex(community, fluxes, { threshold });
        return {
          threshold, index,
          filters: filters.map((f) => ({ filter: { ...f, ...(f.minFlux === undefined ? {} : { minFlux: Number.isNaN(f.minFlux) ? 'NaN' : f.minFlux }) }, ids: explorer.filterCrossFeedingIndex(index, f).map((e) => e.id) })),
        };
      }),
      exchanged: explorer.exchangedMetabolites(community.crossFeeding(fluxes, 1e-6).links, (base) => base === 'h' || base === 'h2o'),
      exchangedAll: explorer.exchangedMetabolites(community.crossFeeding(fluxes, 1e-6).links),
    });
  }
}
// The JS unit test's community: transfers and a fixed cross-feeding.
const testLinks = [{ base: 'lac', from: 'A', to: 'B', flux: 3 }, { base: 'ac', from: 'A', to: 'B', flux: 1 }, { base: 'lac', from: null, to: 'B', flux: 2 },
  { base: 'but', from: 'B', to: 'B', flux: 9 }, { base: 'glc', from: 'C', to: '', flux: 4 }];
const testCommunity = {
  transfersByBase: new Map([['lac', [{ speciesId: 'A', reactionId: 'A@EX_lac' }, { speciesId: 'B', reactionId: 'B@EX_lac' }]]]),
  crossFeeding() { return { links: testLinks, perMetabolite: [] }; },
};
const testIndex = explorer.buildCrossFeedingIndex(testCommunity, new Map(), { threshold: 0.5 });
const exchangedLinks = [['nh4', 9], ['glc-D', 0.5], ['lac-D', 0.2], ['for', 3], ['ac', 0.1], ['ac', 0.3, 'C', 'B'], ['galur', 2], ['ala-L', 5],
  ['lac-L', 1], ['h', 50], ['but', 0.01, null, 'B'], ['Ac', 0.2], ['GLCUR', 1], ['12ppd-S', 2], ['xyz', 2], ['abc', 2], ['ppa', 0.01], ['LAC_D', 0.7]]
  .map(([base, flux, from = 'A', to = 'B']) => ({ base, from, to, flux }));
await write(out, 'explorer.json', {
  cases: explorerCases,
  test: { links: testLinks, index: testIndex },
  exchanged: { links: exchangedLinks, list: explorer.exchangedMetabolites(exchangedLinks, (base) => base === 'h') },
  groups: explorer.CROSS_FEEDING_GROUPS,
});
console.log(`explorer: ${explorerCases.length} solved communities`);

// gimme.js

const toyDoc = {
  id: 'gimme', compartments: { c: 'c' }, metabolites: [{ id: 'a_c', compartment: 'c' }],
  reactions: [
    { id: 'UP', metabolites: { a_c: 1 }, lower_bound: 0, upper_bound: 10 },
    { id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
    { id: 'WASTE', metabolites: { a_c: -1 }, lower_bound: 0, upper_bound: 1000 },
  ],
};
// Reversible reactions, so the split v = v+ - v- carries negative flux.
const reversibleDoc = {
  id: 'gimme-rev', compartments: { c: 'c', e: 'e' },
  metabolites: [{ id: 'a_e', compartment: 'e' }, { id: 'a_c', compartment: 'c' }, { id: 'b_c', compartment: 'c' }],
  reactions: [
    { id: 'EX_a_e', metabolites: { a_e: -1 }, lower_bound: -10, upper_bound: 1000 },
    { id: 'At', metabolites: { a_e: -1, a_c: 1 }, lower_bound: -1000, upper_bound: 1000 },
    { id: 'R1', metabolites: { a_c: -1, b_c: 1 }, lower_bound: -1000, upper_bound: 1000 },
    { id: 'R2', metabolites: { b_c: -1, a_c: 1 }, lower_bound: -5, upper_bound: 5 },
    { id: 'BIOMASS', metabolites: { b_c: -1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 1 },
  ],
};
// No feasible flux: BIOMASS must run but nothing makes a_c.
const infeasibleDoc = {
  id: 'gimme-dead', compartments: { c: 'c' }, metabolites: [{ id: 'a_c', compartment: 'c' }],
  reactions: [{ id: 'BIOMASS', metabolites: { a_c: -1 }, lower_bound: 1, upper_bound: 1000, objective_coefficient: 1 }],
};

function gimmeJSON(r) {
  const out = {};
  for (const [k, v] of Object.entries(r)) {
    if (v instanceof Map) out[k] = Object.fromEntries([...v].map(([id, x]) => [id, x]));
    else if (v instanceof Set) out[k] = [...v];
    else out[k] = v;
  }
  return JSON.parse(JSON.stringify(out));
}
const encodeOption = (v) => (v === undefined ? 'undefined' : (typeof v === 'number' && !Number.isFinite(v) ? String(v) : v));

const gimmeCases = [];
function gimmeCase(name, model, expression, options, extra = {}) {
  const problem = new FluxProblem(model instanceof Model ? model : new Model(model));
  const r = attempt(() => runGIMME(problem, new Map(Object.entries(expression)), options));
  gimmeCases.push({
    name, ...extra,
    expression: encodeScores(expression),
    options: Object.fromEntries(Object.entries(options).map(([k, v]) => [k, encodeOption(v)])),
    ...(r.error ? { error: r.error } : { result: gimmeJSON(r.ok) }),
  });
}
gimmeCase('toy-threshold', toyDoc, { UP: 10, BIOMASS: 10, WASTE: 0 }, { threshold: 5, objectiveFraction: 0.9 }, { model: toyDoc });
gimmeCase('toy-auto', toyDoc, { UP: 8, BIOMASS: 4, WASTE: 0 }, { objectiveFraction: 0.8 }, { model: toyDoc });
gimmeCase('toy-null-threshold', toyDoc, { UP: 8, BIOMASS: 4, WASTE: 0 }, { threshold: null }, { model: toyDoc });
gimmeCase('toy-nan-threshold', toyDoc, { UP: 8, BIOMASS: 4, WASTE: 0, OTHER: 1 }, { threshold: NaN, objectiveFraction: 1.5 }, { model: toyDoc });
gimmeCase('toy-negative-fraction', toyDoc, { UP: 8, WASTE: 0 }, { threshold: 6, objectiveFraction: -1 }, { model: toyDoc });
gimmeCase('toy-nan-fraction', toyDoc, { UP: 8, WASTE: 0 }, { threshold: 6, objectiveFraction: NaN }, { model: toyDoc });
gimmeCase('toy-infinite-threshold', toyDoc, { UP: 1, BIOMASS: 2, WASTE: 3 }, { threshold: Infinity }, { model: toyDoc });
gimmeCase('toy-no-expression', toyDoc, { UP: NaN, WASTE: Infinity }, {}, { model: toyDoc });
gimmeCase('reversible', reversibleDoc, { EX_a_e: 9, At: 9, R1: 1, R2: 8, BIOMASS: 9 }, { threshold: 5, objectiveFraction: 1 }, { model: reversibleDoc });
gimmeCase('reversible-low', reversibleDoc, { EX_a_e: 9, At: 9, R1: 1, R2: 0.5, BIOMASS: 9 }, { threshold: 5, objectiveFraction: 0.5 }, { model: reversibleDoc });
gimmeCase('infeasible', infeasibleDoc, { BIOMASS: 1 }, {}, { model: infeasibleDoc });
{
  // A core model with an expression score per reaction.
  const doc = await loadModel('ecoli_core_mc');
  const r = generator(7);
  const expression = Object.fromEntries(doc.reactions.filter((_, i) => i % 4 !== 3).map((x) => [x.id, Math.round(r() * 2000) / 100]));
  gimmeCase('ecoli-core', doc, expression, { objectiveFraction: 0.9 }, { modelFile: 'ecoli_core_mc' });
}
{
  // The Store's path: the community's merged problem, expression from the
  // omics scores of its first condition.
  const s = usable.find((x) => x.id === 'metanogenesis');
  const { community, models, speciesConfig, spec, gprs } = await scenarioCommunity(s);
  const csv = syntheticCSV(s.species, generator(11));
  const dataset = overlay.parseOmicsCSV(csv);
  const expression = mapObj(overlay.reactionOmicsScores(community, speciesConfig, models, dataset, 'C1'));
  gimmeCase('community', community.merged, expression, { threshold: null, objectiveFraction: 0.9 }, { spec, gprs });
}
await write(out, 'gimme.json', { cases: gimmeCases });
console.log(`gimme: ${gimmeCases.length} cases`);

// published.js

const catalog = JSON.parse(JSON.stringify(published.PUBLISHED_COMMUNITIES));
await write(out, 'published.json', {
  communities: catalog,
  frameworks: published.FRAMEWORKS,
  reproduced: published.reproducedCommunities().map((c) => c.id),
  blocked: published.blockedCommunities().map((c) => c.id),
  find: ['stolyar-2007-syntrophy', 'zuniga-2020-phea-rescue', 'nope', ''].map((id) => ({ id, found: published.findCommunity(id)?.id ?? null })),
});
if (process.argv.includes('--catalog')) {
  await writeFile(join(root, 'rust/crates/coxeter-analysis/src/published.json'), `${JSON.stringify(published.PUBLISHED_COMMUNITIES, null, 2)}\n`);
  console.log('published: rewrote the crate\'s copy of the registry');
}
console.log(`published: ${catalog.length} communities`);

// stolyar.js

function stolyarModelJSON(model) {
  return {
    metabolites: mapObj(model.metabolites),
    reactions: mapObj(model.reactions),
    macromolecules: mapObj(model.macromolecules),
    shared: [...model.shared],
    species: model.species,
  };
}
function stolyarOutputs(files) {
  const r = attempt(() => stolyar.importStolyar(files));
  if (r.error) return { error: r.error };
  const model = r.ok;
  const { bySpecies, community } = stolyar.reactionsBySpecies(model);
  return {
    model: stolyarModelJSON(model),
    validation: stolyar.validateStolyar(model),
    external: stolyar.externalMetabolites(model),
    bySpecies: mapObj(bySpecies), community,
    exchange: stolyar.exchangeStructure(model),
    objective: stolyar.objectiveOf(model),
    joint: stolyar.toJointCobraJSON(model),
    jointNamed: stolyar.toJointCobraJSON(model, { id: 'j', name: 'Joint' }),
    species: Object.fromEntries(stolyar.SPECIES.map((sp) => [sp.id, stolyar.toSpeciesCobraJSON(model, sp.id)])),
    speciesWithCell: stolyar.toSpeciesCobraJSON(model, 'Mm', { cell: { name: 'M', color: '#000' } }),
    unknownSpecies: attempt(() => stolyar.toSpeciesCobraJSON(model, 'Xx')).error,
  };
}

const TINY = {
  reactions: [
    'Dsrc      \t = 1 S \t| \t# \t0  10    0 \t0 0 1 1 \t0.01',
    'Dmake     \t1 S = 1 A + 1 SHARED \t| \t# \t0  100    0 \t0 0 1 1 \t0.01',
    'mue       \t \t \t| \t# \t0  100   -1 \t0 0 1 1 \t0.01',
    'Mtake     \t1 SHARED = 1 ZM_A \t| \t# \t0  100    0 \t0 0 3 1 \t0.01',
    'MC_BIO    \t1 ZM_A = 1 ZM_BIOMASS \t| \t# \t0  100   -0.1 \t0 0 3 1 \t0.01',
    'Sink      \t1 SHARED =  \t| \t0 \t0  100    0 \t0 0 3 1 \t0.01',
  ].join('\n'),
  metabolites: [
    'S \tsubstrate \t0.001 \t0',
    'A \tproduct \t0.001 \t0',
    'SHARED \tshared \t0.001 \t0',
    'ZM_A \tmethanogen product \t0.001 \t0',
    'ZM_BIOMASS \tmethanogen biomass \t0.001 \t1',
  ].join('\n'),
  macromolecules: 'BIO \tBiomass \t1 \t0 0 2 1',
  synthesis: 'BIO    = 2 A',
};
const variants = {
  tiny: TINY,
  'crlf-and-blank-lines': { ...TINY, reactions: `\r\n${TINY.reactions.replace(/\n/g, '\r\n\r\n')}\r\n`, metabolites: `${TINY.metabolites}\n\n` },
  'layer-prefix': { ...TINY, reactions: TINY.reactions.replace('MC_BIO    \t1 ZM_A = 1 ZM_BIOMASS \t| \t# \t0  100   -0.1 \t0 0 3 1 \t0.01', 'MC_BIO    \t1 ZM_A = 1 ZM_BIOMASS \t| \t# \t0  100   -0.1 \t0 0 1 1 \t0.01') },
  undeclared: { ...TINY, reactions: `${TINY.reactions}\nBad \t1 S = 1 GHOST \t| \t# \t0 100 0 \t0 0 1 1 \t0.01` },
  'no-bar': { ...TINY, reactions: `${TINY.reactions}\nNoBar\t1 S = 1 A\t#\t0 1 0\t0 0 1 1` },
  'bar-first': { ...TINY, reactions: `|\tX\t#\t0 1 0\t0 0 1 1` },
  'long-line-no-bar': { ...TINY, reactions: `${'Ü'.repeat(30)}\t"quoted"\t\u{1F600}${'x'.repeat(20)}` },
  'unreadable-bounds': { ...TINY, reactions: `${TINY.reactions}\nBadB \t1 S = 1 A \t| \t# \tzero 1 0 \t0 0 1 1 \t0.01` },
  'one-bound': { ...TINY, reactions: `${TINY.reactions}\nOneB \t1 S = 1 A \t| \t# \t5 \t0 0 1 1 \t0.01` },
  'no-mue': { ...TINY, reactions: TINY.reactions.replace(/mue {7}/, 'growth ') },
  'mue-with-equation': { ...TINY, reactions: TINY.reactions.replace('mue       \t \t \t|', 'mue       \t1 A = \t|') },
  'macro-without-synthesis': { ...TINY, synthesis: 'OTHER = 2 A' },
  'bad-synthesis-term': { ...TINY, synthesis: 'BIO = 2 A + B' },
  'unknown-layer': { ...TINY, reactions: TINY.reactions.replace('\t0 0 3 1 \t0.01', '\t0 0 7 1 \t0.01') },
  // S and A become shared once a methanogen reaction touches them, so this one is the pair's.
  'methanogen-touches-dv': { ...TINY, reactions: `${TINY.reactions}\nMx \t1 S = 1 A \t| \t# \t0 1 0 \t0 0 3 1 \t0.01` },
  'methanogen-without-prefix': { ...TINY, reactions: `${TINY.reactions}\nMq \t1 Q = \t| \t# \t0 1 0 \t0 0 3 1 \t0.01`, metabolites: `${TINY.metabolites}\nQ \tq \t0 \t0` },
  'measured-and-plus-ids': {
    ...TINY,
    reactions: `${TINY.reactions}\nHplus \t6 H+ + 1 S = 0.5 A + .25 H+ \t| \t2.5 \t-10 10 0.5 \t1 2 1 1 \t0.01\nNum \t1 S = 1 2 \t| \tabc \t0 1 \t0 0 1 \t0.01\nNoObj \t1 S = 1 A \t| \t \t0 1 \t5 7 1 \t0.01`,
    metabolites: `${TINY.metabolites}\nH+ \t \t0.001 \t0\n2 \tnumeric \t0 \t1`,
  },
  'missing-layer': { ...TINY, reactions: `${TINY.reactions}\nNoLayer \t1 S = 1 A \t| \t# \t0 1 \t3 \t0.01` },
  'missing-columns': { ...TINY, reactions: `${TINY.reactions}\nShort \t1 S = 1 A \t| \t#` },
  'missing-position': { ...TINY, reactions: `${TINY.reactions}\nShort \t1 S = 1 A \t| \t# \t0 1 0` },
  'synthesis-signs': { ...TINY, synthesis: 'BIO    = 2 A + -3.5 S + 0.25 A\n = 1 A\nOTHER = 1 A' },
  'bad-fractions': { ...TINY, macromolecules: 'BIO \tBiomass \t0.5 \t0 0 2 1\nBIO2 \t\tx', synthesis: 'BIO = 1 A\nBIO2 = 1 S' },
};
const stolyarVariants = Object.fromEntries(Object.entries(variants).map(([name, files]) => [name, { files, ...stolyarOutputs(files) }]));

const termCases = ['6 H+ + 1 H2S', '1 ATP + 1 SO4 + 1 H+', '1 Fdx(red)', '', '0.5 ATP', '.5 X', '5. X', '2  A  +  B', 'A+B', ' 1 A + ', '1e5 X', '-1 X', '1 2 3'];
const equationCases = ['1 GAP + 1 ADP = 1 3PG + 1 ATP + 1 NADH', '1 H2(exch) = ', ' = 1 ZM_Ala', '', '0.5 ATP = 0.5 ADP + 1 Lac',
  '1 A = 1 A', '1 A + 1 B = 1 A', '1 A = 2 B = 3 C', '2 10 + 1 x = 1 2', 'A + A = B'];
const synthesisCases = ['RNA = 6.17 Asp + -3.47 fum + 32.41 ATP\n', 'X = \n', 'Y = 1 a + 2 a', '= 1 a', 'Z = 1 H+ + 2 b', 'W = abc'];

// FBA on the miniature, and on the bundled joint model the JS app ships.
function fbaRecord(problem) {
  const r = problem.optimize();
  return { status: r.status, objectiveValue: r.objectiveValue, fluxes: mapObj(r.fluxes) };
}
const tinyJoint = new Model(stolyar.toJointCobraJSON(stolyar.importStolyar(TINY)));
const cut = new FluxProblem(tinyJoint);
cut.setBounds('Mtake', 0, 0);
const bundledJoint = JSON.parse(await readFile(join(root, 'models/published/stolyar-2007-syntrophy/joint-model.json'), 'utf8'));
const jointModel = new Model(bundledJoint);
const syntrophic = new FluxProblem(jointModel);
syntrophic.setBounds('EX_SO4', 0, 1000);
const respiring = new FluxProblem(jointModel);
respiring.setBounds('EX_SO4', -1000, 1000);
const syntrophicResult = fbaRecord(syntrophic);
const ranges = syntrophic.variability(['mue', 'MC_BIO', 'M12', 'D81', 'D80'], { fraction: 1 });
await write(out, 'stolyar.json', {
  species: stolyar.SPECIES,
  expected: stolyar.EXPECTED,
  terms: termCases.map((side) => ({ side, terms: stolyar.parseTerms(side) })),
  equations: equationCases.map((equation) => ({ equation, stoichiometry: stolyar.parseEquation(equation) })),
  synthesis: synthesisCases.map((text) => {
    const r = attempt(() => mapObj(stolyar.parseMacromoleculeSynthesis(text)));
    return { text, ...(r.error ? { error: r.error } : { ok: r.ok }) };
  }),
  variants: stolyarVariants,
  fba: {
    tiny: fbaRecord(new FluxProblem(tinyJoint)),
    cut: fbaRecord(cut),
    bundled: {
      syntrophic: { status: syntrophicResult.status, objectiveValue: syntrophicResult.objectiveValue, fluxes: Object.fromEntries(['mue', 'MC_BIO', 'M12', 'D01'].map((id) => [id, syntrophicResult.fluxes[id]])) },
      respiring: (() => { const x = fbaRecord(respiring); return { status: x.status, objectiveValue: x.objectiveValue, fluxes: Object.fromEntries(['mue', 'MC_BIO', 'M12', 'D01'].map((id) => [id, x.fluxes[id]])) }; })(),
      ranges: Object.fromEntries([...ranges].map(([id, r]) => [id, [r.min, r.max]])),
    },
  },
});
console.log(`stolyar: ${Object.keys(stolyarVariants).length} variants`);

// The four supplement files, rebuilt from the bundled joint model and bounds
// table (git-ignored: the supplement is the paper's). Not byte-identical to the
// originals (spacing and map positions are not recorded), but the same
// network, so the reader meets 180 real reactions and 170 real identifiers.
{
  const dir = join(root, 'models/published/stolyar-2007-syntrophy');
  const csvRows = (text) => text.trim().split(/\r?\n/).slice(1).map((line) => line.split(','));
  const bounds = csvRows(await readFile(join(dir, 'bounds.csv'), 'utf8'))
    .map(([id, species, lb, ub, c, measured]) => ({ id, species, lb, ub, c, measured }));
  const external = new Set(csvRows(await readFile(join(dir, 'media.csv'), 'utf8')).filter((r) => r[2] === 'external').map((r) => r[0]));
  const byId = new Map(bundledJoint.reactions.map((r) => [r.id, r]));
  const names = new Map(bundledJoint.metabolites.map((m) => [m.id, m.name]));
  const plain = (x) => (String(x).includes('e') ? x.toFixed(20).replace(/0+$/, '').replace(/\.$/, '') : String(x));
  const side = (entries) => entries.map(([m, c]) => `${plain(c)} ${m}`).join(' + ');
  const mue = byId.get('mue');
  const macros = Object.entries(mue.metabolites);
  const files = {
    reactions: bounds.filter((b) => !b.id.endsWith('_syn')).map((b) => {
      const met = Object.entries(byId.get(b.id).metabolites);
      const equation = b.id === 'mue' ? '' : `${side(met.filter(([, c]) => c < 0).map(([m, c]) => [m, -c]))} = ${side(met.filter(([, c]) => c > 0))}`;
      const layer = b.species === 'Mm' ? (b.id.startsWith('MC_') ? 2 : 3) : 1;
      return `${b.id}\t${equation}\t|\t${b.measured === '' ? '#' : b.measured}\t${b.lb} ${b.ub} ${b.c}\t0 0 ${layer} 1\t0.01`;
    }).join('\n'),
    metabolites: bundledJoint.metabolites.filter((m) => !(m.id in mue.metabolites))
      .map((m) => `${m.id}\t${m.name}\t0.001\t${external.has(m.id) ? 1 : 0}`).join('\n'),
    macromolecules: macros.map(([m, c]) => `${m}\t${names.get(m)}\t${-c}\t0 0 2 1`).join('\n'),
    synthesis: macros.map(([m]) => `${m} = ${Object.entries(byId.get(`${m}_syn`).metabolites).filter(([x]) => x !== m)
      .map(([x, c]) => `${plain(-c)} ${x}`).join(' + ')}`).join('\n'),
  };
  for (const [name, file] of [['reactions', 'reactions.txt'], ['metabolites', 'metabolites.txt'], ['macromolecules', 'macromolecules.txt'], ['synthesis', 'macromolecule_synthesis.txt']]) {
    await writeFile(join(large, 'stolyar-reconstructed', file), files[name]);
  }
  const outputs = stolyarOutputs(files);
  const same = !outputs.error && JSON.stringify(outputs.joint) === JSON.stringify(bundledJoint);
  await write(join(large, 'stolyar-reconstructed'), 'outputs.json', { reproducesBundledJoint: same, ...outputs });
  console.log(`stolyar (large): rebuilt supplement ${same ? 'reproduces' : 'does NOT reproduce'} the bundled joint model`);
}

// steadycom-ecoli.js

{
  // A miniature iAF1260: every reaction the builder touches, written as SBML
  // by the app's own writer and read back by the builder.
  const aas = ['arg-L', 'lys-L', 'met-L', 'phe-L'];
  const synth = { 'arg-L': 'ARGSL', 'lys-L': 'DAPDC', 'met-L': 'HSST', 'phe-L': 'PPNDH' };
  const exporters = { 'arg-L': 'ARGt3pp', 'lys-L': 'LYSt3pp', 'phe-L': 'PHEt2rpp' };
  const metabolites = [
    { id: 'glc__D_e', compartment: 'e' }, { id: 'glc__D_c', compartment: 'c' }, { id: 'o2_e', compartment: 'e' },
    { id: 'h_c', compartment: 'c' }, { id: 'h_p', compartment: 'p' },
    ...aas.flatMap((aa) => ['c', 'p', 'e'].map((c) => ({ id: `${aa}_${c}`, compartment: c }))),
  ];
  const reactions = [
    { id: 'EX_glc_e', metabolites: { glc__D_e: -1 }, lower_bound: -8, upper_bound: 999999 },
    { id: 'EX_o2_e', metabolites: { o2_e: -1 }, lower_bound: -18.5, upper_bound: 999999 },
    { id: 'EX_h2o_e_unused', metabolites: { o2_e: -1 }, lower_bound: 0, upper_bound: 5 },
    { id: 'GLCt', metabolites: { glc__D_e: -1, glc__D_c: 1 }, lower_bound: 0, upper_bound: 999999 },
    { id: 'Ht', metabolites: { h_p: -1, h_c: 1 }, lower_bound: -1000, upper_bound: 1000 },
    ...aas.map((aa) => ({ id: synth[aa], metabolites: { glc__D_c: -1, [`${aa}_c`]: 1 }, lower_bound: 0, upper_bound: 1000 })),
    ...Object.entries(exporters).map(([aa, id]) => ({ id, metabolites: { [`${aa}_c`]: -1, [`${aa}_p`]: 1 }, lower_bound: 0, upper_bound: 1000 })),
    ...aas.map((aa) => ({ id: `${aa.split('-')[0].toUpperCase()}tex`, metabolites: { [`${aa}_p`]: -1, [`${aa}_e`]: 1 }, lower_bound: -1000, upper_bound: 1000 })),
    ...aas.map((aa) => ({ id: `${aa.split('-')[0].toUpperCase()}abc`, metabolites: { [`${aa}_p`]: -1, [`${aa}_c`]: 1 }, lower_bound: 0, upper_bound: 1000 })),
    ...aas.map((aa) => ({ id: `EX_${aa.replace('-', '_')}_e`, metabolites: { [`${aa}_e`]: -1 }, lower_bound: 0, upper_bound: 1000 })),
    {
      id: 'BIOMASS_Ec_iAF1260_core_59p81M', objective_coefficient: 1, lower_bound: 0, upper_bound: 999999,
      metabolites: { glc__D_c: -1, 'arg-L_c': -0.3, 'lys-L_c': -0.3, 'met-L_c': -0.15, 'phe-L_c': -0.2 },
    },
  ];
  const doc = { id: 'iAF1260_mini', name: 'iAF1260 miniature', compartments: { c: 'cytosol', p: 'periplasm', e: 'extracellular' }, metabolites, reactions };
  const xml = toSBML(new Model(doc));
  await writeFile(join(out, 'steadycom-ecoli-mini.xml'), xml);
  const spec = ecoli.buildCommunity(xml);
  const result = steadyCom(spec, { upperGuess: 5 });
  const broken = xml.replace(/R_PHEt2rpp/g, 'R_PHEt9');
  const noExchange = xml.replace(/R_EX_met_L_e/g, 'R_EX_met_X_e');
  await write(out, 'steadycom-ecoli.json', {
    provenance: ecoli.PROVENANCE,
    addedReaction: ecoli.ADDED_REACTION,
    strains: ecoli.STRAINS,
    constants: { EX_RATE: ecoli.EX_RATE, EXCHANGE_UPPER_BOUND: ecoli.EXCHANGE_UPPER_BOUND, EXPECTED_GROWTH_RATE: ecoli.EXPECTED_GROWTH_RATE, SENTINEL_BOUND: ecoli.SENTINEL_BOUND },
    aminoAcidExchanges: ecoli.AMINO_ACID_EXCHANGES,
    exchangeBase: ['EX_glc_e', 'EX_glc', 'glc_e', 'EX_EX_a_e_e', 'EX__e'].map((id) => [id, ecoli.exchangeBase(id)]),
    community: {
      members: spec.members.map((m) => ({ id: m.id, strain: m.strain, model: modelJSON(m.model), biomassReaction: m.biomassReaction, exchangeFor: m.exchangeFor })),
      communityUptake: spec.communityUptake,
      totalBiomass: spec.totalBiomass,
      document: spec.document,
    },
    steadyCom: { status: result.status, growthRate: result.growthRate, abundances: mapObj(result.abundances) },
    errors: {
      missingReaction: attempt(() => ecoli.buildCommunity(broken)).error,
      missingExchange: attempt(() => ecoli.buildCommunity(noExchange)).error,
    },
  });
  await writeFile(join(out, 'steadycom-ecoli-broken.xml'), broken);
  await writeFile(join(out, 'steadycom-ecoli-noexchange.xml'), noExchange);
  console.log(`steadycom-ecoli: mu = ${result.growthRate}`);
}

// gut9.js

/** An .xlsx built here: shared and inline strings, one deflated entry. */
function workbook(sheets) {
  const shared = [];
  const sharedIndex = (s) => {
    let i = shared.indexOf(s);
    if (i < 0) { i = shared.length; shared.push(s); }
    return i;
  };
  const escape = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  const column = (i) => String.fromCharCode(65 + i);
  const sheetXml = (rows) => rows.map((cells, r) => {
    const rowNumber = r + 1;
    const xml = cells.map((v, c) => {
      if (v === undefined || v === null || v === '') return '';
      const ref = `${column(c)}${rowNumber}`;
      if (typeof v === 'number') return `<c r="${ref}"><v>${v}</v></c>`;
      if (r % 2 === 0) return `<c r="${ref}" t="s"><v>${sharedIndex(v)}</v></c>`;
      return `<c r="${ref}" t="inlineStr"><is><t>${escape(v)}</t></is></c>`;
    }).join('');
    return `<row r="${rowNumber}">${xml}</row>`;
  }).join('');
  const names = Object.keys(sheets);
  const bodies = names.map((name) => `<worksheet><sheetData>${sheetXml(sheets[name])}</sheetData></worksheet>`);
  const files = [
    ['xl/workbook.xml', `<workbook><sheets>${names.map((n, i) => `<sheet name="${n}" sheetId="${i + 1}" r:id="rId${i + 1}"/>`).join('')}</sheets></workbook>`],
    ['xl/_rels/workbook.xml.rels', `<Relationships>${names.map((_, i) => `<Relationship Id="rId${i + 1}" Type="x/worksheet" Target="worksheets/sheet${i + 1}.xml"/>`).join('')}</Relationships>`],
    ['xl/sharedStrings.xml', `<sst>${shared.map((s) => `<si><t>${escape(s)}</t></si>`).join('')}</sst>`],
    ...bodies.map((b, i) => [`xl/worksheets/sheet${i + 1}.xml`, b]),
  ];
  const locals = [];
  const central = [];
  let offset = 0;
  files.forEach(([name, text], index) => {
    const nameBuf = Buffer.from(name, 'utf8');
    const raw = Buffer.from(text, 'utf8');
    const deflate = index === files.length - 1;
    const data = deflate ? deflateRawSync(raw) : raw;
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(deflate ? 8 : 0, 8);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(raw.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    locals.push(local, nameBuf, data);
    const dir = Buffer.alloc(46);
    dir.writeUInt32LE(0x02014b50, 0);
    dir.writeUInt16LE(deflate ? 8 : 0, 10);
    dir.writeUInt32LE(data.length, 20);
    dir.writeUInt32LE(raw.length, 24);
    dir.writeUInt16LE(nameBuf.length, 28);
    dir.writeUInt32LE(offset, 42);
    central.push(dir, nameBuf);
    offset += local.length + nameBuf.length + data.length;
  });
  const body = Buffer.concat(locals);
  const directory = Buffer.concat(central);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(files.length, 8);
  eocd.writeUInt16LE(files.length, 10);
  eocd.writeUInt32LE(directory.length, 12);
  eocd.writeUInt32LE(body.length, 16);
  return Buffer.concat([body, directory, eocd]);
}

{
  // Two species: A ferments glucose to acetate, B lives on the acetate.
  const rxnRows = [
    ['UT_glc(u)', 'glucose uptake', 'Exchange', 0, 0, 0, 0, 'comm', { 'glc[u]': 1 }],
    ['EX_glc(u)', 'glucose export', 'Exchange', 0, 0, 10000, 0, 'comm', { 'glc[u]': -1 }],
    ['UT_ac(u)', null, 'Exchange', 0, 0, 0, 0, 'comm', { 'ac[u]': 1 }],
    ['EX_ac(u)', 'acetate export', null, 0, 0, 10000, 0, 'comm', { 'ac[u]': -1 }],
    ['EX_glc[u]tr_A', 'glc A<->u', 'Transport', 1, -1000, 1000, 0, 'A', { 'glc[e_A]': -1, 'glc[u]': 1 }],
    ['GLCt_A', 'glc transport', 'Transport', 0, 0, 1000, 0, 'A', { 'glc[e_A]': -1, 'glc[c_A]': 1 }],
    ['GLY_A', 'glycolysis', 'Central', 0, 0, 1000, 0, 'A', { 'glc[c_A]': -1, 'ac[c_A]': 2, 'atp[c_A]': 2 }],
    ['ACt_A', 'ac export', 'Transport', 0, 0, 1000, 0, 'A', { 'ac[c_A]': -1, 'ac[e_A]': 1 }],
    ['EX_ac[u]tr_A', 'ac A<->u', 'Transport', 1, -1000, 1000, 0, 'A', { 'ac[e_A]': -1, 'ac[u]': 1 }],
    ['BIOMASS_A', 'biomass A', 'Biomass', 0, 0, 1000, 1, 'A', { 'atp[c_A]': -10 }],
    ['ATPM_A', 'maintenance A', 'Energy', 0, 1, 1000, 0, 'A', { 'atp[c_A]': -1 }],
    ['IEX_ac[u]tr_B', 'ac B<->u', 'Transport', 1, -1000, 1000, 0, 'B', { 'ac[e_B]': -1, 'ac[u]': 1 }],
    ['ACt_B', 'ac uptake', 'Transport', 0, 0, 1000, 0, 'B', { 'ac[e_B]': -1, 'ac[c_B]': 1 }],
    ['ACK_B', 'acetate kinase', 'Central', 0, 0, 1000, 0, 'B', { 'ac[c_B]': -1, 'atp[c_B]': 0.5 }],
    ['BIOMASS_B', 'biomass B', 'Biomass', 0, 0, 1000, 1, 'B', { 'atp[c_B]': -10 }],
    ['ATPM_B', 'maintenance B', 'Energy', 0, 1, 1000, 0, 'B', { 'atp[c_B]': -1 }],
  ];
  const mets = [
    ['glc[u]', 'glucose', 'C6H12O6', 0, 'comm'], ['ac[u]', 'acetate', 'C2H3O2', -1, 'comm'],
    ['glc[e_A]', 'glucose', 'C6H12O6', 0, 'A'], ['glc[c_A]', 'glucose', 'C6H12O6', 0, 'A'], ['ac[c_A]', 'acetate', null, -1, 'A'],
    ['ac[e_A]', 'acetate', 'C2H3O2', null, 'A'], ['atp[c_A]', null, 'C10H12N5O13P3', -4, 'A'],
    ['ac[e_B]', 'acetate', 'C2H3O2', -1, 'B'], ['ac[c_B]', 'acetate', 'C2H3O2', -1, 'B'], ['atp[c_B]', 'ATP', '', -4, 'B'],
  ];
  const pad = (row, n) => [...row, ...Array(Math.max(0, n - row.length)).fill(null)];
  const sheets = {
    comSp: [['abbr', 'name', 'biomass', 'atpm', '#met', '#rxn', '#ex'], ['A', 'Species A', 'BIOMASS_A', 'ATPM_A', 5, 7, 2], ['B', 'Species B', 'BIOMASS_B', 'ATPM_B', 3, 5, 1], [null, 'trailing']],
    mets: [pad(['mets', 'metNames', 'metFormulas', 'metCharge'], 10).map((v, i) => (i === 9 ? 'species' : v)),
      ...mets.map(([id, name, formula, charge, sp]) => [id, name, formula, charge, null, null, null, null, null, sp])],
    rxns: [pad(['rxns', 'rxnNames', 'subSystems'], 14).map((v, i) => (i >= 9 ? ['rev', 'lb', 'ub', 'c', 'species'][i - 9] : v)),
      ...rxnRows.map(([id, name, sub, rev, lb, ub, c, sp]) => [id, name, sub, null, null, null, null, null, null, rev, lb, ub, c, sp]),
      [null, 'no identifier'], [null, null, 'x']],
    stoich: [['met', 'rxn', 'coef'], ...rxnRows.flatMap(([id, , , , , , , , st]) => Object.entries(st).map(([m, c]) => [m, id, c])), ['glc[u]', 'UT_glc(u)', 0.5]],
    comEx: [['met', 'UT', 'EX', 'EXsp'], [null, null, null, 'A', 'B'],
      ['glc[u]', 'UT_glc(u)', 'EX_glc(u)', 'EX_glc[u]tr_A'], ['ac[u]', 'UT_ac(u)', 'EX_ac(u)', 'EX_ac[u]tr_A', 'IEX_ac[u]tr_B'], [null, 'x']],
  };
  const bytes = workbook(sheets);
  await writeFile(join(out, 'gut9-mini.xlsx'), bytes);
  const e12 = (x) => x.toExponential(12).replace(/e([+-])(\d)$/, 'e$10$2');
  const boundsText = ['rxns\trev\tlb\tub\tc', ...rxnRows.filter(([id]) => id !== 'ATPM_B').map(([id, , , rev, lb, ub, c]) => {
    const upper = id === 'UT_glc(u)' ? 10 : ub;
    return `${id}\t${rev}\t${e12(lb)}\t${e12(upper)}\t${c}`;
  }), 'GHOST\t0\t0\t1\t0', ''].join('\n');
  const stoichText = ['mets\trxns\tcoefs', ...rxnRows.flatMap(([id, , , , , , , , st]) => Object.entries(st).map(([m, c]) => `${m}\t${id}\t${c.toFixed(12)}`)), ''].join('\r\n');
  const badStoich = stoichText.replace('atp[c_B]\tBIOMASS_B\t-10.000000000000', 'atp[c_B]\tBIOMASS_B\t-9.000000000000')
    .replace('ac[c_B]\tACK_B\t-1.000000000000\r\n', '') + 'xx[u]\tACK_B\t1\r\nglc[u]\tNOPE\t1\r\n';
  const plain = gut9.importGut9(bytes);
  const withBounds = gut9.importGut9(bytes, { boundsText });
  const gut9JSON = (m) => ({ ...m, metabolites: mapObj(m.metabolites), reactions: mapObj(m.reactions), communityUptake: mapObj(m.communityUptake),
    communityExport: mapObj(m.communityExport), speciesExchange: mapObj(m.speciesExchange) });
  const spec = gut9.toSteadyComSpec(withBounds);
  const result = steadyCom(spec, { upperGuess: 1 });
  const at = maxTotalBiomassAt(spec, result.growthRate * 0.5);
  const badSheets = (patch) => workbook({ ...sheets, ...patch });
  const errorBooks = {
    'unknown-reaction': badSheets({ stoich: [['met', 'rxn', 'coef'], ['glc[u]', 'NOPE', 1]] }),
    'no-reaction-cell': badSheets({ stoich: [['met', 'rxn', 'coef'], ['glc[u]', null, 1]] }),
    'unknown-metabolite': badSheets({ stoich: [['met', 'rxn', 'coef'], ['zzz', 'UT_glc(u)', 1]] }),
    'missing-sheet': workbook({ comSp: sheets.comSp }),
  };
  const errors = {};
  for (const [name, book] of Object.entries(errorBooks)) {
    await writeFile(join(out, `gut9-${name}.xlsx`), book);
    errors[name] = attempt(() => gut9.importGut9(book)).error;
  }
  await write(out, 'gut9.json', {
    provenance: gut9.PROVENANCE,
    expected: gut9.EXPECTED,
    boundsText, stoichText, badStoich,
    plain: gut9JSON(plain),
    withBounds: gut9JSON(withBounds),
    applied: gut9.applyBounds(gut9.importGut9(bytes), boundsText),
    validateBounds: { plain: gut9.validateBounds(plain, boundsText), withBounds: gut9.validateBounds(withBounds, boundsText) },
    validateStoichiometry: { good: gut9.validateStoichiometry(plain, stoichText), bad: gut9.validateStoichiometry(plain, badStoich) },
    reactionsBySpecies: mapObj(gut9.reactionsBySpecies(withBounds)),
    communityReactionIds: gut9.communityReactionIds(withBounds),
    diet: { plain: mapObj(gut9.diet(plain)), withBounds: mapObj(gut9.diet(withBounds)) },
    parseBounds: [
      { text: 'rxns\trev\tlb\tub\tc\nR1\t1\t-1.0e+00\t2.5e+00\t0\nR2\t0\t0\t1e+03\t1\n' },
      { text: 'rxns\r\nR3\t \t1\r\n\r\n  \nR3\t0\t2\t3\t4\nR4' },
      { text: 'something\telse\n' }, { text: '' },
    ].map(({ text }) => {
      const r = attempt(() => mapObj(gut9.parseBounds(text)));
      return { text, ...(r.error ? { error: r.error } : { ok: r.ok }) };
    }),
    headerErrors: {
      stoich: attempt(() => gut9.validateStoichiometry(plain, 'rxns\tx\n')).error,
      bounds: attempt(() => gut9.validateBounds(plain, 'mets\n')).error,
    },
    unknownSpecies: attempt(() => gut9.reactionsBySpecies({ ...plain, species: plain.species.slice(0, 1) })).error,
    steadyCom: { status: result.status, growthRate: result.growthRate, at: { mu: result.growthRate * 0.5, feasible: at.feasible, total: at.total, status: at.status } },
    errors,
  });
  console.log(`gut9: mu = ${result.growthRate}`);
}

// zuniga.js

// MAT Level 5 writer (as in fixtures-io.mjs): uncompressed elements only.
function concat(...items) {
  const result = new Uint8Array(items.reduce((size, item) => size + item.length, 0));
  let offset = 0;
  for (const item of items) { result.set(item, offset); offset += item.length; }
  return result;
}
function numeric(values, type = 5) {
  const widths = { 5: 4, 6: 4, 9: 8 };
  const methods = { 5: 'setInt32', 6: 'setUint32', 9: 'setFloat64' };
  const bytes = new Uint8Array(values.length * widths[type]);
  const view = new DataView(bytes.buffer);
  values.forEach((value, i) => view[methods[type]](i * widths[type], value, true));
  return bytes;
}
function tag(type, data, small = false) {
  const bytes = new Uint8Array(small ? 8 : 8 + data.length + (8 - data.length % 8) % 8);
  const view = new DataView(bytes.buffer);
  if (small) { view.setUint16(0, type, true); view.setUint16(2, data.length, true); bytes.set(data, 4); } else { view.setUint32(0, type, true); view.setUint32(4, data.length, true); bytes.set(data, 8); }
  return bytes;
}
function matrix({ name = '', cls = 6, shape, type = 9, values = [], raw, extra = new Uint8Array(), nzmax = 0 }) {
  return tag(14, concat(tag(6, numeric([cls, nzmax], 6)), tag(5, numeric(shape, 5)), tag(1, new TextEncoder().encode(name), name.length > 0 && name.length <= 4),
    tag(type, raw ?? numeric(values, type)), extra));
}
const doubles = (values, column = true) => matrix({ shape: column ? [values.length, 1] : [1, values.length], values });
const chars = (text) => matrix({ cls: 4, shape: [text.length ? 1 : 0, [...text].length], type: 16, raw: new TextEncoder().encode(text) });
const charColumn = (text) => matrix({ cls: 4, shape: [[...text].length, 1], type: 16, raw: new TextEncoder().encode(text) });
const cell = (items, name = '', shape = [items.length, 1]) => tag(14, concat(tag(6, numeric([1, 0], 6)), tag(5, numeric(shape, 5)), tag(1, new TextEncoder().encode(name)), ...items));
function sparseMatrix(ir, jc, values, shape) {
  return matrix({ cls: 5, shape, type: 5, raw: numeric(ir), extra: concat(tag(5, numeric(jc)), tag(9, numeric(values, 9))), nzmax: values.length });
}
function struct(fields, name = '') {
  const width = 32;
  const keys = Object.keys(fields);
  const namesBytes = new Uint8Array(width * keys.length);
  keys.forEach((f, i) => namesBytes.set(new TextEncoder().encode(f), i * width));
  return tag(14, concat(tag(6, numeric([2, 0], 6)), tag(5, numeric([1, 1], 5)), tag(1, new TextEncoder().encode(name)),
    tag(5, numeric([width], 5), true), tag(1, namesBytes), ...keys.map((k) => fields[k])));
}
const matFile = (data) => {
  const header = new Uint8Array(128);
  header.set(new TextEncoder().encode('MATLAB 5.0 MAT-file, Coxeter synthetic Zuniga fixture'));
  new DataView(header.buffer).setUint16(124, 0x100, true);
  header[126] = 73; header[127] = 77;
  return concat(header, data);
};

const EC_BIOMASS = 'BIOMASS_Ec_iJO1366_core_53p95M';
/** The JS unit test's synthetic model, as the reader would see it in a MAT file. */
function rawModel(coculture) {
  const suffix = coculture ? '_D' : '';
  const food = `food_c${suffix}`, pre = `pre_c${suffix}`, phe = `phe__L_c${suffix}`;
  const mets = [food, pre, phe];
  const r = (id, metabolites, lb = 0, ub = 1000, gpr = '', objective = 0) => ({ id: id + suffix, metabolites, lb, ub, gpr, objective });
  const reactions = [
    r('SOURCE', { [food]: 1 }, 0, 3),
    r('CHORM', { [food]: -1, [pre]: 1 }, 0, 1000, '(b2599 or isozyme)'),
    r('PPNDH', { [pre]: -1, [phe]: 1 }, 0, 1000, 'b2599'),
    r(EC_BIOMASS, { [phe]: -1 }, 0, 1000, '', 1),
    r('BROKEN_TEXT', { [food]: -1 }, 0, 0, '((other)'),
  ];
  if (coculture) {
    mets.push('phe__L_c_A', 'phe__L_[smp]');
    reactions.push(
      { id: 'SOURCE_A', metabolites: { phe__L_c_A: 1 }, lb: 0, ub: 4, gpr: '', objective: 0 },
      { id: 'BOF_A', metabolites: { phe__L_c_A: -1 }, lb: 1, ub: 1, gpr: '', objective: 1 },
      { id: 'PHELtex_A', metabolites: { phe__L_c_A: 1, 'phe__L_[smp]': -1 }, lb: -1000, ub: 1000, gpr: '', objective: 0 },
      { id: 'PHEtex_D', metabolites: { 'phe__L_[smp]': -1, phe__L_c_D: 1 }, lb: -1000, ub: 1000, gpr: '', objective: 0 },
      { id: 'EX_phe__L_[smp]', metabolites: { 'phe__L_[smp]': -1 }, lb: 0, ub: 0, gpr: '', objective: 0 },
    );
  }
  const rows = new Map(mets.map((id, i) => [id, i]));
  const ir = [], jc = [0], values = [];
  for (const reaction of reactions) {
    for (const [mid, c] of Object.entries(reaction.metabolites).sort((a, b) => rows.get(a[0]) - rows.get(b[0]))) { ir.push(rows.get(mid)); values.push(c); }
    jc.push(values.length);
  }
  return struct({
    rxns: cell(reactions.map((x) => chars(x.id))),
    mets: cell(mets.map((m) => chars(m))),
    genes: cell(['b2599', 'isozyme', 'other'].map((g) => chars(g))),
    rxnNames: cell(reactions.map((x, i) => chars(i === 1 ? '' : `${x.id} name`))),
    metNames: cell(mets.map((m) => chars(`${m} name`))),
    metFormulas: cell(mets.map((m, i) => chars(i === 0 ? 'C6H12O6' : ''))),
    metCharges: doubles(mets.map((_, i) => (i === 1 ? NaN : -i))),
    subSystems: cell(reactions.map((x, i) => cell(i % 2 ? [chars('Sub A'), chars('Sub B')] : [chars('Sub')], '', [1, i % 2 ? 2 : 1]))),
    lb: doubles(reactions.map((x) => x.lb)),
    ub: doubles(reactions.map((x) => x.ub)),
    c: doubles(reactions.map((x) => x.objective)),
    b: doubles(mets.map(() => 0)),
    csense: chars('E'.repeat(mets.length)),
    grRules: cell(reactions.map((x) => chars(x.gpr))),
    rules: cell(reactions.map((x) => chars(x.id.startsWith('BROKEN_TEXT') ? 'x(3)' : ''))),
    S: sparseMatrix(ir, jc, values, [mets.length, reactions.length]),
    description: chars(coculture ? 'Own synthetic coculture fixture' : 'Own synthetic validation fixture'),
  });
}
const seModel = struct({
  rxns: cell([chars('SOURCE'), chars('BOF')]), mets: cell([chars('phe__L_c')]), genes: cell([]),
  lb: doubles([0, 0]), ub: doubles([4, 1000]), c: doubles([0, 1]), b: doubles([0]),
  S: sparseMatrix([0, 0], [0, 1, 2], [1, -1], [1, 2]), description: chars('Own synthetic S. elongatus'),
});
const placeholder = struct({ note: chars('not used') });
const mModels = matFile(cell([placeholder, seModel, placeholder, rawModel(false)], 'SingleM', [1, 4]));
const cmModels = matFile(cell([placeholder, rawModel(true)], 'modelCo2', [1, 2]));
await writeFile(join(out, 'zuniga-M-models.mat'), mModels);
await writeFile(join(out, 'zuniga-CM-modelsKO.mat'), cmModels);

{
  const mono = (await readMat5(mModels)).SingleM;
  const co = (await readMat5(cmModels)).modelCo2;
  const monoDoc = zuniga.convertZunigaMatModel(mono[3], { id: 'toy-mono' });
  const seDoc = zuniga.convertZunigaMatModel(mono[1], { id: 'toy-se' });
  const jointDoc = zuniga.convertZunigaMatModel(co[1], { id: 'toy-co' });
  const study = { sourceModels: { mono: monoDoc, joint: jointDoc }, models: { Ec: new Model(monoDoc), Se: new Model(seDoc) }, sourceHashes: { ownFixture: 'not-an-upstream-model' } };
  const communityJSON = (c) => ({
    name: c.name, objectiveType: c.objectiveType,
    reactionOwner: mapObj(c.reactionOwner), metaboliteOwner: mapObj(c.metaboliteOwner),
    transfersByBase: mapObj(c.transfersByBase), biomassIds: mapObj(c.biomassIds), abundances: mapObj(c.abundances),
    speciesCompartments: mapObj(c.speciesCompartments), activeSpecies: c.activeSpecies.map((s) => s.id),
    normalisedAbundances: mapObj(c.normalisedAbundances()), json: c.toJSON(),
  });
  const scenarioRecords = zuniga.ZUNIGA_SCENARIOS.map((s) => {
    const prepared = zuniga.createZunigaScenario(study, s.id);
    const result = zuniga.solveZunigaScenario(prepared);
    const replay = zuniga.restoreZunigaScenario(study, s.id, prepared.model.toJSON());
    return {
      id: s.id,
      model: modelJSON(prepared.model),
      knockouts: prepared.knockouts, source: prepared.source, sourceHashes: prepared.sourceHashes,
      community: communityJSON(prepared.community),
      lb: [...prepared.problem.lb], ub: [...prepared.problem.ub],
      result: { status: result.status, objectiveValue: result.objectiveValue, fluxes: mapObj(result.fluxes), zuniga: result.zuniga },
      growth: mapObj(prepared.community.growthRates(result.fluxes)),
      crossFeeding: linkJSON(prepared.community.crossFeeding(result.fluxes)),
      explorer: explorer.buildCrossFeedingIndex(prepared.community, result.fluxes),
      replay: { knockouts: replay.knockouts, lb: [...replay.problem.lb], ub: [...replay.problem.ub] },
    };
  });
  // Net transfer, not circulation (the JS unit test's flux map).
  const circulation = structuredClone(study);
  circulation.sourceModels.joint.reactions.push(
    { id: 'PHEreturn_D', metabolites: { phe__L_c_D: -1, 'phe__L_[smp]': 1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 0, gene_reaction_rule: '', notes: { source_reaction_id: 'PHEreturn_D' } },
    { id: 'PHEreuptake_A', metabolites: { 'phe__L_[smp]': -1, phe__L_c_A: 1 }, lower_bound: 0, upper_bound: 1000, objective_coefficient: 0, gene_reaction_rule: '', notes: { source_reaction_id: 'PHEreuptake_A' } },
  );
  circulation.models = study.models;
  const circ = zuniga.createZunigaScenario(circulation, 'mutant-coculture');
  const circFluxes = new Map([...circ.model.reactions.keys()].map((id) => [id, 0]));
  for (const [id, v] of [['Se@SOURCE', 4], ['Se@BOF', 1], [`Ec@${EC_BIOMASS}`, 3], ['Se@PHELtex', -5], ['Se@PHEreuptake', 2], ['Ec@PHEtex', 4], ['Ec@PHEreturn', 1]]) circFluxes.set(id, v);

  const geneRules = ['b2599', 'b2599 or isozyme', '(b2599 and other) or (isozyme and other)', 'b2599 and (isozyme or other)', '', '  ',
    'b2599 or', '(b2599', 'b2599 )', 'b2599 other', 'and b2599', 'B2599 OR x', 'x AND (y Or b2599)', '((b2599))', '()', 'x and', ')(', 'x or　b2599', 'a(b)'];
  const mutations = [
    ['rhs', (raw) => { raw.b[0] = 1; }],
    ['sense', (raw) => { raw.csense = 'LEE'; }],
    ['sense-column', (raw) => { raw.csense = ['E', 'E', 'E']; }],
    ['sense-spaces', (raw) => { raw.csense = ' E E\tE '; }],
    ['sense-empty', (raw) => { raw.csense = ''; }],
    ['bound', (raw) => { raw.lb[0] = 4; }],
    ['nan-objective', (raw) => { raw.c[1] = NaN; }],
    ['vector', (raw) => { raw.ub = Float64Array.from([3]); }],
    ['rules-absent', (raw) => { raw.rules[4] = ''; }],
    ['rules-unknown-gene', (raw) => { raw.rules[4] = 'x(9999)'; }],
    ['rules-zero-gene', (raw) => { raw.rules[4] = 'x(0)'; }],
    ['rules-operators', (raw) => { raw.rules[4] = '(x(1) | x(2)) & x(3)'; }],
    ['rules-invalid', (raw) => { raw.rules[4] = 'x(1) |'; }],
    ['shape', (raw) => { raw.S.shape = [2, 5]; }],
    ['nan-stoichiometry', (raw) => { raw.S.values[0] = NaN; }],
    ['no-description', (raw) => { delete raw.description; }],
    ['charge-field', (raw) => { raw.metCharge = raw.metCharges; delete raw.metCharges; }],
  ];
  const convertCases = [];
  for (const [name, mutate] of mutations) {
    const raw = (await readMat5(mModels)).SingleM[3];
    mutate(raw);
    const r = attempt(() => zuniga.convertZunigaMatModel(raw, { id: name }));
    convertCases.push({ name, ...(r.error ? { error: r.error } : { ok: r.ok }) });
  }

  const sha = (bytes) => createHash('sha256').update(bytes).digest('hex');
  const manifestOf = (sources) => ({ id: 'own-fixture', sources });
  const response = (bytes, status = 200) => async () => new Response(bytes, { status });
  const loaderCases = [];
  const loader = async (name, manifest, fetcher) => {
    const progress = [];
    const r = await attemptAsync(() => zuniga.loadZunigaStudy(manifest, { fetcher, onProgress: (m) => progress.push(m) }));
    loaderCases.push({ name, manifest, ...(r.error ? { error: r.error } : { ok: true }), progress });
  };
  const one = [{ file: 'M-models.mat', url: 'https://example.invalid/own-fixture.mat', sha256: '0'.repeat(64) }];
  await loader('http-503', manifestOf(one), response('', 503));
  await loader('integrity', manifestOf(one), response(Uint8Array.from([1, 2, 3, 4])));
  const both = [
    { file: 'M-models.mat', url: 'https://example.invalid/M-models.mat', sha256: sha(mModels) },
    { file: 'CM-modelsKO.mat', url: 'https://example.invalid/CM-modelsKO.mat', sha256: sha(cmModels) },
  ];
  const serve = async (url) => new Response(url.endsWith('CM-modelsKO.mat') ? cmModels : mModels, { status: 200 });
  await loader('dimensions', manifestOf(both), serve);
  await loader('incomplete', manifestOf(both.slice(0, 1)), serve);
  await loader('no-sources', { id: 'x' }, serve);
  await loader('not-a-mat-file', manifestOf([{ file: 'M-models.mat', url: 'https://example.invalid/x', sha256: sha(Uint8Array.from([1, 2, 3])) }]), response(Uint8Array.from([1, 2, 3])));

  await write(out, 'zuniga.json', {
    scenarios: zuniga.ZUNIGA_SCENARIOS, gene: zuniga.ZUNIGA_GENE, id: zuniga.ZUNIGA_ID,
    geneRules: geneRules.map((rule) => {
      const r = attempt(() => zuniga.evaluateZunigaGeneRule(rule, new Set(['b2599'])));
      return { rule, ...(r.error ? { error: r.error } : { ok: r.ok }), none: attempt(() => zuniga.evaluateZunigaGeneRule(rule)).ok ?? null };
    }),
    converted: { mono: monoDoc, se: seDoc, joint: jointDoc },
    canonical: { joint: zuniga.canonicalZunigaModel(jointDoc), mono: zuniga.canonicalZunigaModel(monoDoc, { speciesId: 'Ec' }), monoNoSpecies: zuniga.canonicalZunigaModel(monoDoc) },
    collision: attempt(() => {
      const c = structuredClone(jointDoc);
      c.metabolites.push({ ...jointDoc.metabolites.at(-1), id: 'phe__L[smp]' });
      return zuniga.canonicalZunigaModel(c);
    }).error,
    reactionCollision: attempt(() => {
      const c = structuredClone(monoDoc);
      c.reactions.push({ ...monoDoc.reactions[0] });
      return zuniga.canonicalZunigaModel(c);
    }).error,
    scenarioResults: scenarioRecords,
    unknownScenario: attempt(() => zuniga.createZunigaScenario(study, 'missing')).error,
    unknownRestore: attempt(() => zuniga.restoreZunigaScenario(study, 'missing', study.models.Ec)).error,
    circulation: { fluxes: mapObj(circFluxes), crossFeeding: linkJSON(circ.community.crossFeeding(circFluxes)), threshold5: linkJSON(circ.community.crossFeeding(circFluxes, 5)) },
    convert: convertCases,
    loader: loaderCases,
    hashes: { 'M-models.mat': sha(mModels), 'CM-modelsKO.mat': sha(cmModels) },
  });
  console.log(`zuniga: ${scenarioRecords.length} scenarios, ${convertCases.length} conversions, ${loaderCases.length} loader cases`);
}

// Large: yeast-GEM omics

{
  const path = join(homedir(), 'Documents/GitHub/pc_model_porcine/other_models/yeast9/yeast-GEM.xml');
  if (existsSync(path)) {
    const model = new Model(parseCOBRASBML(await readFile(path, 'utf8')));
    const community = new CommunityModel({ name: 'yeast', objective: 'equal-growth', medium: {}, species: [{ id: 'sce', model, abundance: 1, enabled: true }] });
    const genes = new Set();
    for (const r of model.reactions.values()) for (const g of String(r.geneReactionRule ?? '').replace(/[()]/g, ' ').split(/\s+/)) if (g && !/^(and|or)$/i.test(g)) genes.add(g);
    const r = generator(99);
    const rows = ['gene,species,A,B'];
    [...genes].sort().forEach((g, i) => { if (i % 9 !== 4) rows.push(`${g},${i % 4 ? 'sce' : ''},${Math.round(r() * 1e4) / 100},${i % 7 ? Math.round(r() * 1e4) / 100 : ''}`); });
    const csv = `${rows.join('\n')}\n`;
    const dataset = overlay.parseOmicsCSV(csv);
    const species = [{ id: 'sce', file: 'yeast' }];
    const models = new Map([['yeast', model]]);
    const a = overlay.reactionOmicsScores(community, species, models, dataset, 'A');
    const b = overlay.reactionOmicsScores(community, species, models, dataset, 'B');
    await write(large, 'yeast9-omics.json', {
      input: path, csv,
      scores: { A: mapObj(a), B: mapObj(b) },
      log2fc: mapObj(overlay.compareReactionScores(a, b)),
      sequential: scaleJSON(overlay.sequentialOmicsColors(a)),
      report: communityOmics.communityOmicsMappingReport(community, species, models, dataset),
    });
    console.log(`yeast9 (large): ${a.size} reactions scored`);
  } else {
    console.log(`yeast9 (large): ${path} absent, skipped`);
  }
}
