/**
 * Multi-species community model.
 *
 * Every species keeps its own namespace and its own compartments. A single
 * shared medium (compartment `s`) sits between the cells and carries every
 * metabolite that leaves or enters a cell. The result is an ordinary COBRA
 * `Model`, so the FBA layer does not need to know that a community is involved.
 *
 * Layout of the merged problem:
 *   sp@rxn            internal reaction of one species (per gDW of that species)
 *   sp@EX_x_e         the species' exchange, as a transfer  x_e(sp) ⇌ a_sp · x(medium)
 *   EX_x_s            community boundary: supply to / washout from the medium
 *   COMMUNITY_GROWTH  only for the equal-growth objective
 *
 * The abundance a_sp (gDW of that species per gDW of community) lives on the
 * medium side of each transfer, which converts a species-specific flux into a
 * community-basis flux. This is the fixed-abundance form of SteadyCom: growth
 * rates are shared, abundances are an input rather than a variable.
 */

import { Model, stripCompartment } from './model.js';

export const SHARED_COMPARTMENT = 's';
export const LINK_COMPARTMENT = 'link';

export const OBJECTIVES = {
  EQUAL_GROWTH: 'equal-growth',
  SUM_BIOMASS: 'sum-biomass',
  PRODUCT: 'product',
};

const SEP = '@';

// Ids in the merged model: a species' own id, a medium metabolite, and the
// community exchange of a medium metabolite.
export const namespaced = (speciesId, id) => `${speciesId}${SEP}${id}`;
export const sharedMetabolite = (base) => `M${SEP}${base}`;
export const communityExchange = (base) => `EX_${base}_s`;

/** Split a namespaced id back into its parts. */
export function splitId(id) {
  const i = id.indexOf(SEP);
  if (i < 0) return { species: null, id };
  return { species: id.slice(0, i), id: id.slice(i + 1) };
}

/**
 * @typedef {object} SpeciesEntry
 * @property {string} id            short id used as namespace, e.g. "eco"
 * @property {Model} model
 * @property {number} abundance     gDW species / gDW community
 * @property {boolean} enabled
 * @property {string} [color]
 */

export class CommunityModel {
  /**
   * @param {object} spec
   * @param {SpeciesEntry[]} spec.species
   * @param {Object<string,{lower?:number, upper?:number}>} [spec.medium] keyed by base metabolite id
   * @param {string} [spec.objective]
   * @param {string} [spec.productTarget] base metabolite id, for the `product` objective
   * @param {string} [spec.name]
   */
  constructor(spec) {
    this.name = spec.name || 'Community';
    this.objectiveType = spec.objective || OBJECTIVES.EQUAL_GROWTH;
    this.productTarget = spec.productTarget || null;
    this.medium = { ...(spec.medium || {}) };
    this.species = spec.species.map((s, i) => ({
      id: s.id || `sp${i + 1}`,
      model: s.model,
      abundance: Number.isFinite(s.abundance) ? s.abundance : 1 / spec.species.length,
      enabled: s.enabled !== false,
      color: s.color || s.model?.cell?.color || '#334155',
    }));

    this.build();
  }

  get activeSpecies() { return this.species.filter((s) => s.enabled); }

  /** Normalised abundances across the enabled species (they must sum to 1). */
  normalisedAbundances() {
    const active = this.activeSpecies;
    const total = active.reduce((sum, s) => sum + Math.max(s.abundance, 0), 0);
    const map = new Map();
    for (const s of active) {
      map.set(s.id, total > 0 ? Math.max(s.abundance, 0) / total : 1 / active.length);
    }
    return map;
  }

  /** Rebuild the merged model. Call after changing species, abundances or objective. */
  build() {
    const abundances = this.normalisedAbundances();
    const metabolites = [];
    const reactions = [];
    const compartments = {};

    /** base metabolite id -> { base, name, formula, charge, producers:Set, consumers:Set } */
    const mediumPool = new Map();

    this.reactionOwner = new Map();   // merged rxn id -> { speciesId, originalId, role }
    this.metaboliteOwner = new Map(); // merged met id -> { speciesId, originalId, compartment }
    this.transfersByBase = new Map(); // base -> [{ speciesId, reactionId, direction }]
    this.speciesCompartments = new Map();

    for (const sp of this.activeSpecies) {
      const a = abundances.get(sp.id);
      const model = sp.model;
      this.speciesCompartments.set(sp.id, model.compartmentIds());

      for (const [id, name] of Object.entries(model.compartmentNames)) {
        compartments[`${sp.id}${SEP}${id}`] = name;
      }

      for (const met of model.metabolites.values()) {
        const mergedId = namespaced(sp.id, met.id);
        metabolites.push({
          id: mergedId,
          // The species' own name for it. Left to be derived from the merged id
          // it comes out as `eco@g6p`, and that is what the map then prints as
          // the label of the node.
          base: met.base,
          name: met.name,
          labelSources: met.labelSources,
          annotation: met.annotation,
          notes: met.notes,
          compartment: met.compartment,
          formula: met.formula,
          charge: met.charge,
          primary: met.explicitPrimary === null ? undefined : met.explicitPrimary,
        });
        this.metaboliteOwner.set(mergedId, {
          speciesId: sp.id, originalId: met.id, compartment: met.compartment,
        });
      }

      for (const rxn of model.reactions.values()) {
        const mergedId = namespaced(sp.id, rxn.id);
        const stoich = {};
        for (const [mid, coef] of Object.entries(rxn.metabolites)) {
          stoich[namespaced(sp.id, mid)] = coef;
        }

        let role = rxn.kind;
        if (rxn.kind === 'exchange') {
          // Re-point the exchange at the shared medium: it stops being a boundary
          // and becomes a real transfer between this cell and the environment.
          const entries = Object.entries(rxn.metabolites);
          if (entries.length === 1) {
            const [localId, coef] = entries[0];
            const met = model.metabolites.get(localId);
            const base = met.base;
            const shared = sharedMetabolite(base);
            stoich[shared] = (stoich[shared] || 0) - coef * a;
            role = 'transfer';

            if (!mediumPool.has(base)) {
              mediumPool.set(base, {
                base, name: met.name.replace(/\s*\[[^\]]*\]\s*$/, ''),
                formula: met.formula, charge: met.charge,
                labelSources: met.labelSources,
              });
            }
            if (!this.transfersByBase.has(base)) this.transfersByBase.set(base, []);
            this.transfersByBase.get(base).push({ speciesId: sp.id, reactionId: mergedId, localId: namespaced(sp.id, localId) });
          }
        }

        reactions.push({
          id: mergedId,
          name: rxn.name,
          labelSources: rxn.labelSources,
          metabolites: stoich,
          lower_bound: rxn.lowerBound,
          upper_bound: rxn.upperBound,
          subsystem: rxn.subsystem,
          pathway: rxn.pathway,
          gene_reaction_rule: rxn.geneReactionRule,
          layout: rxn.layout,
          notes: rxn.notes,
          annotation: rxn.annotation,
        });
        this.reactionOwner.set(mergedId, { speciesId: sp.id, originalId: rxn.id, role });
      }
    }

    // Shared medium metabolites and community boundary exchanges
    compartments[SHARED_COMPARTMENT] = 'Medio compartido';
    for (const info of mediumPool.values()) {
      const id = sharedMetabolite(info.base);
      metabolites.push({
        id,
        base: info.base,
        name: info.name,
        labelSources: info.labelSources,
        compartment: SHARED_COMPARTMENT,
        formula: info.formula,
        charge: info.charge,
      });
      this.metaboliteOwner.set(id, { speciesId: null, originalId: info.base, compartment: SHARED_COMPARTMENT });

      const exId = communityExchange(info.base);
      const limits = this.medium[info.base] || {};
      reactions.push({
        id: exId,
        name: `${info.name} exchange (medium)`,
        metabolites: { [id]: -1 },
        lower_bound: Number.isFinite(limits.lower) ? limits.lower : 0,
        upper_bound: Number.isFinite(limits.upper) ? limits.upper : 1000,
        subsystem: 'Community exchange',
      });
      this.reactionOwner.set(exId, { speciesId: null, originalId: exId, role: 'exchange' });
    }

    // Objective wiring
    const objective = {};
    this.biomassIds = new Map();
    for (const sp of this.activeSpecies) {
      const bio = sp.model.biomassReactionId();
      if (bio) this.biomassIds.set(sp.id, namespaced(sp.id, bio));
    }

    if (this.objectiveType === OBJECTIVES.EQUAL_GROWTH && this.biomassIds.size > 0) {
      const link = { id: 'COMMUNITY_GROWTH', name: 'Community growth rate', metabolites: {}, lower_bound: 0, upper_bound: 1000, subsystem: 'Community' };
      for (const [spId, rxnId] of this.biomassIds) {
        const linkMet = `GROWTHLINK${SEP}${spId}`;
        metabolites.push({ id: linkMet, name: `Growth link (${spId})`, compartment: LINK_COMPARTMENT, formula: '', charge: 0 });
        this.metaboliteOwner.set(linkMet, { speciesId: spId, originalId: linkMet, compartment: LINK_COMPARTMENT });
        const target = reactions.find((r) => r.id === rxnId);
        if (target) target.metabolites[linkMet] = 1;
        link.metabolites[linkMet] = -1;
      }
      reactions.push(link);
      this.reactionOwner.set('COMMUNITY_GROWTH', { speciesId: null, originalId: 'COMMUNITY_GROWTH', role: 'community' });
      objective.COMMUNITY_GROWTH = 1;
    } else if (this.objectiveType === OBJECTIVES.PRODUCT && this.productTarget) {
      const exId = communityExchange(this.productTarget);
      if (reactions.some((r) => r.id === exId)) objective[exId] = 1;
    } else {
      for (const [spId, rxnId] of this.biomassIds) {
        objective[rxnId] = abundances.get(spId) ?? 1;
      }
    }

    compartments[LINK_COMPARTMENT] = 'Acoplamiento (virtual)';

    this.merged = new Model({
      id: 'community',
      name: this.name,
      compartments,
      metabolites,
      reactions,
      objective,
    });
    this.abundances = abundances;
    return this.merged;
  }

  /** Reactions that move a metabolite between a cell and the shared medium. */
  transferReactions() {
    const out = [];
    for (const [base, list] of this.transfersByBase) {
      for (const t of list) out.push({ base, ...t });
    }
    return out;
  }

  /** Growth rate of each species from a solved flux map. */
  growthRates(fluxes) {
    const out = new Map();
    for (const [spId, rxnId] of this.biomassIds) {
      out.set(spId, fluxes.get(rxnId) ?? 0);
    }
    return out;
  }

  /**
   * Who feeds whom. For every medium metabolite, split the transfer fluxes into
   * secretion (positive, community basis) and uptake (negative), then pair them
   * up proportionally so the map can draw explicit cross-feeding arrows.
   *
   * @param {Map<string, number>} fluxes
   * @param {number} [threshold] ignore exchanges smaller than this
   * @returns {{links: object[], perMetabolite: object[]}} links {base, from, to, flux},
   *   largest flux first. A null `from` or `to` stands for the medium.
   */
  crossFeeding(fluxes, threshold = 1e-6) {
    const links = [];
    const perMetabolite = [];

    for (const [base, transfers] of this.transfersByBase) {
      const producers = [];
      const consumers = [];
      for (const t of transfers) {
        const v = fluxes.get(t.reactionId) ?? 0;
        const a = this.abundances.get(t.speciesId) ?? 1;
        const communityFlux = v * a; // mmol per gDW community per h
        if (communityFlux > threshold) producers.push({ speciesId: t.speciesId, flux: communityFlux });
        else if (communityFlux < -threshold) consumers.push({ speciesId: t.speciesId, flux: -communityFlux });
      }

      const exId = communityExchange(base);
      const envFlux = fluxes.get(exId) ?? 0;
      // Negative community exchange = supplied by the medium (an external source).
      if (envFlux < -threshold) producers.push({ speciesId: null, flux: -envFlux });
      else if (envFlux > threshold) consumers.push({ speciesId: null, flux: envFlux });

      if (!producers.length || !consumers.length) {
        if (producers.length || consumers.length) {
          perMetabolite.push({ base, producers, consumers, total: sum(producers) || sum(consumers) });
        }
        continue;
      }

      const totalOut = sum(producers);
      const totalIn = sum(consumers);
      const total = Math.min(totalOut, totalIn);
      for (const p of producers) {
        for (const q of consumers) {
          const share = (p.flux / totalOut) * (q.flux / totalIn) * total;
          if (share > threshold) {
            links.push({ base, from: p.speciesId, to: q.speciesId, flux: share });
          }
        }
      }
      perMetabolite.push({ base, producers, consumers, total });
    }

    links.sort((a, b) => b.flux - a.flux);
    return { links, perMetabolite };
  }

  /** Set the supply limit of a medium component (positive = mmol/gDW/h available). */
  setMediumSupply(base, rate) {
    this.medium[base] = { ...(this.medium[base] || {}), lower: -Math.abs(rate) };
    return this;
  }

  toJSON() {
    return {
      name: this.name,
      objective: this.objectiveType,
      productTarget: this.productTarget,
      medium: this.medium,
      species: this.species.map((s) => ({
        id: s.id, abundance: s.abundance, enabled: s.enabled,
        model: s.model.id, color: s.color,
      })),
    };
  }
}

function sum(list) { return list.reduce((s, x) => s + x.flux, 0); }

/** Base metabolite id of a merged metabolite, regardless of namespace. */
export function baseOf(mergedId, compartment) {
  const { id } = splitId(mergedId.startsWith(`M${SEP}`) ? mergedId.slice(2) : mergedId);
  return stripCompartment(id, compartment);
}
