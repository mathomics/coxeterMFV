import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isDrawnCrossFeedingBase, mediumEndpoints, routeAnchor, routeCrossFeeding } from '../src/render/crossfeed-routes.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function gut9() {
  const doc = JSON.parse(await readFile(join(root, 'models', 'published', 'steadycom-gut-nine', 'map.json'), 'utf8'));
  const M = doc.map;
  return { width: M.canvas.width, height: M.canvas.height, cells: M.cells, nodes: M.nodes, reactions: M.reactions, segments: M.segments };
}

/** Every exchanged metabolite that one species secretes and another takes up. */
function linksOf(layout, pairs) {
  const ends = mediumEndpoints(layout);
  return pairs.filter(([from, base, to]) => ends.has(`${from}|${base}`) && ends.has(`${to}|${base}`))
    .map(([from, base, to]) => ({ from, base, to, flux: 1 }));
}

const inside = (p, rect) => p.x > rect.x && p.x < rect.x + rect.w && p.y > rect.y && p.y < rect.y + rect.h;

test('protons and water get no cross-feeding line', () => {
  assert.equal(isDrawnCrossFeedingBase('h'), false);
  assert.equal(isDrawnCrossFeedingBase('h2o'), false);
  assert.equal(isDrawnCrossFeedingBase('cpd00067'), false);
  assert.equal(isDrawnCrossFeedingBase('ac'), true);
});

test('a cross-feeding line runs from the producer\'s medium node to the consumer\'s, outside every cell', async () => {
  const layout = await gut9();
  const ends = mediumEndpoints(layout);
  const links = linksOf(layout, [
    ['Bt', 'ac', 'Fp'], ['St', 'lac-L', 'Bt'], ['Kp', 'nh4', 'St'], ['Ef', 'gln-L', 'Lc'], ['Ec', 'asp-L', 'Ef'],
    ['Bt', 'h', 'Fp'],
  ]);
  const routes = routeCrossFeeding(layout, links);
  assert.equal(routes.has('Bt|h|Fp'), false, 'no line for protons');
  for (const link of links.filter((l) => l.base !== 'h')) {
    const route = routes.get(`${link.from}|${link.base}|${link.to}`);
    assert.ok(route, `${link.base} ${link.from}→${link.to} is routed`);
    const a = ends.get(`${link.from}|${link.base}`);
    const b = ends.get(`${link.to}|${link.base}`);
    assert.deepEqual(route.points[0], { x: a.x, y: a.y }, 'starts at the producer\'s node');
    assert.deepEqual(route.points.at(-1), { x: b.x, y: b.y }, 'ends at the consumer\'s node');
    // horizontal and vertical runs only
    for (let i = 1; i < route.points.length; i++) {
      const p = route.points[i - 1];
      const q = route.points[i];
      assert.ok(Math.abs(p.x - q.x) < 1e-6 || Math.abs(p.y - q.y) < 1e-6, 'runs are straight');
    }
    // every corner, and the middle of every run, lies outside the cells
    for (let i = 1; i < route.points.length - 1; i++) {
      const p = route.points[i];
      const q = route.points[i + 1];
      const mid = { x: (p.x + q.x) / 2, y: (p.y + q.y) / 2 };
      for (const cell of layout.cells) {
        assert.ok(!inside(p, cell.rect), `${link.base}: corner inside ${cell.id}`);
        if (i < route.points.length - 2) assert.ok(!inside(mid, cell.rect), `${link.base}: run across ${cell.id}`);
      }
    }
  }
});

test('routes are reused while the same links exist', async () => {
  const layout = await gut9();
  const links = linksOf(layout, [['Bt', 'ac', 'Fp'], ['St', 'lac-L', 'Bt']]);
  const first = routeCrossFeeding(layout, links);
  const again = routeCrossFeeding(layout, links.map((l) => ({ ...l, flux: 7 })));
  assert.equal(again, first, 'a new flux does not re-route');
});

test("a picked line's name is pinned where it was clicked, or mid-way along its longest run", () => {
  const line = [{ x: 0, y: 0 }, { x: 0, y: 100 }, { x: 400, y: 100 }];
  assert.deepEqual(routeAnchor(line, { x: 250, y: 130 }), { x: 250, y: 100 });
  assert.deepEqual(routeAnchor(line, { x: -20, y: 40 }), { x: 0, y: 40 });
  assert.deepEqual(routeAnchor(line), { x: 200, y: 100 });
});
