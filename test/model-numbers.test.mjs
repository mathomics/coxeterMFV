/** Numeric model-edit fields accept scalar numbers without coercing structured data into biology. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { addMetaboliteToModel, addReactionToModel, parseStoichiometry } from '../src/core/model-edit.js';

async function model() {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  return new Model(raw);
}

test('JSON stoichiometry rejects booleans and arrays instead of converting them to coefficients', () => {
  for (const value of [true, false, [], [1], ['-2'], [[1]], {}, null, '']) {
    const json = JSON.stringify({ g6p_c: value, f6p_c: 1 });
    assert.throws(() => parseStoichiometry(json), /coefficient.*g6p_c/i);
  }
});

test('object stoichiometry rejects nonscalar coefficients before changing a model', async () => {
  const source = await model();
  const before = source.toJSON();
  for (const value of [true, false, [], [1], ['-2'], [[1]], {}, null, '']) {
    assert.throws(() => addReactionToModel(source, {
      id: 'QA_NUMBERS', metabolites: { g6p_c: value, f6p_c: 1 }, lowerBound: -10, upperBound: 10,
    }), /coefficient.*g6p_c/i);
    assert.deepEqual(source.toJSON(), before);
  }
});

test('optional charge rejects booleans and arrays before changing a model', async () => {
  const source = await model();
  const before = source.toJSON();
  for (const charge of [true, false, [], [1], ['-2'], [[1]], {}]) {
    assert.throws(() => addMetaboliteToModel(source, {
      id: 'qa_numbers_c', name: 'QA numbers', compartment: 'c', charge,
    }), /charge must be numeric/i);
    assert.deepEqual(source.toJSON(), before);
  }
});

test('numeric strings and numbers retain their signs, fractions and exponents', async () => {
  const source = await model();
  const parsed = parseStoichiometry('{"g6p_c":" -2e-1 ","f6p_c":0.2}');
  assert.deepEqual(parsed, { g6p_c: -0.2, f6p_c: 0.2 });
  const reaction = addReactionToModel(source, {
    id: 'QA_NUMBERS', metabolites: { g6p_c: '-0.2', f6p_c: 0.2 }, lowerBound: '-1e1', upperBound: ' 2.5 ',
  });
  assert.deepEqual(reaction.metabolites, parsed);
  assert.equal(reaction.lowerBound, -10);
  assert.equal(reaction.upperBound, 2.5);
  const metabolite = addMetaboliteToModel(source, {
    id: 'qa_numbers_c', name: 'QA numbers', compartment: 'c', charge: '-2',
  });
  assert.equal(metabolite.charge, -2);
});

test('omitted and blank optional charges remain zero', async () => {
  const source = await model();
  for (const [index, charge] of [undefined, null, '', '  ', 0, '0'].entries()) {
    const metabolite = addMetaboliteToModel(source, {
      id: `qa_blank_${index}_c`, name: 'QA blank charge', compartment: 'c', charge,
    });
    assert.equal(metabolite.charge, 0);
  }
});
