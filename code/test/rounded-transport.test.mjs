import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { closestPerimeterPoint, compartmentCornerRadius } from '../src/layout/geometry.js';

function boundaryDistance(rect, point, radius) {
  const nearestX = Math.max(rect.x + radius, Math.min(rect.x + rect.w - radius, point.x));
  const nearestY = Math.max(rect.y + radius, Math.min(rect.y + rect.h - radius, point.y));
  return Math.hypot(point.x - nearestX, point.y - nearestY) - radius;
}

test('rounded projection follows all corners and returns a radial unit normal', () => {
  const rect = { x: 100, y: 200, w: 300, h: 500 };
  const radius = 108;
  for (const x of [rect.x, rect.x + rect.w]) {
    for (const y of [rect.y, rect.y + rect.h]) {
      const point = closestPerimeterPoint(rect, { x, y }, radius);
      assert.ok(Math.abs(boundaryDistance(rect, point, radius)) < 1e-9);
      assert.ok(Math.abs(Math.hypot(point.nx, point.ny) - 1) < 1e-9);
      assert.ok(Math.abs(Math.abs(point.nx) - Math.SQRT1_2) < 1e-9);
      assert.ok(point.x !== x && point.y !== y);
    }
  }
});

test('a mitochondrial transporter stays on the rendered arc when placed, dragged and restored', async () => {
  const source = await readFile(new URL('../models/yeast_core_mc.json', import.meta.url), 'utf8');
  const community = new CommunityModel({
    species: [{ id: 'yeast', model: new Model(JSON.parse(source)), abundance: 1 }],
    medium: {},
  });
  const map = EditableMap.emptyFromCommunity(community);
  const cell = map.cells[0];
  const compartment = cell.compartments.find(compartment => compartment.id === 'm');
  const rect = compartment.rect;
  const radius = compartmentCornerRadius(compartment);
  const boundaryId = cell.id + '/m';
  const reactionId = 'yeast@PYRt2m';
  assert.ok(map.addReaction(community, reactionId, { boundaryId, anchor: { x: rect.x, y: rect.y } }));
  const reaction = map.reactions.get(reactionId);
  assert.ok(Math.abs(boundaryDistance(rect, reaction, radius)) < 1e-9);
  const beforeNodes = JSON.stringify([...map.nodes.values()].filter(node => !node.reactionId));
  const laneDepths = reaction.transportLanes.map(lane => ({
    inside: (lane.insidePoint.x - reaction.x) * reaction.normal.x + (lane.insidePoint.y - reaction.y) * reaction.normal.y,
    outside: (lane.outsidePoint.x - reaction.x) * reaction.normal.x + (lane.outsidePoint.y - reaction.y) * reaction.normal.y,
  }));
  for (const x of [rect.x, rect.x + rect.w]) {
    for (const y of [rect.y, rect.y + rect.h]) {
      assert.ok(map.moveReaction(reactionId, x, y));
      assert.ok(Math.abs(boundaryDistance(rect, reaction, radius)) < 1e-9);
      assert.ok(Math.abs(Math.hypot(reaction.normal.x, reaction.normal.y) - 1) < 1e-9);
      for (let index = 0; index < reaction.transportLanes.length; index += 1) {
        const lane = reaction.transportLanes[index];
        const inside = (lane.insidePoint.x - reaction.x) * reaction.normal.x + (lane.insidePoint.y - reaction.y) * reaction.normal.y;
        const outside = (lane.outsidePoint.x - reaction.x) * reaction.normal.x + (lane.outsidePoint.y - reaction.y) * reaction.normal.y;
        assert.ok(boundaryDistance(rect, lane.insidePoint, radius) < 0);
        assert.ok(boundaryDistance(rect, lane.outsidePoint, radius) > 0);
        assert.ok(Math.abs(inside - laneDepths[index].inside) < 1e-6);
        assert.ok(Math.abs(outside - laneDepths[index].outside) < 1e-6);
      }
    }
  }
  assert.equal(JSON.stringify([...map.nodes.values()].filter(node => !node.reactionId)), beforeNodes);
  const restored = EditableMap.fromJSON(map.toJSON());
  assert.deepEqual(restored.toJSON(), map.toJSON());
  assert.ok(restored.moveReaction(reactionId, rect.x, rect.y));
  assert.ok(Math.abs(boundaryDistance(rect, restored.reactions.get(reactionId), radius)) < 1e-9);
});
