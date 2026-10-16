import test from 'node:test';
import assert from 'node:assert/strict';
import {
  compareReactionScores, divergingOmicsColors, evaluateGPR, parseGPR,
  parseOmicsCSV, reactionOmicsScores, sequentialOmicsColors,
} from '../src/omics/overlay.js';
import { Model } from '../src/core/model.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';

test('omics CSV accepts wide one- or multi-condition tables and optional species', () => {
  const data = parseOmicsCSV(
    'species,gene,Control,Treatment\nA,g1,10,20\nA,g2,4,2\nB,g1,8,16\n',
    { dataType: 'gene' },
  );
  assert.deepEqual(data.conditions, ['Control', 'Treatment']);
  assert.equal(data.rowCount, 3);
  assert.equal(data.values.get('A\u0000g1').values.Treatment, 20);
});

test('GPR numeric evaluation uses AND=min and OR=max', () => {
  const ast = parseGPR('(g1 and g2) or g3');
  const values = new Map([['g1', 8], ['g2', 3], ['g3', 5]]);
  assert.equal(evaluateGPR(ast, (id) => values.get(id)), 5);

  const missingAnd = parseGPR('g1 and missing');
  assert.equal(evaluateGPR(missingAnd, (id) => values.get(id)), null);

  const availableOr = parseGPR('g1 or missing');
  assert.equal(evaluateGPR(availableOr, (id) => values.get(id)), 8);
});

test('gene/protein identifiers map through local reaction GPRs with species disambiguation', () => {
  const model = new Model({
    id: 'm',
    compartments: { c: 'cytosol' },
    metabolites: [
      { id: 'a_c', compartment: 'c' },
      { id: 'b_c', compartment: 'c' },
    ],
    reactions: [
      { id: 'R1', metabolites: { a_c: -1, b_c: 1 }, gene_reaction_rule: 'g1 and g2' },
      { id: 'R2', metabolites: { b_c: -1, a_c: 1 }, gene_reaction_rule: 'g3 or g4' },
    ],
  });
  const community = {
    merged: { reactions: new Map([['sp@R1', {}], ['sp@R2', {}]]) },
    reactionOwner: new Map([
      ['sp@R1', { speciesId: 'sp', originalId: 'R1' }],
      ['sp@R2', { speciesId: 'sp', originalId: 'R2' }],
    ]),
  };
  const species = [{ id: 'sp', file: 'model' }];
  const models = new Map([['model', model]]);
  const data = parseOmicsCSV('species,protein,C1\nsp,g1,10\nsp,g2,4\nsp,g3,2\nsp,g4,7\n', {
    dataType: 'protein',
  });

  const scores = reactionOmicsScores(community, species, models, data, 'C1');
  assert.equal(scores.get('sp@R1'), 4);
  assert.equal(scores.get('sp@R2'), 7);
});

test('two-condition comparison supports log2 fold change and direct difference', () => {
  const a = new Map([['R1', 3], ['R2', 7]]);
  const b = new Map([['R1', 7], ['R2', 3]]);

  const fc = compareReactionScores(a, b, { metric: 'log2fc', pseudocount: 1 });
  assert.equal(Number(fc.get('R1').toFixed(6)), 1);
  assert.equal(Number(fc.get('R2').toFixed(6)), -1);

  const delta = compareReactionScores(a, b, { metric: 'difference' });
  assert.equal(delta.get('R1'), 4);
  assert.equal(delta.get('R2'), -4);
});

test('omics colour scales are sequential for one condition and diverging for comparisons', () => {
  const one = sequentialOmicsColors(new Map([['R1', 1], ['R2', 10]]));
  assert.notEqual(one.colors.get('R1'), one.colors.get('R2'));
  assert.equal(one.kind, 'sequential');

  const compare = divergingOmicsColors(new Map([['R1', -2], ['R2', 0], ['R3', 2]]));
  assert.equal(compare.kind, 'diverging');
  assert.notEqual(compare.colors.get('R1'), compare.colors.get('R3'));
});

test('renderer lets omics colours override reaction colour while keeping fixed Escher width', () => {
  const layout = {
    width: 300,
    height: 200,
    environment: { hidden: true },
    cells: [],
    reactions: new Map([['R1', { id: 'R1', x: 150, y: 100, onMembrane: false, compartments: [] }]]),
    nodes: new Map(),
    segments: [{
      reactionId: 'R1',
      kind: 'axis',
      side: 'product',
      from: { x: 100, y: 100 },
      to: { x: 200, y: 100 },
    }],
    texts: new Map(),
  };
  const fluxes = new Map([['R1', 5]]);
  const scale = new FluxScale(fluxes.values());
  const svg = renderMap(layout, {
    fluxes,
    scale,
    omicsEnabled: true,
    omicsColors: new Map([['R1', '#abcdef']]),
    omicsFallbackColor: '#cccccc',
    showCrossFeeding: false,
  });
  assert.ok(svg.includes('stroke="#abcdef"'));
  assert.ok(svg.includes('stroke-width="7.5"'));
});
