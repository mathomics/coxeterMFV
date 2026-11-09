import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { renderMap } from '../src/render/mapview.js';
import { reactionEndpointGeometry } from '../src/render/reaction-path.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

async function fixture() {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const community = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }],
    medium: {},
  });
  const map = EditableMap.emptyFromCommunity(community);
  map.addReaction(community, 'eco@GAPD');
  return { community, map };
}

const scale = {
  reference: 1,
  width: (v) => Math.abs(v) > 1 ? 14 : 1.1,
  color: (v) => Math.abs(v) > 1 ? '#be123c' : '#cbd5e1',
};

test('reaction flux changes colour but never changes stroke width', async () => {
  const { map } = await fixture();
  const a = renderMap(map, {
    fluxes: new Map([['eco@GAPD', 0.1]]),
    scale,
    showCurrency: true,
    currencyLabels: true,
  });
  const b = renderMap(map, {
    fluxes: new Map([['eco@GAPD', 100]]),
    scale,
    showCurrency: true,
    currencyLabels: true,
  });

  const segmentWidths = (svg) => [...svg.matchAll(/class="emc-seg[^"]*"[^>]*stroke-width="([^"]+)"/g)]
    .map((match) => Number(match[1]));
  const low = segmentWidths(a);
  const high = segmentWidths(b);

  assert.ok(low.length > 0);
  assert.deepEqual(low, high);
  assert.ok(low.every((width) => width === 7.5));

  assert.match(a, /stroke="#cbd5e1"/);
  assert.match(b, /stroke="#be123c"/);
});

test('secondary metabolites render as small Escher-style orange circles with labels', async () => {
  const { map } = await fixture();
  const svg = renderMap(map, {
    fluxes: new Map([['eco@GAPD', 10]]),
    scale,
    showCurrency: true,
    currencyLabels: true,
  });

  const satellites = [...map.nodes.values()].filter((node) => node.kind === 'currency');
  assert.ok(satellites.length >= 3);
  assert.ok(satellites.every((node) => node.r === 7));
  assert.match(svg, /fill="#e67e4c"/);
  const circles = [...svg.matchAll(/class="emc-node[^"]*"[^>]*r="([^"]+)"[^>]*fill="([^"]+)"/g)];
  assert.ok(circles.length > 0);
  assert.ok(circles.every((match) => [7, 13].includes(Number(match[1]))));
  assert.ok(circles.every((match) => match[2] === '#e67e4c'));
  assert.match(svg, />nad</);
  assert.match(svg, />nadh</);
  assert.match(svg, />h</);
});


test('flux direction uses metabolite arrowheads and never adds a central backbone arrow', async () => {
  const { map } = await fixture();

  const positive = renderMap(map, {
    fluxes: new Map([['eco@GAPD', 8]]),
    scale,
    showCurrency: true,
    currencyLabels: true,
  });
  const heads = (svg) => [...svg.matchAll(/class="emc-arrow"[^>]*data-node="([^"]+)"/g)].map((match) => match[1]);
  const productNodes = map.segments.filter((seg) => seg.side === 'product').map((seg) => seg.nodeId);
  assert.deepEqual(heads(positive).sort(), productNodes.sort());
  assert.doesNotMatch(positive, /emc-arrow-flux/);

  const negative = renderMap(map, {
    fluxes: new Map([['eco@GAPD', -8]]),
    scale,
    showCurrency: true,
    currencyLabels: true,
  });
  const substrateNodes = map.segments.filter((seg) => seg.side === 'substrate').map((seg) => seg.nodeId);
  assert.deepEqual(heads(negative).sort(), substrateNodes.sort());
  assert.doesNotMatch(negative, /emc-arrow-flux/);

  const zero = renderMap(map, {
    fluxes: new Map([['eco@GAPD', 0]]),
    scale,
    showCurrency: true,
    currencyLabels: true,
  });
  assert.deepEqual(heads(zero), []);
  assert.doesNotMatch(zero, /emc-arrow-flux/);
});

test('primary and secondary branches and their heads use one colour and opacity', async () => {
  const { map } = await fixture();
  const svg = renderMap(map, {
    fluxes: new Map([['eco@GAPD', 8]]), scale, showCurrency: true,
    reactionColorMode: 'subsystem', reactionColors: new Map([['eco@GAPD', '#234abc']]),
    crossFeedingExplorer: { visibleEvents: [] }, showCrossFeeding: true,
  });
  const paths = [...svg.matchAll(/<path class="emc-seg[^"]*"[^>]*stroke="([^"]+)"[^>]*opacity="([^"]+)"/g)];
  const heads = [...svg.matchAll(/<polygon class="emc-arrow"[^>]*fill="([^"]+)"[^>]*opacity="([^"]+)"/g)];
  assert.ok(paths.length > 0 && heads.length > 0);
  assert.ok(paths.every((match) => match[1] === '#234abc' && match[2] === '0.14'));
  assert.ok(heads.every((match) => match[1] === '#234abc' && match[2] === '0.14'));
  const css = await readFile(join(root, 'assets', 'map.css'), 'utf8');
  assert.doesNotMatch(css, /\.emc-seg-currency\s*\{[^}]*opacity\s*:/);
});

test('selected reactions retain matching branch and arrow opacity while cross-feeding is highlighted', async () => {
  const { map } = await fixture();
  for (const selection of [{ selectedReaction: 'eco@GAPD' }, { multiSelection: { reactions: new Set(['eco@GAPD']) } }]) {
    const svg = renderMap(map, {
      fluxes: new Map([['eco@GAPD', 8]]), scale, showCurrency: true,
      crossFeedingExplorer: { visibleEvents: [] }, showCrossFeeding: true, ...selection,
    });
    const paths = [...svg.matchAll(/<path class="emc-seg[^\"]*"[^>]*opacity="([^\"]+)"/g)];
    const heads = [...svg.matchAll(/<polygon class="emc-arrow"[^>]*opacity="([^\"]+)"/g)];
    assert.ok(paths.length > 0 && heads.length > 0);
    assert.ok([...paths, ...heads].every(match => match[1] === '1'));
  }
});

test('the shaft ends at the arrow base and leaves a clear gap before either node size', () => {
  for (const node of [{ kind: 'metabolite', primary: true }, { kind: 'currency', primary: false }, { kind: 'metabolite', primary: false }]) {
    const points = [{ x: 0, y: 0 }, { x: 100, y: 0 }];
    const original = JSON.stringify(points);
    const geometry = reactionEndpointGeometry(points, { node, arrow: true });
    const radius = node.primary ? 13 : 7;
    assert.ok(Math.abs(100 - geometry.arrow.tip.x - radius - 4) < 1e-6);
    assert.equal(geometry.arrow.tip.y, 0);
    assert.ok(geometry.arrow.base.x < geometry.arrow.tip.x);
    const pathEnd = geometry.d.match(/L ([\d.-]+) ([\d.-]+)$/);
    assert.ok(Math.abs(Number(pathEnd[1]) - geometry.arrow.base.x) < 0.01);
    assert.equal(Number(pathEnd[2]), geometry.arrow.base.y);
    assert.equal(JSON.stringify(points), original, 'rendering must not modify endpoint coordinates');
    const noHead = reactionEndpointGeometry(points, { node, arrow: false });
    assert.equal(noHead.arrow, null);
    assert.ok(Math.abs(Number(noHead.d.match(/L ([\d.-]+)/)[1]) - (100 - radius - 4)) < 0.01);
  }
});

test('curved branches keep their Bézier shape while clipping away the node-end stretch', () => {
  const points = [{ x: 0, y: 0 }, { x: 30, y: 60 }, { x: 80, y: 60 }, { x: 100, y: 0 }];
  const original = JSON.stringify(points);
  const geometry = reactionEndpointGeometry(points, { node: { kind: 'metabolite' }, arrow: true });
  assert.match(geometry.d, /^M 0 0 C /);
  assert.ok(Math.abs(Math.hypot(geometry.arrow.tip.x - 100, geometry.arrow.tip.y) - 17) < 1e-6);
  assert.ok(Math.abs(Math.hypot(geometry.arrow.tip.x - geometry.arrow.base.x, geometry.arrow.tip.y - geometry.arrow.base.y) - 17.8125) < 1e-6);
  assert.ok(geometry.arrow.tip.y > 0, 'the tip follows the edited curve, rather than a straight chord');
  assert.equal(JSON.stringify(points), original);
});

test('boundary arrows also have no shaft beyond the tip, and collapsed branches do not draw inside nodes', () => {
  const geometry = reactionEndpointGeometry([{ x: 0, y: 0 }, { x: 100, y: 0 }], { arrow: true });
  assert.ok(Math.abs(geometry.arrow.tip.x - 98) < 1e-6);
  assert.ok(geometry.arrow.base.x < 98);
  const collapsed = reactionEndpointGeometry([{ x: 96, y: 0 }, { x: 100, y: 0 }], { node: { kind: 'currency' }, arrow: true });
  assert.equal(collapsed.d, '');
  assert.equal(collapsed.arrow, null);
});
