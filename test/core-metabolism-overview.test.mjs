import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel, namespaced } from '../src/core/community.js';
import {
  CORE_OVERVIEW_GROUPS,
  buildCoreMetabolismOverview,
  coreOverviewSupport,
} from '../src/map/core-metabolism-overview.js';
import { layoutPreset } from '../src/layout/layout-settings.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function steadyComCommunity() {
  const species = [];
  for (const id of ['Ec1','Ec2','Ec3','Ec4']) {
    const raw = JSON.parse(await readFile(
      join(root, 'models', 'published', 'steadycom-ecoli-auxotrophs', `preview-${id}.json`),
      'utf8',
    ));
    species.push({ id, model: new Model(raw), abundance: 0.25, enabled: true });
  }
  return new CommunityModel({
    name: 'SteadyCom glycolysis fixture',
    objective: 'equal-growth',
    species,
    medium: {},
  });
}

test('core overview contains glycolysis only', async () => {
  assert.equal(CORE_OVERVIEW_GROUPS.length, 1);
  assert.equal(CORE_OVERVIEW_GROUPS[0].id, 'glycolysis');
  assert.deepEqual(
    [...CORE_OVERVIEW_GROUPS[0].reactions],
    ['PGI','PFK','FBA','TPI','GAPD','PGK','PGM','ENO','PYK'],
  );

  const community = await steadyComCommunity();
  const support = coreOverviewSupport(community);
  assert.equal(support.supported, true);
  assert.equal(support.coverage, 1);
});

test('glycolysis is the only reaction set drawn in every cell', async () => {
  const community = await steadyComCommunity();
  const built = buildCoreMetabolismOverview(community, layoutPreset('spacious'));

  assert.ok(built?.map);
  assert.equal(built.perCell.length, 4);
  assert.equal(built.pathwayGroups.length, 4);
  assert.equal(built.placed, 36);

  const allowed = new Set(CORE_OVERVIEW_GROUPS[0].reactions);
  for (const cell of built.map.cells) {
    const reactions = [...built.map.reactions.values()]
      .filter((rxn) => rxn.cellId === cell.id);
    assert.equal(reactions.length, 9, `${cell.id} should show 9 glycolysis reactions`);

    for (const rxn of reactions) {
      const localId = rxn.id.slice(cell.id.length + 1);
      assert.ok(allowed.has(localId), `unexpected reaction ${rxn.id}`);
    }
  }
});

test('glycolysis metabolite landmarks run vertically from G6P to pyruvate', async () => {
  const community = await steadyComCommunity();
  const built = buildCoreMetabolismOverview(community, layoutPreset('spacious'));

  const order = ['g6p_c','f6p_c','fdp_c','g3p_c','13dpg_c','3pg_c','2pg_c','pep_c','pyr_c'];

  for (const cellId of ['Ec1','Ec2','Ec3','Ec4']) {
    const cell = built.map.cells.find((item) => item.id === cellId);
    const cytosol = cell.compartments.find((comp) => comp.id === 'c').rect;

    const points = order.map((localId) => {
      const node = built.map.nodesForMetabolite(namespaced(cellId, localId))[0];
      assert.ok(node, `${cellId} missing ${localId}`);
      return {
        x: (node.x - cytosol.x) / cytosol.w,
        y: (node.y - cytosol.y) / cytosol.h,
      };
    });

    for (let i = 1; i < points.length; i += 1) {
      assert.ok(points[i].y > points[i - 1].y,
        `${cellId} glycolysis should progress downward at step ${i}`);
    }

    for (const point of points) {
      assert.ok(Math.abs(point.x - 0.50) < 0.001,
        `${cellId} main glycolysis backbone should be centered vertically`);
    }
  }
});

test('DHAP branches left of the vertical backbone and rejoins through TPI', async () => {
  const community = await steadyComCommunity();
  const built = buildCoreMetabolismOverview(community, layoutPreset('spacious'));

  const cell = built.map.cells.find((item) => item.id === 'Ec1');
  const cytosol = cell.compartments.find((comp) => comp.id === 'c').rect;
  const dhap = built.map.nodesForMetabolite(namespaced('Ec1','dhap_c'))[0];
  const g3p = built.map.nodesForMetabolite(namespaced('Ec1','g3p_c'))[0];

  assert.ok(dhap && g3p);
  assert.ok(dhap.x < g3p.x - cytosol.w * 0.08, 'DHAP should branch clearly to the left');
  assert.ok(Math.abs(dhap.y - g3p.y) < cytosol.h * 0.08, 'DHAP/G3P should occupy the same branch level');

  const tpi = built.map.reactions.get(namespaced('Ec1','TPI'));
  assert.ok(tpi?.axis, 'TPI should have an axis');
  assert.ok(Math.abs(tpi.axis.dir.x) > Math.abs(tpi.axis.dir.y),
    'TPI should be predominantly horizontal from DHAP toward G3P');
});

test('linear glycolysis reactions point predominantly downward', async () => {
  const community = await steadyComCommunity();
  const built = buildCoreMetabolismOverview(community, layoutPreset('spacious'));

  for (const localId of ['PGI','PFK','GAPD','PGK','PGM','ENO','PYK']) {
    const reaction = built.map.reactions.get(namespaced('Ec1', localId));
    assert.ok(reaction?.axis, `missing axis for ${localId}`);
    assert.ok(Math.abs(reaction.axis.dir.y) > Math.abs(reaction.axis.dir.x),
      `${localId} should be predominantly vertical`);
    assert.ok(reaction.axis.dir.y > 0, `${localId} should point downward`);
  }
});

test('overview labels contain glycolysis metabolites only', async () => {
  const community = await steadyComCommunity();
  const built = buildCoreMetabolismOverview(community, layoutPreset('spacious'));
  const texts = [...built.map.texts.values()].map((item) => item.text);

  assert.ok(texts.includes('GLYCOLYSIS'));
  for (const label of ['G6P','F6P','FBP','DHAP','G3P','1,3-BPG','3PG','2PG','PEP','PYR']) {
    assert.ok(texts.includes(label), `missing label ${label}`);
  }

  for (const forbidden of ['TCA CYCLE','PENTOSE PHOSPHATE PATHWAY','SER / GLY FAMILY ← 3PG']) {
    assert.ok(!texts.includes(forbidden), `forbidden overview label ${forbidden}`);
  }
});
