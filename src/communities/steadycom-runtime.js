/**
 * Run SteadyCom on a published community from inside the browser.
 *
 * src/core/steadycom.js holds the method, and this module holds what the
 * application needs to hand it. The tools/ scripts run SteadyCom from the
 * papers' own source files and record the result, which answers "what did this
 * community turn out to be". Running it in the browser answers "what would it do
 * at a growth rate I choose", which is the question the formulation is actually
 * built around: fix mu and the problem is an ordinary LP.
 *
 * A published community ships everything needed:
 *
 *   model-<member>.json   the member model with its own bounds already applied
 *   media.csv             metabolite, exchange reaction, lower and upper bound
 *                         for the whole community
 *   bounds.csv            complete reaction ownership and bounds for a joint
 *                         model with an explicit shared [u] compartment
 *
 * so the spec is assembled from those rather than rebuilt from the paper.
 *
 * The single-file build embeds the complete SteadyCom members and their CSV
 * constraints as well as the map excerpts. A build that carries only the
 * excerpts cannot solve these genome-scale problems, and the store detects that
 * missing input.
 */

import { exchangeBase } from './steadycom-ecoli.js';
import { Model, stripCompartment } from '../core/model.js';
import { OBJECTIVES, LINK_COMPARTMENT } from '../core/community.js';
import { fiberConstraints } from './gut9-figure4.js';

/** Single-metabolite extracellular boundaries, keyed by the metabolite traded. */
export function exchangeIndex(model) {
  const out = {};
  for (const [rid, reaction] of model.reactions) {
    const entries = Object.entries(reaction.metabolites);
    if (entries.length !== 1) continue;
    const [mid, coefficient] = entries[0];
    const metabolite = model.metabolites.get(mid);
    const external = metabolite && (metabolite.compartment === 'e'
      || /extracellular|external/i.test(model.compartmentNames[metabolite.compartment] ?? ''));
    if (!external && !rid.startsWith('EX_')) continue;
    if (coefficient !== -1) {
      throw new Error(`${rid}: SteadyCom exchange requires one consumed metabolite with coefficient -1.`);
    }
    const base = metabolite
      ? stripCompartment(mid, metabolite.compartment).replace(/\[[^\]]+\]$/, '')
      : exchangeBase(rid);
    if (out[base] && out[base] !== rid) {
      throw new Error(`${base}: more than one exchange reaction needs an explicit community formulation.`);
    }
    out[base] = rid;
  }
  return out;
}

/**
 * Read `media.csv`: `metabolite,exchange_reaction,lower_bound,upper_bound`.
 *
 * A negative lower bound is uptake, and SteadyCom wants it as a positive budget,
 * so the sign is flipped here. Rows that only allow secretion carry no budget and
 * are left out.
 */
export function parseMediaCSV(text, members = []) {
  const uptake = {};
  const lines = String(text ?? '').trim().split(/\r?\n/);
  const header = (lines.shift() ?? '').split(',').map((s) => s.trim());
  const col = (name) => header.indexOf(name);
  const iMet = col('metabolite');
  const iLower = col('lower_bound');
  const iExchange = col('exchange_reaction');
  const basesByReaction = new Map();
  for (const { model } of members) {
    for (const [base, reaction] of Object.entries(exchangeIndex(model))) {
      if (basesByReaction.has(reaction) && basesByReaction.get(reaction) !== base) {
        throw new Error(`${reaction}: member exports disagree about the exchanged metabolite.`);
      }
      basesByReaction.set(reaction, base);
    }
  }
  if (iMet < 0 || iLower < 0) {
    throw new Error('media.csv has no "metabolite" and "lower_bound" columns.');
  }

  for (const line of lines) {
    if (!line.trim()) continue;
    const cells = line.split(',');
    const lower = Number(cells[iLower]);
    if (!Number.isFinite(lower) || lower >= 0) continue;
    const reaction = iExchange >= 0 ? cells[iExchange]?.trim() : null;
    const base = members.length ? basesByReaction.get(reaction) : cells[iMet].trim();
    if (!base) throw new Error(`${reaction ?? cells[iMet]}: the medium uptake has no matching member exchange.`);
    uptake[base] = -lower;
  }
  return uptake;
}

/**
 * The SteadyCom problem for a community of separate member models.
 *
 * @param {{id:string, model:import('../core/model.js').Model}[]} members
 * @param {Object<string,number>} communityUptake  metabolite -> budget (positive)
 * @param {number} [totalBiomass=1]
 */
export function steadyComSpec(members, communityUptake, totalBiomass = 1) {
  if (!members?.length) throw new Error('A SteadyCom problem needs at least one member.');

  return {
    members: members.map(({ id, model }) => {
      const biomassReaction = model.biomassReactionId();
      if (!biomassReaction) {
        throw new Error(`${id} has no biomass reaction, so its growth cannot be coupled.`);
      }
      return { id, model, biomassReaction, exchangeFor: exchangeIndex(model) };
    }),
    communityUptake: { ...communityUptake },
    totalBiomass,
    metadata: { kind: 'separate' },
  };
}

/**
 * Reassemble a published joint community from complete member exports and its
 * complete bounds table. Shared [u] metabolites remain shared and are never
 * stripped or duplicated into separate member mass balances.
 *
 * `fiber`, when given as { bacteroides, faecalibacterium } in C-mmol/gDW/h,
 * adds the Figure 4 fiber uptake constraints (see gut9-figure4.js).
 */
export function jointSteadyComSpec(members, boundsCSV, { totalBiomass = 1, fiber = null } = {}) {
  if (!members?.length) throw new Error('A joint SteadyCom problem needs member models.');
  if (!Number.isFinite(totalBiomass) || totalBiomass <= 0) throw new Error('Total biomass must be positive and finite.');
  if (fiber && ![fiber.bacteroides, fiber.faecalibacterium].every(value => Number.isFinite(value) && value > 0)) {
    throw new Error('Both fiber uptake limits must be positive and finite.');
  }
  const bounds = jointBounds(boundsCSV);
  const reactions = new Map();
  const metabolites = new Map();
  const reactionOwners = new Map();
  const bases = new Map();
  const speciesExchange = new Map();
  const communityUptake = new Map();
  const communityExport = new Map();
  const memberIds = new Set();
  const descriptors = [];

  for (const { id, model } of members) {
    if (!id || id === 'comm' || memberIds.has(id)) throw new Error(`Invalid or duplicate member ${id}.`);
    memberIds.add(id);
    const biomassReaction = model.biomassReactionId();
    if (!biomassReaction || !model.reactions.has(biomassReaction)) throw new Error(`${id} has no valid biomass reaction.`);
    descriptors.push({ id, name: model.name, biomassReaction });
    for (const metabolite of model.metabolites.values()) {
      const shared = metabolite.id.endsWith('[u]');
      const doc = { ...metabolite.toJSON(), species: shared ? 'comm' : id };
      const previous = metabolites.get(doc.id);
      if (previous && JSON.stringify(previous) !== JSON.stringify(doc)) {
        throw new Error(`${doc.id}: member exports disagree about the shared metabolite.`);
      }
      metabolites.set(doc.id, doc);
      if (shared) bases.set(doc.id, doc.id.slice(0, -3));
    }
    for (const reaction of model.reactions.values()) {
      const row = bounds.get(reaction.id);
      if (!row || row.species !== id) throw new Error(`${reaction.id}: no matching bounds for member ${id}.`);
      if (reactions.has(reaction.id)) throw new Error(`Duplicate joint reaction ${reaction.id}.`);
      reactions.set(reaction.id, {
        id: reaction.id, name: reaction.name, subsystem: reaction.subsystem,
        metabolites: { ...reaction.metabolites }, species: id,
        lowerBound: row.lower, upperBound: row.upper, objective: row.objective,
      });
      reactionOwners.set(reaction.id, id);
      const shared = Object.entries(reaction.metabolites).filter(([mid]) => mid.endsWith('[u]'));
      if (!shared.length) continue;
      const local = Object.entries(reaction.metabolites).filter(([mid]) => !mid.endsWith('[u]'));
      if (shared.length !== 1 || shared[0][1] !== 1 || local.length !== 1 || local[0][1] !== -1) {
        throw new Error(`${reaction.id}: unsupported species-to-community exchange structure.`);
      }
      const key = `${shared[0][0]}\u0000${id}`;
      if (speciesExchange.has(key)) throw new Error(`${shared[0][0]}: duplicate community exchange for ${id}.`);
      speciesExchange.set(key, reaction.id);
    }
  }

  for (const [rid, row] of bounds) {
    if (row.species !== 'comm') {
      if (!reactions.has(rid)) throw new Error(`${rid}: bounds table names a missing member reaction.`);
      continue;
    }
    const match = /^(UT|EX)_(.+)\(u\)$/.exec(rid);
    if (!match) throw new Error(`${rid}: unsupported community boundary identifier.`);
    const met = `${match[2]}[u]`;
    bases.set(met, match[2]);
    if (!metabolites.has(met)) metabolites.set(met, { id: met, name: met, compartment: 'u', species: 'comm' });
    const uptake = match[1] === 'UT';
    const index = uptake ? communityUptake : communityExport;
    if (index.has(met)) throw new Error(`${met}: duplicate community ${uptake ? 'uptake' : 'export'} boundary.`);
    index.set(met, rid);
    reactions.set(rid, {
      id: rid, name: rid, metabolites: { [met]: uptake ? 1 : -1 }, species: 'comm',
      lowerBound: row.lower, upperBound: row.upper, objective: row.objective,
    });
    reactionOwners.set(rid, null);
  }
  for (const met of bases.keys()) {
    if (!communityUptake.has(met) || !communityExport.has(met)) {
      throw new Error(`${met}: the bounds table lacks a community uptake or export boundary.`);
    }
  }
  const constraints = fiber ? fiberConstraints({ reactions, speciesExchange }, fiber) : [];
  return {
    members: descriptors, reactions, metabolites, communityTag: 'comm', totalBiomass, constraints,
    metadata: {
      kind: 'joint', reactionOwners, bases, exchanges: speciesExchange, speciesExchange,
      communityUptake, communityExport,
      fiber: fiber ? { ...fiber } : null,
    },
  };
}

/** Snapshot a spec for a Worker. Maps survive structured cloning, and Model instances become documents. */
export function serializeSteadyComSpec(spec) {
  return structuredClone({
    ...spec,
    members: spec.members.map(({ model, ...member }) => ({
      ...member, ...(model ? { model: model.toJSON() } : {}),
    })),
  });
}

/** Restore a worker snapshot, giving every separate member its own Model instance. */
export function restoreSteadyComSpec(snapshot) {
  const copy = structuredClone(snapshot);
  copy.members = copy.members.map(({ model, ...member }) => ({
    ...member, ...(model ? { model: new Model(model) } : {}),
  }));
  return copy;
}

/**
 * Absolute community rates from the joint solution. Positive species exchange
 * means secretion. Uptake and export are the actual UT/EX columns, not budgets.
 * Keys are shared metabolite identifiers, with display bases in metadata.bases.
 */
export function communityBalances(spec, fluxes) {
  const metadata = spec.metadata;
  const speciesExchanges = new Map();
  const communityUptake = new Map();
  const communityExport = new Map();
  const value = (rid) => Number(fluxes?.get(rid) ?? 0);
  for (const [key, rid] of metadata?.speciesExchange ?? []) {
    const at = key.indexOf('\u0000');
    const met = key.slice(0, at);
    const species = key.slice(at + 1);
    if (!speciesExchanges.has(met)) speciesExchanges.set(met, new Map());
    const perSpecies = speciesExchanges.get(met);
    perSpecies.set(species, (perSpecies.get(species) ?? 0) + value(rid));
  }
  for (const [met, rid] of metadata?.communityUptake ?? []) communityUptake.set(met, value(rid));
  for (const [met, rid] of metadata?.communityExport ?? []) communityExport.set(met, value(rid));
  return { speciesExchanges, communityUptake, communityExport };
}

/**
 * Conventional FBA at a given composition. Member fluxes are specific rates,
 * and their shared-medium coefficients carry the supplied biomass amounts. This
 * projection does not introduce SteadyCom's variable abundances or fiber rows.
 *
 * `abundances` is a Map or object of member id -> biomass amount. Returns the
 * merged Model with the id and ownership maps that relate it back to the spec.
 */
export function jointCommunityProjection(spec, abundances, {
  objective = OBJECTIVES.EQUAL_GROWTH, productTarget = null,
} = {}) {
  if (!spec.reactions || spec.metadata?.kind !== 'joint') throw new Error('A joint community projection needs a joint SteadyCom spec.');
  if (!Object.values(OBJECTIVES).includes(objective)) throw new Error(`Unsupported community objective ${objective}.`);
  const amounts = new Map();
  for (const member of spec.members) {
    const amount = abundances instanceof Map ? abundances.get(member.id) : abundances?.[member.id];
    if (!Number.isFinite(amount) || amount < 0) throw new Error(`${member.id}: fixed abundance must be finite and nonnegative (got ${amount}).`);
    amounts.set(member.id, amount);
  }
  if (![...amounts.values()].some(amount => amount > 0)) throw new Error('A fixed community needs at least one positive abundance.');

  const compartments = { s: 'Shared medium', [LINK_COMPARTMENT]: 'Growth coupling (virtual)' };
  const metabolites = [];
  const metaboliteOwner = new Map();
  const metaboliteIds = new Map();
  const speciesCompartments = new Map(spec.members.map(member => [member.id, new Set()]));
  for (const [id, metabolite] of spec.metabolites) {
    const shared = spec.metadata.bases.has(id);
    const compartment = shared ? 's' : projectionCompartment(metabolite);
    const owner = shared ? null : metabolite.species;
    if (!shared && !amounts.has(owner)) throw new Error(`${id}: no member owns this metabolite.`);
    const base = shared ? spec.metadata.bases.get(id) : null;
    const mergedId = shared ? `M@${base}_s` : `${owner}@${id}`;
    metaboliteIds.set(id, mergedId);
    compartments[compartment] ??= ({ c: 'Cytosol', e: 'Extracellular', p: 'Periplasm' })[compartment] ?? compartment;
    metabolites.push({
      ...metabolite, id: mergedId, compartment,
      name: metabolite.name ?? id, formula: metabolite.formula ?? '', charge: metabolite.charge ?? 0,
    });
    metaboliteOwner.set(mergedId, { speciesId: owner, originalId: shared ? base : id, compartment });
    if (!shared) speciesCompartments.get(owner).add(compartment);
  }

  const reactions = [];
  const reactionOwner = new Map();
  const biomassIds = new Map(spec.members.map(member => [member.id, `${member.id}@${member.biomassReaction}`]));
  const exchangeIds = new Set(spec.metadata.speciesExchange.values());
  const projectedReactions = new Map();
  for (const [id, reaction] of spec.reactions) {
    const community = reaction.species === (spec.communityTag ?? 'comm');
    const owner = community ? null : reaction.species;
    const amount = community ? 1 : amounts.get(owner);
    if (!community && amount === undefined) throw new Error(`${id}: no member owns this reaction.`);
    const mergedId = community ? id : `${owner}@${id}`;
    const stoichiometry = {};
    for (const [mid, coefficient] of Object.entries(reaction.metabolites)) {
      const target = metaboliteIds.get(mid);
      if (!target) throw new Error(`${id}: unknown metabolite ${mid}.`);
      const scale = !community && spec.metadata.bases.has(mid) ? amount : 1;
      const projected = coefficient * scale;
      if (projected !== 0) stoichiometry[target] = projected;
    }
    const row = {
      id: mergedId, name: reaction.name ?? id, metabolites: stoichiometry,
      lower_bound: amount === 0 ? 0 : reaction.lowerBound,
      upper_bound: amount === 0 ? 0 : reaction.upperBound,
      subsystem: reaction.subsystem ?? '',
    };
    reactions.push(row);
    projectedReactions.set(mergedId, row);
    reactionOwner.set(mergedId, {
      speciesId: owner, originalId: id,
      role: community ? 'community' : exchangeIds.has(id) ? 'transfer'
        : biomassIds.get(owner) === mergedId ? 'biomass' : 'metabolic',
    });
  }

  const activeSpecies = spec.members.filter(member => amounts.get(member.id) > 0)
    .map(member => ({ ...member, abundance: amounts.get(member.id), enabled: true }));
  const objectiveCoefficients = {};
  if (objective === OBJECTIVES.EQUAL_GROWTH) {
    const growth = { id: 'COMMUNITY_GROWTH', name: 'Community growth rate', metabolites: {}, lower_bound: 0, upper_bound: 0, subsystem: 'Community' };
    let upper = Infinity;
    for (const member of activeSpecies) {
      const id = `GROWTHLINK@${member.id}`;
      const biomass = projectedReactions.get(biomassIds.get(member.id));
      if (!biomass) throw new Error(`${member.id}: the biomass reaction is missing from the joint model.`);
      metabolites.push({ id, name: `Growth link (${member.id})`, compartment: LINK_COMPARTMENT, formula: '', charge: 0 });
      metaboliteOwner.set(id, { speciesId: member.id, originalId: id, compartment: LINK_COMPARTMENT });
      biomass.metabolites[id] = 1;
      growth.metabolites[id] = -1;
      upper = Math.min(upper, biomass.upper_bound);
    }
    growth.upper_bound = Math.max(0, upper);
    reactions.push(growth);
    reactionOwner.set(growth.id, { speciesId: null, originalId: growth.id, role: 'community' });
    objectiveCoefficients[growth.id] = 1;
  } else if (objective === OBJECTIVES.SUM_BIOMASS) {
    for (const member of activeSpecies) objectiveCoefficients[biomassIds.get(member.id)] = amounts.get(member.id);
  } else {
    const shared = spec.metadata.bases.has(productTarget) ? productTarget
      : [...spec.metadata.bases].find(([, base]) => base === productTarget)?.[0];
    const exportId = spec.metadata.communityExport.get(shared);
    const uptakeId = spec.metadata.communityUptake.get(shared);
    if (!exportId || !uptakeId) throw new Error(`No community boundary is available for product ${productTarget ?? '(none)'}.`);
    objectiveCoefficients[exportId] = 1;
    objectiveCoefficients[uptakeId] = -1;
  }

  const transfersByBase = new Map();
  for (const [key, id] of spec.metadata.speciesExchange) {
    const at = key.indexOf('\u0000');
    const met = key.slice(0, at), speciesId = key.slice(at + 1);
    const base = spec.metadata.bases.get(met);
    const local = Object.keys(spec.reactions.get(id).metabolites).find(mid => mid !== met);
    if (!transfersByBase.has(base)) transfersByBase.set(base, []);
    transfersByBase.get(base).push({ speciesId, reactionId: `${speciesId}@${id}`, localId: metaboliteIds.get(local) });
  }
  const communityExchangeIds = new Map([...spec.metadata.bases].map(([met, base]) => [base, {
    uptake: spec.metadata.communityUptake.get(met), export: spec.metadata.communityExport.get(met),
  }]));
  return {
    model: new Model({ id: 'joint-community', name: 'Joint community at fixed composition', compartments, metabolites, reactions, objective: objectiveCoefficients }),
    biomassIds, reactionOwner, metaboliteOwner, transfersByBase, communityExchangeIds,
    activeSpecies, abundances: amounts,
    speciesCompartments: new Map([...speciesCompartments].map(([id, values]) => [id, [...values]])),
  };
}

function projectionCompartment(metabolite) {
  if (metabolite.compartment) return metabolite.compartment;
  return /\[([a-z])(?:_[^\]]+)?\]$/.exec(metabolite.id)?.[1]
    ?? /_([a-z][a-z0-9]?)$/.exec(metabolite.id)?.[1] ?? 'c';
}

function jointBounds(text) {
  const lines = String(text ?? '').trim().split(/\r?\n/);
  const header = csvRow(lines.shift() ?? '');
  const required = ['reaction', 'species', 'lower_bound', 'upper_bound', 'objective_coefficient'];
  const columns = required.map((name) => header.indexOf(name));
  if (columns.some((index) => index < 0)) throw new Error('Joint bounds.csv is missing required columns.');
  const out = new Map();
  for (const line of lines) {
    if (!line.trim()) continue;
    const fields = csvRow(line);
    const [id, species, lowerText, upperText, objectiveText] = columns.map((index) => fields[index]);
    const lower = Number(lowerText), upper = Number(upperText), objective = Number(objectiveText);
    if (!id || !species || [lowerText, upperText, objectiveText].some((value) => value === '' || value === undefined)
      || ![lower, upper, objective].every(Number.isFinite) || lower > upper) {
      throw new Error(`${id || 'Unknown reaction'}: invalid joint bounds row.`);
    }
    if (out.has(id)) throw new Error(`Duplicate joint bounds row ${id}.`);
    out.set(id, { species, lower, upper, objective });
  }
  return out;
}

function csvRow(line) {
  const fields = [];
  let field = '', quoted = false;
  for (let i = 0; i < line.length; i++) {
    const char = line[i];
    if (char === '"') {
      if (quoted && line[i + 1] === '"') { field += '"'; i++; }
      else quoted = !quoted;
    } else if (char === ',' && !quoted) { fields.push(field.trim()); field = ''; }
    else field += char;
  }
  if (quoted) throw new Error('Unclosed quote in joint bounds.csv.');
  fields.push(field.trim());
  return fields;
}

/**
 * Translate SteadyCom's flux keys into the merged model's.
 *
 * SteadyCom names a member's reaction `Ec1|PGI`; the community model the map is
 * drawn from names the same reaction `Ec1@PGI`. Everything the map, the
 * inspector and the flux scale read goes through the second, so a result is only
 * usable once it has been renamed. `owners`, when given, maps an unprefixed key
 * to the member id to prefix it with.
 */
export function toMergedFluxes(fluxes, separator = '@', owners = null) {
  const out = new Map();
  for (const [key, value] of fluxes ?? []) {
    const at = key.indexOf('|');
    const owner = owners?.get(key);
    const id = owner ? `${owner}${separator}${key}`
      : at < 0 ? key : `${key.slice(0, at)}${separator}${key.slice(at + 1)}`;
    out.set(id, value);
  }
  return out;
}
