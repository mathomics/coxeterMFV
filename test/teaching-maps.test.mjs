/**
 * The curated maps of the teaching scenarios are what their builder writes,
 * draw their cofactors quiet, and keep the shared medium beside the cells: every
 * medium node stands at a cell's exchange, and each metabolite has one pool,
 * at one of them, with the community's supply and washout.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { buildScenarioMap } from '../tools/scenario-maps/teaching-maps.mjs';

const catalog = JSON.parse(await readFile(new URL('../models/scenarios.json', import.meta.url), 'utf8'));
const teaching = catalog.scenarios.filter((s) => s.species?.every((sp) => /_core_mc$/.test(sp.model)));

test('there are seven teaching scenarios, each with its own map', () => {
  assert.deepEqual(teaching.map((s) => s.map), teaching.map((s) => `maps/${s.id}.json`));
  assert.equal(teaching.length, 7);
});

for (const scenario of teaching) {
  test(`${scenario.id}: rebuilt as committed, quiet, with its medium beside the cells`, async () => {
    const { doc } = await buildScenarioMap(scenario);
    const file = JSON.parse(await readFile(new URL(`../models/${scenario.map}`, import.meta.url), 'utf8'));
    assert.deepEqual(file, JSON.parse(JSON.stringify(doc)), 'rerun tools/scenario-maps/teaching-maps.mjs');

    const map = file.map;
    assert.equal(map.cofactorStyle, 'quiet');
    const reactions = new Map(map.reactions.map((r) => [r.id, r]));
    const medium = map.nodes.filter((n) => n.kind === 'medium');
    for (const node of medium) {
      const exchange = map.segments.find((s) => s.nodeId === node.id && reactions.get(s.reactionId)?.cellId);
      assert.ok(exchange, `${node.id} stands at a cell's exchange, not in a row of its own`);
    }
    const pools = medium.filter((n) => !n.reactionLocal);
    assert.equal(new Set(pools.map((n) => n.mergedId)).size, pools.length, 'one pool per metabolite');
    for (const id of map.medium.exchanges) {
      const stub = map.segments.find((s) => s.reactionId === id && s.nodeId);
      assert.ok(pools.some((n) => n.id === stub?.nodeId), `${id} leaves from its pool`);
    }
  });
}
