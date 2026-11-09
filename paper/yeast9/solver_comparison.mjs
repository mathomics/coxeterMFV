/**
 * Solve the manuscript's complete aerobic Yeast9 model with three Coxeter methods.
 * Export the manuscript map, in the light theme, with the active TCA, cytosolic
 * isocitrate and amino acid alternatives, and ordinary FVA of every drawn reaction.
 * Run with node paper/yeast9/solver_comparison.mjs.
 */

import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { Model } from '../../src/core/model.js';
import { FluxProblem } from '../../src/solver/fba.js';
import { EditableMap } from '../../src/map/editable-map.js';
import { parseMapDocument, renameMapSpecies } from '../../src/map/map-document.js';
import { renderMap } from '../../src/render/mapview.js';
import { FluxScale } from '../../src/render/theme.js';
import { startGLPK } from '../lib/coxeter-headless.mjs';
import { addAminoAcidRoutes, addCytosolicIsocitrateDehydrogenase } from './figure_amino_acids.mjs';

const here = new URL('./', import.meta.url);
const output = new URL('results/solver-comparison/', here);
await mkdir(output, { recursive: true });
await startGLPK();

const modelText = await readFile(new URL('models/yeastgem_v9.1.1-edited.json', here), 'utf8');
const mapText = await readFile(new URL('models/yeast9_cytosol_mitochondria_peroxisome.json', here), 'utf8');
const curatedText = await readFile(new URL('../../models/maps/yeast9-9-1-1.json', here), 'utf8');
const model = new Model(JSON.parse(modelText));
const document = parseMapDocument(mapText);
const map = EditableMap.fromJSON(document.map);
const speciesId = map.cells[0].id;
const curated = renameMapSpecies(parseMapDocument(curatedText), 'sce', speciesId);
const currentMap = EditableMap.fromJSON(curated.map);
// These alternatives carry TCA flux in the complete model but are absent from the older manuscript drawing.
const addedReactions = ['r_4262', 'r_2131'];
for (const id of addedReactions) {
  const rid = `${speciesId}@${id}`;
  const reaction = currentMap.reactions.get(rid);
  if (!reaction) {
    throw new Error(`The current curated map does not contain ${id}.`);
  }
  map.reactions.set(rid, reaction);
  for (const segment of currentMap.segments) {
    if (segment.reactionId !== rid) {
      continue;
    }
    map.segments.push(segment);
    if (segment.nodeId && !map.nodes.has(segment.nodeId)) {
      map.nodes.set(segment.nodeId, currentMap.nodes.get(segment.nodeId));
    }
  }
}
addedReactions.push(...addAminoAcidRoutes(map, model, speciesId));
addedReactions.push(...addCytosolicIsocitrateDehydrogenase(map, model, speciesId));
map.texts.clear();
const problem = new FluxProblem(model, { backend: 'glpk' });
const ids = [];
const drawnIds = new Map();
for (const rid of map.reactions.keys()) {
  const original = rid.slice(rid.lastIndexOf('@') + 1);
  if (!model.reactions.has(original)) {
    throw new Error(`The manuscript map reaction ${rid} is absent from the model.`);
  }
  drawnIds.set(rid, original);
  if (!ids.includes(original)) {
    ids.push(original);
  }
}

const solutions = {
  fba: problem.optimize(),
  pfba: problem.parsimonious(),
  cyclefree: problem.cycleFree(),
};
const values = [];
const summary = {};
for (const [method, solution] of Object.entries(solutions)) {
  if (solution.status !== 'optimal') {
    throw new Error(`${method} failed: ${solution.status} ${solution.message}`);
  }
  let total = 0;
  let drawnActive = 0;
  for (const value of solution.fluxes.values()) {
    total += Math.abs(value);
  }
  for (const id of ids) {
    const value = solution.fluxes.get(id);
    values.push(value);
    // Match the manuscript's activity threshold so numerical residuals are not counted as active reactions.
    if (Math.abs(value) > 1e-6) {
      drawnActive += 1;
    }
  }
  summary[method] = {
    status: solution.status,
    objective: solution.objectiveValue,
    totalAbsoluteFlux: total,
    drawnActive,
    internalFluxBefore: solution.internalFluxBefore ?? null,
    internalFlux: solution.internalFlux ?? null,
  };
  console.log(method, JSON.stringify(summary[method]));
}

// Mid and dark tones, so that even the smallest flux stays visible over the
// pale cytoplasm and mitochondrion of the light theme in print.
const scale = new FluxScale(values, {
  dark: false,
  absoluteStops: [
    { value: 0.001, color: '#4c9be8' },
    { value: 0.01, color: '#2b59c3' },
    { value: 0.1, color: '#7b2cbf' },
    { value: 1, color: '#c2185b' },
    { value: 10, color: '#e65100' },
  ],
  absoluteLog: true,
  // The figure shows a zoomed part of the map at print size. A visible minimum
  // width keeps biosynthesis readable beside much larger respiratory fluxes.
  minWidth: 9,
  maxWidth: 26,
});
let css = await readFile(new URL('../../assets/map.css', import.meta.url), 'utf8');
css += '\n.emc-cell-plate { display: none; }\n';
// Figure 2 crops the map and names its compartments itself, so the map's own
// compartment tags, which the crop would cut through, are hidden.
css += '.emc-comp-label { display: none; }\n';
css += '.emc-map, .emc-map text { font-family: Arial, "Liberation Sans", sans-serif; }\n';
for (const [method, solution] of Object.entries(solutions)) {
  const fluxes = new Map();
  for (const [drawn, original] of drawnIds) {
    fluxes.set(drawn, solution.fluxes.get(original));
  }
  let svg = renderMap(map, {
    fluxes,
    solvedFluxes: fluxes,
    scale,
    reactionColorMode: 'absflux',
    reactionWidthMode: 'flux',
    offStyle: true,
    lineHops: true,
    labels: 'none',
    dark: false,
    forExport: true,
  });
  svg = svg.replace(/(<svg[^>]*>)/, `$1<style>${css}</style>`);
  svg = svg.replace(/\s(data-[\w-]+)(?=[\s/>])(?!\s*=)/g, ' $1=""');
  await writeFile(new URL(`map_${method}.svg`, output), svg);
}

const modelHash = createHash('sha256').update(modelText).digest('hex');
const ranges = new Map();
try {
  const previous = JSON.parse(await readFile(new URL('comparison.json', output), 'utf8'));
  if (previous.modelSHA256 === modelHash && previous.objectiveFraction === 1
      && previous.solver === 'Coxeter GLPK 5.0 / glpk.js 5.0.0') {
    for (const row of previous.rows) {
      if (Number.isFinite(row.fva?.min) && Number.isFinite(row.fva?.max)) {
        ranges.set(row.id, row.fva);
      }
    }
  }
} catch (error) {
  if (error.code !== 'ENOENT') {
    throw error;
  }
}
const pending = ids.filter((id) => !ranges.has(id));
console.log(`Computing FVA for ${pending.length} reactions; reusing ${ids.length - pending.length} matching ranges.`);
const computed = await problem.variabilityAsync(pending, {
  fraction: 1,
  chunkSize: 10,
  onProgress: (_partial, done, total) => console.log(`fva ${done}/${total}`),
});
for (const [id, range] of computed) {
  ranges.set(id, range);
}

const rows = [];
for (const id of ids) {
  const fva = ranges.get(id);
  if (![fva?.min, fva?.max].every(Number.isFinite)) {
    throw new Error(`The range of ${id} did not solve.`);
  }
  for (const method of ['pfba', 'cyclefree']) {
    const value = solutions[method].fluxes.get(id);
    if (value < fva.min - 1e-5 || value > fva.max + 1e-5) {
      throw new Error(`${method} flux of ${id} lies outside FVA.`);
    }
  }
  rows.push({
    id,
    name: model.reactions.get(id).name,
    compartments: model.reactions.get(id).compartments,
    fba: solutions.fba.fluxes.get(id),
    pfba: solutions.pfba.fluxes.get(id),
    cyclefree: solutions.cyclefree.fluxes.get(id),
    fva,
  });
}

const result = {
  model: model.id,
  condition: 'Aerobic source bounds; glucose uptake limited to 1 mmol gDW^-1 h^-1',
  modelSHA256: modelHash,
  mapSHA256: createHash('sha256').update(mapText).digest('hex'),
  curatedMapSHA256: createHash('sha256').update(curatedText).digest('hex'),
  addedReactions,
  solver: 'Coxeter GLPK 5.0 / glpk.js 5.0.0',
  objectiveFraction: 1,
  modelReactions: model.reactions.size,
  drawnReactions: ids.length,
  mapTheme: 'light',
  fontFamily: 'Arial',
  sharedScale: {
    reference: scale.reference,
    max: scale.max,
    colorMode: 'absolute-logarithmic',
    minimumVisibleWidth: scale.minWidth,
    legend: [
      { value: 0, color: '#9ca3af', width: scale.zeroWidth },
      ...scale.absoluteLegendStops(),
    ],
  },
  summary,
  rows,
};
await writeFile(new URL('comparison.json', output), JSON.stringify(result, null, 2) + '\n');
const csv = ['reaction,fba,pfba,cyclefree,fva_min,fva_max'];
for (const row of rows) {
  csv.push([row.id, row.fba, row.pfba, row.cyclefree, row.fva.min, row.fva.max].join(','));
}
await writeFile(new URL('fluxes_and_ranges.csv', output), csv.join('\n') + '\n');
const figureDocument = {
  ...document,
  metadata: {
    ...document.metadata,
    reactionCount: map.reactions.size,
    nodeCount: map.nodes.size,
    segmentCount: map.segments.length,
    textCount: 0,
  },
  map: map.toJSON(),
};
await writeFile(new URL('figure_map.json', output), JSON.stringify(figureDocument, null, 2) + '\n');
const tca = [];
for (const id of ['r_0302', 'r_0280', 'r_4262', 'r_0658', 'r_2131', 'r_0659', 'r_1112', 'r_1128']) {
  const reaction = model.reactions.get(id);
  tca.push({
    id,
    name: reaction.name,
    bounds: [reaction.lowerBound, reaction.upperBound],
    stoichiometry: reaction.metabolites,
    fba: solutions.fba.fluxes.get(id),
    pfba: solutions.pfba.fluxes.get(id),
    cyclefree: solutions.cyclefree.fluxes.get(id),
    drawn: ids.includes(id),
  });
}
await writeFile(new URL('tca_diagnostic.json', output), JSON.stringify(tca, null, 2) + '\n');
const aminoAcidExchanges = [
  'r_1810', 'r_1873', 'r_1879', 'r_1880', 'r_1881', 'r_1883', 'r_1889',
  'r_1891', 'r_1893', 'r_1897', 'r_1899', 'r_1900', 'r_1902', 'r_1903',
  'r_1904', 'r_1906', 'r_1911', 'r_1912', 'r_1913', 'r_1914',
];
const aminoAcids = [];
for (const id of aminoAcidExchanges) {
  const exchange = model.reactions.get(id);
  const extracellularId = Object.keys(exchange.metabolites)[0];
  const extracellular = model.metabolites.get(extracellularId);
  let cytosolic = null;
  for (const metabolite of model.metabolites.values()) {
    if (metabolite.name === extracellular.name && metabolite.compartment === 'c') {
      cytosolic = metabolite;
      break;
    }
  }
  if (!cytosolic || exchange.lowerBound < 0 || exchange.metabolites[extracellularId] !== -1) {
    throw new Error(`Check the amino acid uptake constraint for ${id}.`);
  }
  const charging = [];
  for (const reaction of model.reactions.values()) {
    if (reaction.name.toLowerCase().includes('trna synthetase') && reaction.metabolites[cytosolic.id] < 0) {
      charging.push(reaction.id);
    }
  }
  const fluxes = {};
  for (const [method, solution] of Object.entries(solutions)) {
    let charged = 0;
    for (const chargingId of charging) {
      charged += solution.fluxes.get(chargingId);
    }
    const exchangeFlux = solution.fluxes.get(id);
    if (exchangeFlux < -1e-6 || charged <= 1e-6) {
      throw new Error(`${method} does not demonstrate blocked uptake and positive tRNA charging for ${id}.`);
    }
    fluxes[method] = { exchange: exchangeFlux, cytosolicTRNACharging: charged };
  }
  aminoAcids.push({
    name: extracellular.name,
    exchange: id,
    bounds: [exchange.lowerBound, exchange.upperBound],
    cytosolicMetabolite: cytosolic.id,
    chargingReactions: charging,
    fluxes,
  });
}
const activeExchanges = [];
for (const reaction of model.reactions.values()) {
  if (Object.keys(reaction.metabolites).length !== 1) {
    continue;
  }
  const fluxes = {};
  let active = false;
  for (const [method, solution] of Object.entries(solutions)) {
    const value = solution.fluxes.get(reaction.id);
    fluxes[method] = value;
    if (Math.abs(value) > 1e-6) {
      active = true;
    }
  }
  if (active) {
    activeExchanges.push({ id: reaction.id, name: reaction.name, fluxes });
  }
}
await writeFile(new URL('amino_acid_diagnostic.json', output), JSON.stringify({ aminoAcids, activeExchanges }, null, 2) + '\n');
console.log(`Saved ${fileURLToPath(output)}`);
