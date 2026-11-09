import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { Store } from '../src/ui/store.js';
import { FluxScale } from '../src/render/theme.js';
import { renderMap, geometryHandleSide } from '../src/render/mapview.js';
import {
  closestPerimeterPoint, compartmentCorners, compartmentShape, COMPARTMENT_SHAPES,
} from '../src/layout/geometry.js';
import { shapeAmountAt, shapeHandlePoint } from '../src/map/geometry-edit.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function yeast() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'yeast_core_mc.json'), 'utf8'));
  const model = new Model(raw);
  const community = new CommunityModel({ species: [{ id: 'yeast', model, abundance: 1 }], medium: {} });
  return { model, community };
}

async function yeastStore() {
  const { model, community } = await yeast();
  const store = new Store('.');
  store.models.set('fixture', model);
  store.species = [{ id: 'yeast', file: 'fixture', abundance: 1, enabled: true }];
  store.community = community;
  store.map = EditableMap.blank({ width: 1200, height: 850 });
  store.scale = new FluxScale([]);
  store.addMapCell('yeast', { x: 80, y: 70, w: 900, h: 650 });
  return store;
}

/* How far a point is from an ellipse, in units of the ellipse: 0 on it. */
const onEllipse = (rect, p) => Math.hypot(
  (p.x - (rect.x + rect.w / 2)) / (rect.w / 2),
  (p.y - (rect.y + rect.h / 2)) / (rect.h / 2),
) - 1;

test('an unedited compartment keeps the corners it always had', async () => {
  const { community } = await yeast();
  const map = EditableMap.emptyFromCommunity(community);
  const [cell] = map.cells;
  const m = cell.compartments.find((c) => c.id === 'm');
  const c = cell.compartments.find((x) => x.id === 'c');
  const min = (r) => Math.min(r.w, r.h);
  assert.equal(m.shape, undefined);
  assert.ok(Math.abs(compartmentCorners(m).rx - min(m.rect) * 0.36) < 1e-9, 'round organelle');
  assert.ok(Math.abs(compartmentCorners(c).rx - Math.min(40, min(c.rect) * 0.16)) < 1e-9, 'square-ish shell');
  assert.equal(compartmentShape(m).oval, 0);
});

test('the oval amount pulls the corners out to an oval, and the corner amount still shapes that oval', () => {
  const rect = { x: 100, y: 50, w: 600, h: 300 };
  const corners = (shape) => compartmentCorners({ id: 'x', rect, shape });
  const close = (a, b) => Math.abs(a.rx - b.rx) < 1e-9 && Math.abs(a.ry - b.ry) < 1e-9;
  assert.deepEqual(corners({ corner: 0, oval: 0 }), { rx: 0, ry: 0 });
  assert.deepEqual(corners({ corner: 1, oval: 0 }), { rx: 150, ry: 150 });
  // A full oval is the ellipse at its roundest, and still an oval at its boxiest:
  // the corner diamond does not go dead once the outline is oval.
  assert.deepEqual(corners({ corner: 1, oval: 1 }), { rx: 300, ry: 150 });
  assert.ok(close(corners({ corner: 0, oval: 1 }), { rx: 90, ry: 45 }), JSON.stringify(corners({ corner: 0, oval: 1 })));
  const boxier = corners({ corner: 0.4, oval: 1 });
  assert.ok(boxier.rx < 300 && boxier.rx > 90 && Math.abs(boxier.rx / boxier.ry - 2) < 1e-9, 'in between, in proportion');
  // Halfway: circular corners of 75 blended halfway to an oval reaching 0.65 of each half side.
  assert.ok(close(corners({ corner: 0.5, oval: 0.5 }), { rx: 75 + 0.5 * (195 - 75), ry: 75 + 0.5 * (97.5 - 75) }));

  // Projected onto the full ellipse, any point lands on it, with the ellipse's own normal.
  const ellipse = corners({ corner: 1, oval: 1 });
  for (const p of [{ x: 90, y: 40 }, { x: 650, y: 330 }, { x: 400, y: 0 }, { x: 300, y: 200 }, { x: 800, y: 200 }]) {
    const q = closestPerimeterPoint(rect, p, ellipse);
    assert.ok(Math.abs(onEllipse(rect, q)) < 1e-9, JSON.stringify(q));
    const gx = (q.x - 400) / (300 * 300);
    const gy = (q.y - 200) / (150 * 150);
    const g = Math.hypot(gx, gy);
    assert.ok(Math.abs(q.nx - gx / g) < 1e-9 && Math.abs(q.ny - gy / g) < 1e-9);
  }
});

test('reshaping a compartment moves its transporters onto the new outline, turned to face out', async () => {
  const { community } = await yeast();
  const map = EditableMap.emptyFromCommunity(community);
  const [cell] = map.cells;
  const m = cell.compartments.find((c) => c.id === 'm');
  const rect = { ...m.rect };
  const id = 'yeast@PYRt2m';
  assert.ok(map.addReaction(community, id, { boundaryId: `${cell.id}/m`, anchor: { x: rect.x, y: rect.y } }));
  const rxn = map.reactions.get(id);
  const owned = () => [...map.nodes.values()].filter((n) => n.reactionId === id)
    .map((n) => Math.hypot(n.x - rxn.x, n.y - rxn.y));
  const reach = owned();
  const others = JSON.stringify([...map.nodes.values()].filter((n) => !n.reactionId));

  // Only the amount given changes; the corner it had stays.
  const corner = compartmentShape(m).corner;
  assert.deepEqual(map.editCompartmentShape(cell.id, 'm', { oval: 0.5 }).shape, { corner, oval: 0.5 });
  const result = map.editCompartmentShape(cell.id, 'm', { corner: 1, oval: 1 });
  assert.deepEqual(result.shape, { corner: 1, oval: 1 });
  assert.deepEqual(m.rect, rect, 'the rectangle stays');
  assert.ok(Math.abs(onEllipse(rect, rxn)) < 1e-9, 'the transporter sits on the ellipse');
  const gx = (rxn.x - (rect.x + rect.w / 2)) / (rect.w / 2) ** 2;
  const gy = (rxn.y - (rect.y + rect.h / 2)) / (rect.h / 2) ** 2;
  assert.ok(Math.abs(rxn.normal.x * gy - rxn.normal.y * gx) < 1e-9, 'it faces out of the ellipse');
  owned().forEach((d, i) => assert.ok(Math.abs(d - reach[i]) < 1e-6, 'its metabolites came along'));
  assert.equal(JSON.stringify([...map.nodes.values()].filter((n) => !n.reactionId)), others);

  // Saved and opened again, it keeps its shape.
  const restored = EditableMap.fromJSON(map.toJSON());
  assert.deepEqual(restored.cells[0].compartments.find((c) => c.id === 'm').shape, m.shape);

  // The shared medium has no shape to edit.
  assert.equal(map.editCompartmentShape(null, map.environment.id, { oval: 1 }), null);
});

test('a shape handle stands on its edge where its amount says, clear of the other handles', () => {
  const rect = { x: 0, y: 0, w: 800, h: 500 };
  const side = 24;
  for (const which of ['corner', 'oval']) {
    for (const amount of [0, 0.25, 0.5, 1]) {
      const p = shapeHandlePoint(rect, which, amount, side);
      assert.ok(Math.abs(shapeAmountAt(rect, which, p, side) - amount) < 1e-9);
    }
    const along = which === 'corner' ? 'x' : 'y';
    const half = (which === 'corner' ? rect.w : rect.h) / 2;
    assert.equal(shapeHandlePoint(rect, which, 0.5, side)[which === 'corner' ? 'y' : 'x'], 0, 'on the edge');
    assert.ok(shapeHandlePoint(rect, which, 0, side)[along] >= side * 1.2 - 1e-9, 'clear of the corner handle');
    assert.ok(shapeHandlePoint(rect, which, 1, side)[along] <= half - side * 1.6 + 1e-9, 'clear of the side handle');
  }
  // Dragged past either end, it stops there.
  assert.equal(shapeAmountAt(rect, 'corner', { x: -50, y: 0 }, side), 0);
  assert.equal(shapeAmountAt(rect, 'oval', { x: 0, y: 900 }, side), 1);
});

test('a selected compartment shows an orange corner diamond and a yellow oval diamond', async () => {
  const { community } = await yeast();
  const map = EditableMap.emptyFromCommunity(community);
  const [cell] = map.cells;
  const m = cell.compartments.find((c) => c.id === 'm');
  map.editCompartmentShape(cell.id, 'm', { corner: 0.4, oval: 0.6 });
  const svg = renderMap(map, {
    scale: { reference: 1, width: () => 2, color: () => '#000' },
    selectedGeometry: { cellId: cell.id, compartmentId: 'm' },
  });
  const side = geometryHandleSide(0);
  for (const [which, colour, amount] of [['corner', '#f97316', 0.4], ['oval', '#ffc512', 0.6]]) {
    const tag = svg.match(new RegExp(`<path class="emc-geometry-handle emc-geometry-shape emc-geometry-${which}"[^>]*>`))?.[0];
    assert.ok(tag, `${which} diamond`);
    assert.match(tag, new RegExp(`fill="${colour}"`));
    const p = shapeHandlePoint(m.rect, which, amount, side);
    const [, x, y] = tag.match(/d="M([\d.-]+) ([\d.-]+)/);
    assert.ok(Math.abs(Number(x) - p.x) < 0.1 && Math.abs(Number(y) - (p.y - side * 0.62)) < 0.1, `${which} placed by its amount`);
  }
  // Over the name plates: the mitochondrion's sits astride the top edge, on the corner diamond's track.
  assert.ok(svg.indexOf('emc-layer-geometry-editor') > svg.indexOf('emc-layer-compartment-labels'),
    'handles are drawn after the compartment names');

  // And the outline is drawn with the elliptical corners.
  const { rx, ry } = compartmentCorners(m);
  assert.ok(rx > ry);
  assert.match(svg, new RegExp(`data-compartment-space="m" [^>]*rx="${Number(rx.toFixed(2))}[^"]*" ry="${Number(ry.toFixed(2))}`));

  // The shared medium is not reshaped, so it gets no diamonds.
  const medium = renderMap(map, {
    scale: { reference: 1, width: () => 2, color: () => '#000' },
    selectedGeometry: { cellId: null, compartmentId: map.environment.id },
  });
  assert.doesNotMatch(medium, /emc-geometry-shape/);
});

test('the compartment list names every compartment, and why the rest cannot be drawn yet', async () => {
  const store = await yeastStore();
  let choices = new Map(store.mapCompartmentChoices().map((c) => [c.compartmentId, c]));
  assert.equal(choices.get('e').status, 'drawn');
  assert.equal(choices.get('c').status, 'ready');
  assert.equal(choices.get('m').status, 'waiting');
  assert.match(choices.get('m').reason, /draw that first/);
  assert.deepEqual(store.availableMapCompartments().map((c) => c.compartmentId),
    [...choices.values()].filter((c) => c.status === 'ready').map((c) => c.compartmentId));

  store.addMapCompartment('yeast', 'c', { x: 130, y: 120, w: 800, h: 540 });
  choices = new Map(store.mapCompartmentChoices().map((c) => [c.compartmentId, c]));
  assert.equal(choices.get('c').status, 'drawn');
  assert.equal(choices.get('im').status, 'ready');
  assert.match(choices.get('im').reason, /^goes inside /);
});

test('a new compartment takes the chosen shape, and a preset reshapes one in a single undo step', async () => {
  const store = await yeastStore();
  store.setCompartmentShapePreset('oval');
  store.addMapCompartment('yeast', 'c', { x: 130, y: 120, w: 800, h: 540 });
  const c = store.map.cells[0].compartments.find((x) => x.id === 'c');
  assert.deepEqual(c.shape, COMPARTMENT_SHAPES.oval);

  store.setCompartmentShapePreset('nonsense');
  assert.equal(store.compartmentShapePreset, 'auto');
  store.addMapCompartment('yeast', 'im', { x: 250, y: 220, w: 360, h: 260 });
  assert.equal(store.map.cells[0].compartments.find((x) => x.id === 'im').shape, undefined, 'auto leaves it as its kind');

  store.setCompartmentShape('yeast', 'c', COMPARTMENT_SHAPES.rectangle);
  assert.deepEqual(store.map.cells[0].compartments.find((x) => x.id === 'c').shape, COMPARTMENT_SHAPES.rectangle);
  store.undoMap();
  assert.deepEqual(store.map.cells[0].compartments.find((x) => x.id === 'c').shape, COMPARTMENT_SHAPES.oval);
});
