/** Stoichiometry input preserves model identifiers and rejects sums that cannot represent a finite reaction. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { addMetaboliteToModel, addReactionToModel, parseStoichiometry } from '../src/core/model-edit.js';

test('stoichiometry preserves identifiers that match inherited object properties', async () => {
  const raw = JSON.parse(await readFile(new URL('../models/ecoli_core_mc.json', import.meta.url), 'utf8'));
  const model = new Model(raw);
  for (const id of ['__proto__', 'constructor', 'toString']) {
    addMetaboliteToModel(model, { id, name: `QA ${id}`, compartment: 'c' });
    const text = `${id}:-1, f6p_c:1`;
    const parsed = parseStoichiometry(text);
    assert.equal(Object.hasOwn(parsed, id), true);
    assert.equal(parsed[id], -1);
    const reaction = addReactionToModel(model, {
      id: `QA_${id}`, metabolites: text, lowerBound: 0, upperBound: 10,
    });
    assert.equal(reaction.metabolites[id], -1);
    assert.equal(reaction.metabolites.f6p_c, 1);
    const roundTrip = new Model(JSON.parse(JSON.stringify(model.toJSON())));
    assert.equal(roundTrip.reactions.get(reaction.id).metabolites[id], -1);
    const json = JSON.stringify(Object.fromEntries([[id, -1], ['f6p_c', 1]]));
    assert.deepEqual(parseStoichiometry(json), parsed);
    assert.deepEqual(parseStoichiometry(`${id}:-1, ${id}:1, f6p_c:1`), { f6p_c: 1 });
  }
});

test('repeated finite coefficients cannot overflow to an infinite stoichiometry', () => {
  assert.throws(() => parseStoichiometry('g6p_c:1e308, g6p_c:1e308, f6p_c:-1'), /finite.*g6p_c|g6p_c.*finite/i);
  assert.deepEqual(parseStoichiometry('g6p_c:1e308, g6p_c:-1e308, f6p_c:1'), { f6p_c: 1 });
});
