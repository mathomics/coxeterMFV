/**
 * The editable map: what is drawn, as opposed to what exists.
 *
 * The invariants worth defending are that a map may legitimately hold every
 * reaction, some of them or none; that an empty one still draws the structure of
 * the community; that the shell it draws is the same shell a full map draws; and
 * that none of this can reach the model or the solver.
 */

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { FluxProblem } from '../src/solver/fba.js';
import { layoutCommunity } from '../src/layout/layout.js';
import { renderMap } from '../src/render/mapview.js';
import { FluxScale } from '../src/render/theme.js';
import { signedDistance } from '../src/layout/geometry.js';
import { EditableMap } from '../src/map/editable-map.js';
import { BOUNDARY_KIND, boundaryKey, ENVIRONMENT_KEY } from '../src/map/boundaries.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function loadModel(name) {
  return new Model(JSON.parse(await readFile(join(root, 'models', `${name}.json`), 'utf8')));
}

const MEDIUM = {
  glc__D: { lower: -10 }, o2: { lower: -20 }, nh4: { lower: -1000 },
  pi: { lower: -1000 }, h2o: { lower: -1000 }, h: { lower: -1000 },
  co2: { lower: -1000 }, photon: { lower: -80 },
};

async function buildCommunity(names = [['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc']]) {
  const species = [];
  for (const [id, file] of names) {
    species.push({ id, model: await loadModel(file), abundance: 1 / names.length });
  }
  return new CommunityModel({ species, medium: MEDIUM });
}

const render = (map) => renderMap(map, { fluxes: new Map(), scale: new FluxScale([]) });

// Empty maps

test('an empty one-species map draws the cell and its compartments only', async () => {
  const com = await buildCommunity([['eco', 'ecoli_core_mc']]);
  const map = EditableMap.emptyFromCommunity(com);

  assert.equal(map.cells.length, 1);
  const cell = map.cells[0];
  assert.equal(cell.id, 'eco');
  assert.ok(cell.rect.w > 0 && cell.rect.h > 0, 'the cell has no area');
  // e > p > c, every one of them drawn.
  assert.deepEqual(cell.compartments.map((c) => c.id).sort(), ['c', 'e', 'p']);
  assert.ok(cell.outerCompartment, 'no outer compartment');
  assert.ok(map.environment.rect.w > 0, 'no shared medium');

  // And nothing inside them.
  assert.equal(map.reactions.size, 0, 'an empty map drew a reaction');
  assert.equal(map.nodes.size, 0, 'an empty map drew a metabolite');
  assert.equal(map.segments.length, 0, 'an empty map drew a segment');
  assert.deepEqual(map.medium.nodes, []);
  assert.deepEqual(map.medium.exchanges, []);
  assert.ok(map.isEmpty());
});

test('an empty multi-species map draws every cell', async () => {
  const com = await buildCommunity([
    ['eco', 'ecoli_core_mc'], ['sce', 'yeast_core_mc'],
    ['mba', 'methanogen_core_mc'], ['lla', 'lactobacillus_core_mc'],
  ]);
  const map = EditableMap.emptyFromCommunity(com);

  assert.deepEqual(map.cells.map((c) => c.id).sort(), ['eco', 'lla', 'mba', 'sce']);
  for (const cell of map.cells) {
    const model = com.species.find((s) => s.id === cell.id).model;
    assert.deepEqual(cell.compartments.map((c) => c.id).sort(), model.compartmentIds().sort(),
      `${cell.id} is missing compartments`);
  }
  // Cells still do not overlap, and still sit inside the medium.
  const rects = map.cells.map((c) => c.rect);
  for (let i = 0; i < rects.length; i++) {
    for (let j = i + 1; j < rects.length; j++) {
      const a = rects[i];
      const b = rects[j];
      assert.ok(!(a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h),
        `cells ${i} and ${j} overlap`);
    }
    assert.ok(signedDistance(map.environment.rect, { x: rects[i].x, y: rects[i].y }) < 0,
      'a cell escaped the shared medium');
  }
  assert.equal(map.reactions.size, 0);
  assert.equal(map.nodes.size, 0);
  assert.equal(map.segments.length, 0);
});

test('the compartment hierarchy of an empty map still nests strictly', async () => {
  const com = await buildCommunity();
  const map = EditableMap.emptyFromCommunity(com);
  let checked = 0;
  for (const cell of map.cells) {
    const byId = new Map(cell.compartments.map((c) => [c.id, c]));
    for (const comp of cell.compartments) {
      if (!comp.parentId) continue;
      const parent = byId.get(comp.parentId);
      assert.ok(parent, `${comp.id} has no parent rect`);
      assert.ok(comp.rect.x >= parent.rect.x - 0.5, `${comp.id} escapes left of ${parent.id}`);
      assert.ok(comp.rect.y >= parent.rect.y - 0.5, `${comp.id} escapes above ${parent.id}`);
      assert.ok(comp.rect.x + comp.rect.w <= parent.rect.x + parent.rect.w + 0.5, `${comp.id} escapes right`);
      assert.ok(comp.rect.y + comp.rect.h <= parent.rect.y + parent.rect.h + 0.5, `${comp.id} escapes below`);
      checked += 1;
    }
  }
  assert.ok(checked >= 4, `only checked ${checked} nested compartments`);
});

test('an empty map has exactly the shell a full map has', async () => {
  // The shell must not depend on what is drawn inside it, or a user who starts
  // empty and fills the map in would watch the cells move under them.
  const com = await buildCommunity();
  const full = EditableMap.fromCommunity(com);
  const empty = EditableMap.emptyFromCommunity(com);

  assert.deepEqual(empty.canvas, full.canvas);
  assert.deepEqual(empty.environment.rect, full.environment.rect);
  assert.equal(empty.cells.length, full.cells.length);
  for (let i = 0; i < full.cells.length; i++) {
    assert.deepEqual(empty.cells[i].rect, full.cells[i].rect, `cell ${i} moved`);
    assert.deepEqual(
      empty.cells[i].compartments.map((c) => ({ id: c.id, rect: c.rect, parentId: c.parentId })),
      full.cells[i].compartments.map((c) => ({ id: c.id, rect: c.rect, parentId: c.parentId })),
      `compartments of cell ${i} moved`,
    );
  }
  assert.deepEqual(
    empty.boundaries.map((b) => b.id).sort(),
    full.boundaries.map((b) => b.id).sort(),
    'the boundaries differ',
  );
});

test('an empty map renders as valid SVG with no metabolic content', async () => {
  const com = await buildCommunity();
  const svg = render(EditableMap.emptyFromCommunity(com));

  assert.ok(svg.startsWith('<svg') && svg.endsWith('</svg>'));
  assert.ok(!/NaN|undefined|Infinity/.test(svg), 'a numeric attribute did not resolve');
  for (const tag of ['g', 'text', 'defs']) {
    const open = (svg.match(new RegExp(`<${tag}[\\s>]`, 'g')) ?? []).length;
    const close = (svg.match(new RegExp(`</${tag}>`, 'g')) ?? []).length;
    assert.equal(open, close, `unbalanced <${tag}>`);
  }
  // The structure is there...
  for (const cell of com.activeSpecies) {
    assert.ok(svg.includes(`data-cell="${cell.id}"`), `cell ${cell.id} not drawn`);
  }
  assert.ok(svg.includes('data-compartment="c"'), 'compartments not drawn');
  assert.ok(svg.includes('emc-layer-env'), 'the shared medium is not drawn');
  // ...and nothing else is.
  assert.ok(!svg.includes('data-rxn='), 'a reaction marker was drawn');
  assert.ok(!svg.includes('data-node='), 'a metabolite node was drawn');
  assert.ok(!svg.includes('emc-marker'), 'a marker glyph was drawn');
  assert.ok(!svg.includes('emc-seg'), 'a segment was drawn');
});

// Full conversion

test('a full automatic layout converts to an equivalent map', async () => {
  const com = await buildCommunity();
  const layout = layoutCommunity(com);
  const map = EditableMap.fromLayout(layout);

  assert.ok(map instanceof EditableMap);
  assert.equal(map.width, layout.width);
  assert.equal(map.height, layout.height);
  assert.deepEqual(map.canvas, { width: layout.width, height: layout.height });
  assert.equal(map.environment, layout.environment);
  assert.equal(map.cells, layout.cells);
  assert.equal(map.nodes.size, layout.nodes.size);
  assert.equal(map.reactions.size, layout.reactions.size);
  assert.equal(map.segments.length, layout.segments.length);
  assert.ok(map.reactions.size > 100, `only ${map.reactions.size} reactions`);
  assert.ok(!map.isEmpty());

  // Membership answers agree with the layout it came from.
  for (const id of layout.reactions.keys()) assert.ok(map.hasReaction(id), id);
  for (const id of layout.nodes.keys()) assert.ok(map.hasNode(id), id);
  assert.ok(!map.hasReaction('not-a-reaction'));
  assert.ok(!map.hasNode('not-a-node'));

  // And it draws exactly what the layout drew.
  assert.equal(render(map), render(layout), 'the SVG changed');
});

test('a map built from a community matches one built from its layout', async () => {
  const com = await buildCommunity([['eco', 'ecoli_core_mc']]);
  const direct = EditableMap.fromCommunity(com);
  const viaLayout = EditableMap.fromLayout(layoutCommunity(com));
  assert.equal(render(direct), render(viaLayout));
});

// Boundaries

test('every compartment of every cell becomes a boundary, plus the environment', async () => {
  const com = await buildCommunity();
  for (const map of [EditableMap.fromCommunity(com), EditableMap.emptyFromCommunity(com)]) {
    const env = map.boundary(ENVIRONMENT_KEY);
    assert.ok(env, 'no environment boundary');
    assert.equal(env.kind, BOUNDARY_KIND.ENVIRONMENT);
    assert.equal(env.rect, map.environment.rect, 'the environment boundary has no geometry');

    let expected = 1;
    for (const cell of map.cells) {
      for (const comp of cell.compartments) {
        const b = map.boundary(boundaryKey(cell.id, comp.id));
        assert.ok(b, `no boundary for ${cell.id}/${comp.id}`);
        // Identity: whose it is, and which two compartments it lies between.
        assert.equal(b.cellId, cell.id);
        assert.equal(b.inside, comp.id);
        assert.equal(b.outside, comp.parentId ?? 's');
        assert.equal(b.kind, comp.parentId === null ? BOUNDARY_KIND.CELL : BOUNDARY_KIND.MEMBRANE);
        // Geometry, for a later phase to hit-test against.
        assert.equal(b.rect, comp.rect);
        assert.ok(b.membrane, `${b.id} has no membrane style`);
        assert.ok(Number.isFinite(b.depth));
        expected += 1;
      }
      assert.equal(map.boundariesOf(cell.id).length, cell.compartments.length);
    }
    assert.equal(map.boundaries.length, expected);
    assert.ok(map.hasBoundary(ENVIRONMENT_KEY));
    assert.ok(!map.hasBoundary('nope/x'));
    assert.equal(map.boundary('nope/x'), null);
  }
});

// Partial maps

test('a partial map keeps its reactions, their segments and their metabolites', async () => {
  const com = await buildCommunity([['eco', 'ecoli_core_mc']]);
  const full = EditableMap.fromCommunity(com);
  const keep = full.reactionIds().slice(0, 12);
  const partial = full.subsetByReactions(keep);

  assert.deepEqual(partial.reactionIds().sort(), [...keep].sort());
  assert.ok(partial.reactions.size < full.reactions.size);
  assert.ok(!partial.isEmpty());

  // Every segment belongs to a kept reaction, and every kept reaction keeps all
  // of the segments it had.
  for (const seg of partial.segments) {
    assert.ok(partial.hasReaction(seg.reactionId), `${seg.reactionId} has an orphan segment`);
  }
  for (const id of keep) {
    assert.equal(partial.segmentsOf(id).length, full.segmentsOf(id).length, `${id} lost segments`);
  }
  // Every node still has something attached to it, and every segment's node exists.
  for (const seg of partial.segments) {
    if (!seg.nodeId) continue;
    assert.ok(partial.hasNode(seg.nodeId), `segment points at missing node ${seg.nodeId}`);
  }
  for (const id of partial.nodeIds()) {
    assert.ok(partial.segmentsAt(id).length > 0, `${id} is an orphan node`);
  }
  // The shell is untouched.
  assert.deepEqual(partial.canvas, full.canvas);
  assert.equal(partial.cells.length, full.cells.length);
  assert.equal(partial.boundaries.length, full.boundaries.length);

  const svg = render(partial);
  assert.ok(svg.startsWith('<svg') && !/NaN|undefined|Infinity/.test(svg));
  for (const id of keep) assert.ok(svg.includes(`data-rxn="${id}"`), `${id} not drawn`);
});

test('a map with zero reactions is a valid map, not a broken one', async () => {
  const com = await buildCommunity([['eco', 'ecoli_core_mc']]);
  const full = EditableMap.fromCommunity(com);
  const cleared = full.cleared();

  assert.equal(cleared.reactions.size, 0);
  assert.equal(cleared.nodes.size, 0);
  assert.equal(cleared.segments.length, 0);
  assert.ok(cleared.isEmpty());
  assert.deepEqual(cleared.subsetByReactions([]).stats(), cleared.stats());
  // Same shell as the full map it came from, and it renders.
  assert.equal(cleared.cells.length, full.cells.length);
  assert.equal(cleared.boundaries.length, full.boundaries.length);
  const svg = render(cleared);
  assert.ok(svg.includes('data-cell="eco"') && !svg.includes('data-rxn='));
});

test('a subset is independent of the map it came from', async () => {
  const com = await buildCommunity([['eco', 'ecoli_core_mc']]);
  const full = EditableMap.fromCommunity(com);
  const withNode = full.segments.find((seg) => seg.nodeId && full.nodes.get(seg.nodeId)?.kind === 'metabolite');
  const partial = full.subsetByReactions([withNode.reactionId]);

  const id = withNode.nodeId;
  const before = { x: full.nodes.get(id).x, y: full.nodes.get(id).y };
  assert.ok(partial.moveNode(id, before.x + 25, before.y + 25), 'the subset refused to move its node');

  assert.deepEqual({ x: full.nodes.get(id).x, y: full.nodes.get(id).y }, before,
    'moving a node in the subset moved it in the original');
  assert.notDeepEqual({ x: partial.nodes.get(id).x, y: partial.nodes.get(id).y }, before);
  // The axis of the copied reaction is its own object too.
  assert.notEqual(partial.reactions.get(withNode.reactionId).axis,
    full.reactions.get(withNode.reactionId).axis);
});

// V0.2 drag, via map

test('metabolite dragging still works through the map', async () => {
  const com = await buildCommunity();
  const map = EditableMap.fromCommunity(com);
  const node = [...map.nodes.values()].find((n) => n.kind === 'metabolite');
  const attached = map.segmentsAt(node.id);
  assert.ok(attached.length > 0, `${node.id} has no segments`);

  assert.ok(map.canMoveNode(node.id));
  const moved = map.moveNode(node.id, node.x + 20, node.y + 14);
  assert.equal(moved, node);
  // The clamp may refuse part of the move; what must hold is that the segments
  // followed wherever it ended up.
  for (const seg of attached) {
    const end = seg.side === 'substrate' ? seg.from : seg.to;
    assert.ok(Math.abs(end.x - node.x) < 1e-6 && Math.abs(end.y - node.y) < 1e-6,
      `${seg.reactionId} did not follow`);
  }

  // The V0.2 eligibility policy is unchanged.
  const currency = [...map.nodes.values()].find((n) => n.kind === 'currency');
  const was = { x: currency.x, y: currency.y };
  assert.equal(map.canMoveNode(currency.id), true);
  assert.equal(map.moveNode(currency.id, was.x + 35, was.y + 25), currency);
  assert.notDeepEqual({ x: currency.x, y: currency.y }, was);

  const medium = [...map.nodes.values()].find((n) => n.kind === 'medium');
  assert.ok(map.canMoveNode(medium.id), 'shared-medium nodes must stay draggable');
});

// I/O

test('a map survives a JSON round trip, axis identity included', async () => {
  const com = await buildCommunity([['eco', 'ecoli_core_mc']]);
  const map = EditableMap.fromCommunity(com);
  const back = EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON())));

  assert.deepEqual(back.stats(), map.stats());
  assert.equal(render(back), render(map), 'the round trip changed the drawing');
  assert.deepEqual(back.boundaries.map((b) => b.id), map.boundaries.map((b) => b.id));

  // The axis segment must share objects with its reaction again, or moving an
  // axis later would leave the drawing behind.
  let checked = 0;
  for (const seg of back.segments) {
    const axis = back.reactions.get(seg.reactionId)?.axis;
    if (!axis) continue;
    if (seg.kind === 'axis') {
      assert.equal(seg.from, axis.a1, `${seg.reactionId} axis start not re-linked`);
      assert.equal(seg.to, axis.a2, `${seg.reactionId} axis end not re-linked`);
      checked += 1;
    } else if (seg.side === 'substrate') {
      assert.equal(seg.to, axis.a1, `${seg.reactionId} substrate not re-linked`);
    } else if (seg.side === 'product') {
      assert.equal(seg.from, axis.a2, `${seg.reactionId} product not re-linked`);
    }
  }
  assert.ok(checked > 50, `only checked ${checked} axes`);

  // Dragging still behaves on the restored map.
  const node = [...back.nodes.values()].find((n) => n.kind === 'metabolite');
  assert.ok(back.moveNode(node.id, node.x + 5, node.y + 5));
});

test('an empty map survives a JSON round trip', async () => {
  const com = await buildCommunity([['eco', 'ecoli_core_mc']]);
  const empty = EditableMap.emptyFromCommunity(com);
  const back = EditableMap.fromJSON(JSON.parse(JSON.stringify(empty.toJSON())));
  assert.ok(back.isEmpty());
  assert.deepEqual(back.stats(), empty.stats());
  assert.equal(render(back), render(empty));
});

// The map cannot reach the LP

test('what is drawn has no effect on the simulation', async () => {
  const com = await buildCommunity();
  const reference = new FluxProblem(com.merged).parsimonious();
  const bounds = [...com.merged.reactions.values()].map((r) => `${r.id}:${r.lowerBound}:${r.upperBound}`);

  // Build every kind of map this phase allows, in turn.
  const full = EditableMap.fromCommunity(com);
  const empty = EditableMap.emptyFromCommunity(com);
  const partial = full.subsetByReactions(full.reactionIds().slice(0, 25));
  for (const map of [full, empty, partial, full.cleared()]) {
    for (const node of [...map.nodes.values()].slice(0, 20)) {
      map.moveNode(node.id, node.x + 40, node.y - 30);
    }
  }

  const after = new FluxProblem(com.merged).parsimonious();
  assert.equal(after.status, reference.status);
  assert.equal(after.objectiveValue, reference.objectiveValue);
  for (const [id, v] of reference.fluxes) {
    assert.equal(after.fluxes.get(id), v, `flux of ${id} changed`);
  }
  assert.deepEqual([...com.merged.reactions.values()].map((r) => `${r.id}:${r.lowerBound}:${r.upperBound}`),
    bounds, 'a reaction bound moved');
  assert.equal(com.merged.reactions.size, bounds.length);
});

test('the model keeps every reaction a partial map leaves out', async () => {
  const com = await buildCommunity([['eco', 'ecoli_core_mc']]);
  const full = EditableMap.fromCommunity(com);
  const partial = full.subsetByReactions(full.reactionIds().slice(0, 5));

  const hidden = full.reactionIds().filter((id) => !partial.hasReaction(id));
  assert.ok(hidden.length > 50, `only ${hidden.length} reactions hidden`);
  for (const id of hidden) {
    assert.ok(com.merged.reactions.has(id), `${id} vanished from the model when it left the map`);
  }
});

// Determinism

test('building a map twice gives the same map', async () => {
  const runs = [];
  for (let i = 0; i < 2; i++) {
    const com = await buildCommunity([['eco', 'ecoli_core_mc']]);
    runs.push({
      full: render(EditableMap.fromCommunity(com)),
      empty: render(EditableMap.emptyFromCommunity(com)),
    });
  }
  assert.equal(runs[0].full, runs[1].full);
  assert.equal(runs[0].empty, runs[1].empty);
});


test('saved incremental edits survive a JSON round trip', async () => {
  const com = await buildCommunity([['eco', 'ecoli_core_mc']]);
  const map = EditableMap.emptyFromCommunity(com);

  const reactionId = [...com.merged.reactions.keys()]
    .find((id) => !id.includes('COMMUNITY_GROWTH'));
  assert.ok(reactionId);
  assert.ok(map.addReaction(com, reactionId));

  const movable = [...map.nodes.values()].find((n) => map.canMoveNode(n.id));
  assert.ok(movable);
  map.moveNode(movable.id, movable.x + 9, movable.y + 7);

  const json = JSON.stringify(map.toJSON());
  const back = EditableMap.fromJSON(JSON.parse(json));

  assert.deepEqual(back.toJSON(), map.toJSON());
  assert.deepEqual(back.stats(), map.stats());
});
