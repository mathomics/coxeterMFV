/** Adapt the bundled E. coli network for private-case loader tests, without using the published models. */
import { Model } from '../../src/core/model.js';
import { EditableMap } from '../../src/map/editable-map.js';
import { ZUNIGA_ID, createZunigaScenario } from '../../src/communities/zuniga.js';

export function zunigaCoreFixture(raw) {
  const mono = structuredClone(raw);
  mono.id = 'core-private-fixture-Ec';
  mono.genes = [{ id: 'b2599' }];
  for (const reaction of mono.reactions) {
    reaction.gene_reaction_rule = '';
    reaction.objective_coefficient = 0;
    if (reaction.id === 'PGI') {
      reaction.id = 'PPNDH';
      reaction.gene_reaction_rule = 'b2599';
    }
    if (reaction.id === 'BIOMASS_Ec_core') {
      reaction.id = 'BIOMASS_Ec_iJO1366_core_53p95M';
      reaction.objective_coefficient = 1;
    }
  }
  mono.objective = { BIOMASS_Ec_iJO1366_core_53p95M: 1 };
  const se = structuredClone(mono);
  se.id = 'core-private-fixture-Se';
  se.genes = [];
  for (const reaction of se.reactions) {
    reaction.gene_reaction_rule = '';
    if (reaction.id === 'BIOMASS_Ec_iJO1366_core_53p95M') {
      reaction.id = 'BOF';
    }
  }
  se.objective = { BOF: 1 };
  const joint = { id: 'core-private-fixture-joint', compartments: mono.compartments, metabolites: [], reactions: [], genes: mono.genes, objective: {} };
  for (const [model, suffix] of [[mono, 'D'], [se, 'A']]) {
    for (const metabolite of model.metabolites) {
      joint.metabolites.push({ ...structuredClone(metabolite), id: `${metabolite.id}_${suffix}` });
    }
    for (const reaction of model.reactions) {
      const next = structuredClone(reaction);
      next.id = `${reaction.id}_${suffix}`;
      next.metabolites = {};
      for (const [id, coefficient] of Object.entries(reaction.metabolites)) {
        next.metabolites[`${id}_${suffix}`] = coefficient;
      }
      joint.reactions.push(next);
    }
    for (const [id, coefficient] of Object.entries(model.objective)) {
      joint.objective[`${id}_${suffix}`] = coefficient;
    }
  }
  const study = {
    manifest: {
      id: ZUNIGA_ID, community: ZUNIGA_ID, runtime: 'zuniga-mat', map: 'map.json', fluxReference: 0.1,
      members: [{ id: 'Ec' }, { id: 'Se' }],
      sources: [{ file: 'core-fixture', sha256: 'bundled-core-fixture' }],
    },
    sourceHashes: { 'core-fixture': 'bundled-core-fixture' },
    sourceModels: { mono, joint },
    models: { Ec: new Model(mono), Se: new Model(se) },
  };
  const community = createZunigaScenario(study, 'mutant-coculture').community;
  const map = EditableMap.emptyFromCommunity(community, { width: 1800, height: 1200 });
  map.addReaction(community, 'Ec@PPNDH');
  map.addReaction(community, 'Se@BOF');
  return { study, map };
}
