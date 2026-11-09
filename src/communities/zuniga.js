/**
 * Exact source-matrix FBA for the published ΔpheA rescue experiment of
 * Zuñiga et al. (2020) Nat Commun 11:3803, doi:10.1038/s41467-020-17612-8.
 * Models are downloaded privately, never redistributed with Coxeter.
 */
import { readMat5 } from '../import/mat5.js';
import { Model, stripCompartment } from '../core/model.js';
import { FluxProblem } from '../solver/fba.js';
import { evaluateGeneRule } from '../core/gene-rules.js';

export const ZUNIGA_ID = 'zuniga-2020-phea-rescue';
export const ZUNIGA_SCENARIOS = [
  { id: 'wt-monoculture', name: 'WT monoculture', mutant: false, coculture: false },
  { id: 'mutant-monoculture', name: 'Mutant monoculture', mutant: true, coculture: false },
  { id: 'mutant-coculture', name: 'Mutant coculture', mutant: true, coculture: true },
];
export const ZUNIGA_GENE = 'b2599';
const BIOMASS_EC = 'Ec@BIOMASS_Ec_iJO1366_core_53p95M';
const BIOMASS_SE = 'Se@BOF';
const clone = (value) => JSON.parse(JSON.stringify(value));
const array = (value) => value == null ? [] : Array.isArray(value) || ArrayBuffer.isView(value) ? Array.from(value) : [value];
const text = (value) => typeof value === 'string' ? value : array(value).map(text).join('; ');

/** The general GPR evaluator (src/core/gene-rules.js), under the name the case exports. */
export const evaluateZunigaGeneRule = evaluateGeneRule;

function sourceGeneRule(raw, index, genes) {
  const original = text(array(raw.grRules)[index] ?? '');
  try { evaluateZunigaGeneRule(original); return { rule: original, notes: {} }; }
  catch {
    const encoded = text(array(raw.rules)[index] ?? '');
    if (!encoded) throw new Error(`No valid original GPR for reaction ${index}.`);
    const translated = encoded.replace(/x\((\d+)\)/g, (_, n) => {
      const gene = genes[Number(n) - 1];
      if (!gene) throw new Error('Original GPR references an unknown gene.');
      return gene;
    }).replace(/&/g, ' and ').replace(/\|/g, ' or ');
    evaluateZunigaGeneRule(translated);
    return { rule: translated, notes: { original_gr_rule: original, original_rules: encoded, gpr_source: 'rules: original indexed Boolean expression' } };
  }
}

/**
 * Convert a COBRA struct read from a MAT file into a COBRA JSON document, keeping
 * every coefficient, bound and objective in original MATLAB order.
 */
export function convertZunigaMatModel(raw, { id = 'zuniga-source' } = {}) {
  const rxns = array(raw.rxns).map(text), mets = array(raw.mets).map(text), genes = array(raw.genes).map(text);
  const S = raw.S;
  if (!S?.ir || !S?.jc || !S?.values || S.shape?.[0] !== mets.length || S.shape?.[1] !== rxns.length) {
    throw new Error('The original sparse matrix does not match its identifiers.');
  }
  const lb = array(raw.lb), ub = array(raw.ub), objective = array(raw.c), rhs = array(raw.b);
  if ([lb, ub, objective].some(values => values.length !== rxns.length)) throw new Error('Incomplete source reaction vectors.');
  if (rhs.length !== mets.length || rhs.some(value => value !== 0)) throw new Error('This case requires the original zero equality RHS.');
  if (raw.csense && text(raw.csense).replace(/\s/g, '').split('').some(value => value !== 'E')) throw new Error('Unsupported source constraint sense.');
  const compartments = {};
  const metNames = array(raw.metNames), formulas = array(raw.metFormulas), charges = array(raw.metCharges ?? raw.metCharge);
  const metabolites = mets.map((mid, i) => {
    const info = originalMetaboliteInfo(mid);
    compartments[info.compartment] = info.compartment;
    return { id: mid, name: text(metNames[i] ?? mid), compartment: info.compartment,
      base: info.base, formula: text(formulas[i] ?? ''), charge: Number.isFinite(charges[i]) ? charges[i] : null };
  });
  const names = array(raw.rxnNames), subsystems = array(raw.subSystems);
  const reactions = rxns.map((rid, j) => {
    const stoich = {};
    for (let k = S.jc[j]; k < S.jc[j + 1]; k++) {
      if (!Number.isFinite(S.values[k]) || !mets[S.ir[k]]) throw new Error('Invalid source stoichiometry.');
      if (S.values[k]) stoich[mets[S.ir[k]]] = S.values[k];
    }
    if (![lb[j], ub[j], objective[j]].every(Number.isFinite) || lb[j] > ub[j]) throw new Error('Invalid original reaction bounds or objective.');
    const gpr = sourceGeneRule(raw, j, genes);
    return { id: rid, name: text(names[j] ?? rid), metabolites: stoich,
      lower_bound: lb[j], upper_bound: ub[j], objective_coefficient: objective[j],
      subsystem: text(subsystems[j] ?? ''), gene_reaction_rule: gpr.rule,
      notes: { source_reaction_id: rid, ...gpr.notes } };
  });
  return { id, name: text(raw.description ?? id), metabolites, reactions,
    genes: genes.map(gene => ({ id: gene, name: gene })), compartments,
    notes: { source_description: text(raw.description), source_reaction_count: rxns.length, source_metabolite_count: mets.length,
      source_matrix_nonzeros: S.values.length, namespace_conversion: 'Identifiers only; no abundance scaling, added reactions, medium changes or objective rewiring.' } };
}

function originalMetaboliteInfo(mid) {
  if (mid.endsWith('[smp]')) return { speciesId: null, originalId: mid, compartment: 's', base: mid.replace(/_?\[smp\]$/, '') };
  const suffix = /_(A|D)$/.exec(mid);
  const local = suffix ? mid.slice(0, -2) : mid;
  const compartment = /_([a-z][a-z0-9]?)$/.exec(local)?.[1] ?? 'c';
  return { speciesId: suffix ? suffix[1] === 'A' ? 'Se' : 'Ec' : null, originalId: local,
    compartment, base: stripCompartment(local, compartment) };
}

/**
 * Rename identifiers into Coxeter's namespace (`Ec@PPNDH`, `M@base` for the
 * shared [smp] pool) as a lossless bijection. The LP coefficients are unchanged,
 * and each element keeps its source id in its notes. `speciesId` puts every id
 * on one member, for a monoculture.
 */
export function canonicalZunigaModel(source, { speciesId = null } = {}) {
  const doc = clone(source);
  const metIds = new Map();
  const usedMetabolites = new Set();
  for (const met of doc.metabolites) {
    const info = originalMetaboliteInfo(met.id);
    const species = speciesId ?? info.speciesId;
    const target = met.id.endsWith('[smp]') ? `M@${info.base}` : species ? `${species}@${info.originalId}` : met.id;
    if (usedMetabolites.has(target)) throw new Error('Metabolite namespace collision.');
    usedMetabolites.add(target);
    metIds.set(met.id, target);
    met.notes = { ...(met.notes ?? {}), source_metabolite_id: met.id };
    met.id = target;
    met.base = info.base;
    met.compartment = info.compartment;
  }
  const used = new Set();
  for (const reaction of doc.reactions) {
    const original = reaction.id;
    const suffix = /_(A|D)$/.exec(original);
    const species = speciesId ?? (suffix ? suffix[1] === 'A' ? 'Se' : 'Ec' : null);
    const local = suffix && !speciesId ? original.slice(0, -2) : original;
    const entries = Object.entries(reaction.metabolites);
    const onlyShared = entries.length === 1 && entries[0][0].endsWith('[smp]');
    reaction.id = species ? `${species}@${local}` : onlyShared && original.startsWith('EX_')
      ? `EX_${originalMetaboliteInfo(entries[0][0]).base}_s` : original;
    if (used.has(reaction.id)) throw new Error('Reaction namespace collision.');
    used.add(reaction.id);
    reaction.metabolites = Object.fromEntries(entries.map(([mid, value]) => [metIds.get(mid), value]));
    reaction.notes = { ...(reaction.notes ?? {}), source_reaction_id: original };
  }
  doc.id = `${source.id}-canonical`;
  return doc;
}

function exactCommunity(model, study, scenario) {
  const members = ['Ec', 'Se'].map(id => ({ id, model: study.models[id], abundance: 1, enabled: id === 'Ec' || scenario.coculture,
    color: id === 'Ec' ? '#b45309' : '#047857' }));
  const reactionOwner = new Map(), metaboliteOwner = new Map(), transfersByBase = new Map();
  for (const met of model.metabolites.values()) {
    const match = /^(Ec|Se)@(.+)$/.exec(met.id);
    metaboliteOwner.set(met.id, { speciesId: match?.[1] ?? null, originalId: match?.[2] ?? met.base, compartment: met.compartment });
  }
  for (const rxn of model.reactions.values()) {
    const match = /^(Ec|Se)@(.+)$/.exec(rxn.id);
    const shared = Object.entries(rxn.metabolites).filter(([mid]) => mid.startsWith('M@'));
    const role = match && shared.length ? 'transfer' : rxn.kind;
    reactionOwner.set(rxn.id, { speciesId: match?.[1] ?? null, originalId: match?.[2] ?? rxn.id, role });
    if (match) for (const [mid, coefficient] of shared) {
      const base = mid.slice(2), localId = Object.keys(rxn.metabolites).find(id => id.startsWith(`${match[1]}@`));
      if (!transfersByBase.has(base)) transfersByBase.set(base, []);
      transfersByBase.get(base).push({ speciesId: match[1], reactionId: rxn.id, localId, coefficient });
    }
  }
  const biomassIds = new Map([['Ec', BIOMASS_EC]]);
  if (scenario.coculture) biomassIds.set('Se', BIOMASS_SE);
  const community = {
    name: 'Zuñiga et al. (2020): ΔpheA rescue', objectiveType: 'source-biomass-objective', productTarget: null, medium: {},
    species: members, merged: model, reactionOwner, metaboliteOwner, transfersByBase, biomassIds,
    abundances: new Map(members.map(member => [member.id, member.enabled ? 1 : 0])),
    speciesCompartments: new Map(members.map(member => [member.id, member.model.compartmentIds()])),
    get activeSpecies() { return this.species.filter(member => member.enabled); },
    normalisedAbundances() { return new Map(this.abundances); },
    growthRates(fluxes) { return new Map([...biomassIds].map(([id, rxn]) => [id, fluxes.get(rxn) ?? 0])); },
    crossFeeding(fluxes, threshold = 1e-6) { return exactCrossFeeding(this, fluxes, threshold); },
    toJSON() { return { name: this.name, objective: this.objectiveType, species: members.map(member => ({ id: member.id, enabled: member.enabled, model: member.model.id })) }; },
  };
  return community;
}

function exactCrossFeeding(community, fluxes, threshold) {
  const links = [], perMetabolite = [];
  for (const [base, transfers] of community.transfersByBase) {
    const producers = [], consumers = [];
    const bySpecies = new Map();
    for (const transfer of transfers) {
      const net = (fluxes.get(transfer.reactionId) ?? 0) * transfer.coefficient;
      bySpecies.set(transfer.speciesId, (bySpecies.get(transfer.speciesId) ?? 0) + net);
    }
    for (const [speciesId, net] of bySpecies) {
      if (net > threshold) producers.push({ speciesId, flux: net });
      if (net < -threshold) consumers.push({ speciesId, flux: -net });
    }
    const boundary = community.merged.reactions.get(`EX_${base}_s`);
    const net = boundary ? (fluxes.get(boundary.id) ?? 0) * (boundary.metabolites[`M@${base}`] ?? 0) : 0;
    if (net > threshold) producers.push({ speciesId: null, flux: net });
    if (net < -threshold) consumers.push({ speciesId: null, flux: -net });
    const totalOut = producers.reduce((sum, entry) => sum + entry.flux, 0);
    const totalIn = consumers.reduce((sum, entry) => sum + entry.flux, 0);
    const total = Math.min(totalOut, totalIn);
    if (totalOut || totalIn) perMetabolite.push({ base, producers, consumers, total: total || totalOut || totalIn });
    if (totalOut > 0 && totalIn > 0) for (const producer of producers) for (const consumer of consumers) {
      if (producer.speciesId === consumer.speciesId) continue;
      const flux = total * producer.flux / totalOut * consumer.flux / totalIn;
      if (flux > threshold) links.push({ base, from: producer.speciesId, to: consumer.speciesId, flux });
    }
  }
  links.sort((a, b) => b.flux - a.flux);
  return { links, perMetabolite };
}

/**
 * Build one of ZUNIGA_SCENARIOS from a loaded study. The mutant cases apply the
 * b2599 deletion, which must block exactly Ec@PPNDH. Returns what
 * restoreZunigaScenario returns.
 */
export function createZunigaScenario(study, scenarioId) {
  const scenario = ZUNIGA_SCENARIOS.find(entry => entry.id === scenarioId);
  if (!scenario) throw new Error(`Unknown Zuñiga scenario: ${scenarioId}`);
  const raw = scenario.coculture ? study.sourceModels.joint : study.sourceModels.mono;
  const doc = canonicalZunigaModel(raw, { speciesId: scenario.coculture ? null : 'Ec' });
  const knockouts = [];
  if (scenario.mutant) {
    for (const rxn of doc.reactions) if (!evaluateZunigaGeneRule(rxn.gene_reaction_rule, new Set([ZUNIGA_GENE]))) {
      rxn.lower_bound = 0; rxn.upper_bound = 0; knockouts.push(rxn.id);
    }
    if (knockouts.length !== 1 || knockouts[0] !== 'Ec@PPNDH') throw new Error('The original pheA deletion does not match the documented PPNDH knockout.');
  }
  return restoreZunigaScenario(study, scenarioId, new Model(doc), knockouts);
}

/** Replay a personal export against its full canonical LP, without a merger. */
export function restoreZunigaScenario(study, scenarioId, activeModel, knockouts = null) {
  const scenario = ZUNIGA_SCENARIOS.find(entry => entry.id === scenarioId);
  if (!scenario) throw new Error('Unknown case scenario.');
  const model = activeModel instanceof Model ? activeModel : new Model(activeModel);
  const community = exactCommunity(model, study, scenario);
  return { model, community, problem: new FluxProblem(model, { backend: 'glpk' }), scenario,
    knockouts: knockouts ?? (scenario.mutant ? ['Ec@PPNDH'] : []), sourceHashes: { ...study.sourceHashes },
    source: scenario.coculture ? 'CM-modelsKO.mat:modelCo2{2}' : 'M-models.mat:SingleM{4}' };
}

/**
 * Solve a prepared scenario with its source objective and attach the provenance
 * as `result.zuniga`, including the fluxes keyed by source reaction id.
 */
export function solveZunigaScenario(prepared) {
  const result = prepared.problem.optimize();
  result.zuniga = { id: ZUNIGA_ID, scenarioId: prepared.scenario.id, gene: prepared.scenario.mutant ? ZUNIGA_GENE : null,
    knockouts: [...prepared.knockouts], source: prepared.source, sourceHashes: prepared.sourceHashes,
    method: 'Stationary FBA with the source objective; no equal-growth or abundance scaling',
    originalFluxes: Object.fromEntries([...result.fluxes].map(([id, value]) => [prepared.model.reactions.get(id)?.notes?.source_reaction_id ?? id, value])) };
  return result;
}

/**
 * Download the MAT files listed in `manifest.sources`, check each SHA-256, cache
 * the bytes in the browser's IndexedDB and convert the three source models.
 * Throws when a hash or a pinned model dimension does not match.
 */
export async function loadZunigaStudy(manifest, { fetcher = globalThis.fetch, onProgress = () => {} } = {}) {
  const sources = {};
  const sourceHashes = {};
  for (const source of manifest.sources ?? []) {
    onProgress(`Downloading the original ${source.file}…`);
    const key = `${source.url}#${source.sha256}`;
    let bytes = await privateCacheGet(key);
    if (!bytes) {
      const response = await fetcher(source.url);
      if (!response.ok) throw new Error(`${source.file}: HTTP ${response.status}`);
      bytes = await response.arrayBuffer();
    }
    const hash = await binarySha256(bytes);
    if (hash !== source.sha256) throw new Error(`Source integrity check failed for ${source.file}.`);
    await privateCachePut(key, bytes);
    sourceHashes[source.file] = hash;
    onProgress(`Reading the full original ${source.file}…`);
    sources[source.file] = await readMat5(bytes);
  }
  const single = sources['M-models.mat']?.SingleM;
  const joint = sources['CM-modelsKO.mat']?.modelCo2;
  if (!single?.[3] || !single?.[1] || !joint?.[1]) throw new Error('The original model cell arrays are incomplete.');
  const mono = convertZunigaMatModel(single[3], { id: 'zuniga-iJO1366' });
  const se = convertZunigaMatModel(single[1], { id: 'zuniga-iJB785-cscB' });
  const co = convertZunigaMatModel(joint[1], { id: 'zuniga-original-knockout-analysis-community' });
  for (const [doc, rxns, mets] of [[mono, 2583, 1805], [se, 899, 793], [co, 3162, 2547]]) {
    if (doc.reactions.length !== rxns || doc.metabolites.length !== mets) throw new Error('The pinned source model dimensions do not match.');
  }
  mono.cell = { name: 'E. coli K-12', organism: 'Escherichia coli K-12 MG1655', color: '#b45309' };
  se.cell = { name: 'S. elongatus cscB+', organism: 'Synechococcus elongatus PCC7942 cscB+', color: '#047857' };
  return { manifest, sourceHashes, sourceModels: { mono, joint: co }, models: { Ec: new Model(mono), Se: new Model(se) } };
}

async function binarySha256(bytes) {
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  return [...new Uint8Array(digest)].map(value => value.toString(16).padStart(2, '0')).join('');
}

async function privateDatabase() {
  if (typeof indexedDB === 'undefined') return null;
  return new Promise(resolve => {
    const request = indexedDB.open('coxeter-public-model-cache', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('sources');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => resolve(null);
  });
}
async function privateCacheGet(key) {
  const db = await privateDatabase();
  if (!db) return null;
  return new Promise(resolve => {
    const transaction = db.transaction('sources', 'readonly');
    const request = transaction.objectStore('sources').get(key);
    request.onsuccess = () => resolve(request.result ?? null);
    request.onerror = () => resolve(null);
    transaction.oncomplete = () => db.close();
  });
}
async function privateCachePut(key, bytes) {
  const db = await privateDatabase();
  if (!db) return;
  await new Promise(resolve => {
    const transaction = db.transaction('sources', 'readwrite');
    transaction.objectStore('sources').put(bytes, key);
    transaction.oncomplete = transaction.onerror = transaction.onabort = () => { db.close(); resolve(); };
  });
}
