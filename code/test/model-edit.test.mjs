import test from 'node:test';
import assert from 'node:assert/strict';
import { Model } from '../src/core/model.js';
import { Store } from '../src/ui/store.js';
import {
  addMetaboliteToModel,
  addReactionToModel,
  geneIdsFromGPR,
  parseStoichiometry,
} from '../src/core/model-edit.js';

function model() {
  return new Model({
    id: 'demo',
    name: 'Demo',
    compartments: { c: 'cytosol', e: 'extracellular' },
    metabolites: [
      { id: 'a_c', name: 'A', compartment: 'c' },
      { id: 'b_c', name: 'B', compartment: 'c' },
    ],
    reactions: [],
    genes: [],
  });
}

test('stoichiometry accepts comma/line syntax and JSON objects', () => {
  assert.deepEqual(parseStoichiometry('a_c:-2, b_c:1'), { a_c: -2, b_c: 1 });
  assert.deepEqual(parseStoichiometry('a_c -1\nb_c 1'), { a_c: -1, b_c: 1 });
  assert.deepEqual(parseStoichiometry('{"a_c":-1,"b_c":1}'), { a_c: -1, b_c: 1 });
});

test('new metabolite is added to the biological Model and exported', () => {
  const m = model();
  addMetaboliteToModel(m, { id: 'c_c', name: 'C', compartment: 'c' });
  assert.ok(m.metabolites.has('c_c'));
  assert.ok(m.toJSON().metabolites.some((x) => x.id === 'c_c'));
});

test('new reaction validates stoichiometry and exports bounds and GPR', () => {
  const m = model();
  addReactionToModel(m, {
    id: 'R_NEW',
    name: 'New reaction',
    metabolites: 'a_c:-1, b_c:1',
    lowerBound: -10,
    upperBound: 25,
    geneReactionRule: 'geneA and (geneB or geneC)',
  });

  const r = m.reactions.get('R_NEW');
  assert.deepEqual(r.metabolites, { a_c: -1, b_c: 1 });
  assert.equal(r.lowerBound, -10);
  assert.equal(r.upperBound, 25);
  assert.equal(r.geneReactionRule, 'geneA and (geneB or geneC)');

  const exported = m.toJSON().reactions.find((x) => x.id === 'R_NEW');
  assert.equal(exported.lower_bound, -10);
  assert.equal(exported.upper_bound, 25);
  assert.equal(exported.gene_reaction_rule, 'geneA and (geneB or geneC)');
});

test('genes mentioned in a new GPR are added to model genes', () => {
  const m = model();
  addReactionToModel(m, {
    id: 'R_GPR',
    metabolites: { a_c: -1, b_c: 1 },
    lowerBound: 0,
    upperBound: 1000,
    gpr: 'g1 and (g2 or g3)',
  });
  assert.deepEqual(geneIdsFromGPR('g1 and (g2 or g3)'), ['g1', 'g2', 'g3']);
  const ids = m.genes.map((g) => typeof g === 'string' ? g : g.id);
  assert.ok(ids.includes('g1') && ids.includes('g2') && ids.includes('g3'));
});

test('duplicate IDs, bad compartments, unknown metabolites and bad bounds are rejected', () => {
  const m = model();
  assert.throws(() => addMetaboliteToModel(m, { id: 'a_c', compartment: 'c' }), /already exists/);
  assert.throws(() => addMetaboliteToModel(m, { id: 'x_z', name: 'X', compartment: 'z' }), /Invalid compartment/);
  assert.throws(() => addMetaboliteToModel(m, { id: 'x_c', name: '', compartment: 'c' }), /metabolite name is required/);
  assert.throws(() => addReactionToModel(m, {
    id: 'R_BAD', metabolites: 'missing_c:-1', lowerBound: 0, upperBound: 1000,
  }), /does not exist/);
  assert.throws(() => addReactionToModel(m, {
    id: 'R_BAD2', metabolites: 'a_c:-1,b_c:1', lowerBound: 10, upperBound: 0,
  }), /lower bound/);
});


test('Store model export contains user-created metabolites and reactions', () => {
  const m = model();
  addMetaboliteToModel(m, { id: 'c_c', name: 'C', compartment: 'c' });
  addReactionToModel(m, {
    id: 'R_EXPORT',
    name: 'Exported edit',
    metabolites: 'a_c:-1,c_c:1',
    lowerBound: -5,
    upperBound: 10,
    gpr: 'gx1 or gx2',
  });

  const store = new Store('.');
  store.models.set('demo-key', m);
  store.species = [{ id: 'sp1', file: 'demo-key', abundance: 1, enabled: true }];
  store.modelEditorSpeciesId = 'sp1';

  const doc = JSON.parse(store.modelJSON());
  assert.ok(doc.metabolites.some((x) => x.id === 'c_c'));
  const reaction = doc.reactions.find((x) => x.id === 'R_EXPORT');
  assert.ok(reaction);
  assert.deepEqual(reaction.metabolites, { a_c: -1, c_c: 1 });
  assert.equal(reaction.lower_bound, -5);
  assert.equal(reaction.upper_bound, 10);
  assert.equal(reaction.gene_reaction_rule, 'gx1 or gx2');
});
