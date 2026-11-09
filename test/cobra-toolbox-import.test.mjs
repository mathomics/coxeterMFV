import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { prepareModelUploads, parseModelUpload } from '../src/core/model-import.js';

// mini.mat and mini.xlsx hold one five-reaction model in the COBRA Toolbox
// layouts; test/fixtures/cobra-toolbox/make.py writes both.
const fixture = (name) => readFile(new URL(`./fixtures/cobra-toolbox/${name}`, import.meta.url));

function assertMiniModel(model) {
  assert.deepEqual([...model.reactions.keys()], ['EX_glc__D_e', 'GLCt', 'HEX1', 'ATPS', 'BIOMASS']);
  assert.deepEqual([...model.metabolites.keys()], ['glc__D_e', 'glc__D_c', 'g6p_c', 'atp_c', 'adp_c'],
    'the [c] suffixes are written as the Toolbox SBML export writes them');
  assert.deepEqual(model.reactions.get('ATPS').metabolites, { adp_c: -2, atp_c: 2 });
  assert.equal(model.reactions.get('EX_glc__D_e').lowerBound, -10);
  assert.equal(model.reactions.get('GLCt').geneReactionRule, '(g1 and g2) or g3');
  assert.equal(model.reactions.get('HEX1').subsystem, 'Glycolysis');
  assert.deepEqual(model.reactions.get('HEX1').labelSources.ec, ['2.7.1.1']);
  assert.equal(model.biomassReactionId(), 'BIOMASS');
  const g6p = model.metabolites.get('g6p_c');
  assert.deepEqual([g6p.compartment, g6p.base, g6p.name, g6p.formula, g6p.charge], ['c', 'g6p', 'D-Glucose 6-phosphate', 'C6H11O9P', -2]);
  assert.deepEqual(g6p.labelSources.kegg, ['C00092']);
  assert.equal(model.compartmentNames.e, 'Extracellular');
  assert.deepEqual(model.genes.map((g) => g.id), ['g1', 'g2', 'g3', 'g4', 'g5', 'g6']);
}

test('a COBRA Toolbox .mat model imports, and a v7.3 file is refused with a way out', async () => {
  const prepared = await prepareModelUploads([{ name: 'mini.mat', bytes: await fixture('mini.mat') }]);
  assertMiniModel(prepared.entries[0].model);
  assert.equal(prepared.entries[0].model.name, 'Mini COBRA Toolbox model');
  assert.equal(prepared.entries[0].modelKey, 'upload:mini');
  assert.deepEqual(prepared.medium, { glc__D: 10 });

  const hdf5 = new Uint8Array(1024);
  hdf5.set(new TextEncoder().encode('MATLAB 7.3 MAT-file, Platform: GLNXA64, HDF5 schema 1.00'));
  hdf5.set([0x89, 0x48, 0x44, 0x46, 0x0d, 0x0a, 0x1a, 0x0a], 512);
  await assert.rejects(parseModelUpload({ name: 'big.mat', bytes: hdf5 }), /big\.mat: this is a MATLAB v7\.3 \(HDF5\) file.*-v7/);
});

test('a COBRA Toolbox Excel workbook imports with the same model', async () => {
  const model = await parseModelUpload({ name: 'mini.xlsx', bytes: await fixture('mini.xlsx') });
  assertMiniModel(model);
  assert.equal(model.compartmentNames.c, 'Cytosol', 'the Compartment column names the bracketed id');
});
