import test from 'node:test';
import assert from 'node:assert/strict';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import {
  compartmentLayoutProfile,
  isYeast9CompartmentSet,
} from '../src/core/compartments.js';
import { layoutCommunity } from '../src/layout/layout.js';

const compartments = {
  c: 'cytoplasm',
  ce: 'cell envelope',
  e: 'extracellular',
  er: 'endoplasmic reticulum',
  erm: 'endoplasmic reticulum membrane',
  g: 'Golgi',
  gm: 'Golgi membrane',
  lp: 'lipid particle',
  m: 'mitochondrion',
  mm: 'mitochondrial membrane',
  n: 'nucleus',
  p: 'peroxisome',
  v: 'vacuole',
  vm: 'vacuolar membrane',
};

function yeast9MiniModel() {
  return new Model({
    id: 'yeast9-mini',
    name: 'Yeast9 topology fixture',
    compartments,
    cell: {
      name: 'S. cerevisiae',
      organism: 'Saccharomyces cerevisiae',
      shape: 'round',
    },
    metabolites: Object.keys(compartments).map((id) => ({
      id: `marker_${id}`,
      name: `marker ${id}`,
      compartment: id,
      formula: '',
      charge: 0,
      primary: true,
    })),
    reactions: [],
  });
}

test('Yeast9 is detected as a dedicated compartment layout profile', () => {
  const ids = Object.keys(compartments);
  assert.equal(isYeast9CompartmentSet(ids, compartments), true);
  assert.equal(compartmentLayoutProfile(ids, compartments), 'yeast9');
});

test('Yeast9 uses its biological 14-compartment containment tree', () => {
  const model = yeast9MiniModel();
  const { byId } = model.compartmentTree();

  assert.equal(byId.size, 14);
  const expectedParents = {
    e: null,
    ce: 'e',
    c: 'ce',
    erm: 'c',
    er: 'erm',
    gm: 'c',
    g: 'gm',
    lp: 'c',
    mm: 'c',
    m: 'mm',
    n: 'c',
    p: 'c',
    vm: 'c',
    v: 'vm',
  };

  for (const [id, parent] of Object.entries(expectedParents)) {
    assert.equal(byId.get(id)?.parent ?? null, parent, `${id} should be inside ${parent ?? 'the cell'}`);
  }

  // The easiest mistake in a yeast layout: in Yeast9 p means peroxisome, not
  // bacterial periplasm.
  assert.equal(byId.get('p').parent, 'c');
});

test('Yeast9 cytosolic organelles are rendered as separate non-overlapping islands', () => {
  const model = yeast9MiniModel();
  const community = new CommunityModel({
    species: [{ id: 'sce', model, abundance: 1, enabled: true }],
    medium: {},
  });
  const layout = layoutCommunity(community, { contents: 'none' });
  const cell = layout.cells[0];

  assert.equal(cell.compartments.length, 14);
  const byId = new Map(cell.compartments.map((compartment) => [compartment.id, compartment]));
  const cytosol = byId.get('c').rect;
  const organelles = ['erm', 'gm', 'lp', 'mm', 'n', 'p', 'vm'].map((id) => byId.get(id));

  for (const compartment of organelles) {
    assert.ok(compartment, 'expected organelle compartment');
    const r = compartment.rect;
    assert.ok(r.x >= cytosol.x && r.y >= cytosol.y, `${compartment.id} starts outside cytosol`);
    assert.ok(r.x + r.w <= cytosol.x + cytosol.w + 1e-6, `${compartment.id} exceeds cytosol width`);
    assert.ok(r.y + r.h <= cytosol.y + cytosol.h + 1e-6, `${compartment.id} exceeds cytosol height`);
  }

  for (let i = 0; i < organelles.length; i++) {
    for (let j = i + 1; j < organelles.length; j++) {
      const a = organelles[i].rect;
      const b = organelles[j].rect;
      const overlap = a.x < b.x + b.w && b.x < a.x + a.w
        && a.y < b.y + b.h && b.y < a.y + a.h;
      assert.equal(overlap, false, `${organelles[i].id} overlaps ${organelles[j].id}`);
    }
  }
});
