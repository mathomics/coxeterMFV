import test from 'node:test';
import assert from 'node:assert/strict';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import { EditableMap } from '../src/map/editable-map.js';
import { renderMap, frameLabelScale, exportFrameLabelScale } from '../src/render/mapview.js';

/*
 * The names of compartments, cells and the medium.
 *
 * Each is a pill with its text, and the two are sized together. Sized
 * separately (the pill once, for the text at its base size, and the text again
 * by the zoom), a name at the fitted view of a large map comes out several
 * times wider than the pill it is centred in, across the membrane on either
 * side of it.
 */

function cell({ tree = null } = {}) {
  const met = (id, compartment) => ({ id, name: id, compartment, formula: '', charge: 0 });
  const model = new Model({
    id: 'alga',
    name: 'A cell with a long descriptive name',
    compartments: { e: 'e', c: 'c', h: 'h', u: 'u', i: 'i', m: 'm', g: 'g' },
    ...(tree ? { compartment_tree: tree } : {}),
    metabolites: [met('a_e', 'e'), met('a_c', 'c'), met('a_h', 'h'), met('a_u', 'u'), met('a_i', 'i'), met('a_m', 'm'), met('a_g', 'g')],
    reactions: [{ id: 'T', name: 'T', metabolites: { a_e: -1, a_c: 1 }, lower_bound: -1000, upper_bound: 1000 }],
  });
  return new CommunityModel({ species: [{ id: 'alga', model, abundance: 1 }], medium: {} });
}

const render = (map, state = {}) => renderMap(map, {
  fluxes: new Map(), scale: { color: () => '#94a3b8', width: () => 1, reference: 1 }, ...state,
});

/** Every name plate in a rendering: where it is anchored, its pill, its text and its limit. */
function plates(svg) {
  const re = /<g class="emc-comp-label[^"]*" data-compartment-label="([^"]+)" transform="translate\(([-\d.]+) ([-\d.]+)\)">\s*<g class="emc-frame-label" style="--fit:([\d.]+)">\s*<rect x="([-\d.]+)" y="([-\d.]+)" width="([-\d.]+)" height="([-\d.]+)"[^>]*\/>\s*<text x="([-\d.]+)" y="([-\d.]+)" text-anchor="middle" textLength="([-\d.]+)"[^>]*>([^<]*)<\/text>/g;
  return [...svg.matchAll(re)].map((m) => ({
    id: m[1], x: +m[2], y: +m[3], fit: +m[4],
    pill: { x: +m[5], y: +m[6], w: +m[7], h: +m[8] }, textAt: +m[9], textLength: +m[11], text: m[12],
  }));
}

/** Where a plate is drawn on the map at a given zoom. */
function drawn(plate, zoom) {
  const scale = Math.min(frameLabelScale(12 / zoom), plate.fit);
  return {
    scale,
    x: plate.x + plate.pill.x * scale,
    y: plate.y + plate.pill.y * scale,
    w: plate.pill.w * scale,
    h: plate.pill.h * scale,
  };
}

const ZOOMS = [2, 1, 0.5, 0.2, 0.1, 0.05];

test('every compartment and the medium is named, each text cut to its own pill', () => {
  const map = EditableMap.emptyFromCommunity(cell());
  const found = plates(render(map));
  assert.deepEqual(found.map((p) => p.id).sort(), ['c', 'e', 'g', 'h', 'i', 'm', 's', 'u']);
  for (const plate of found) {
    // The text is pinned to a width, and the pill is that width and its padding.
    assert.ok(plate.textLength > 0);
    assert.equal(Math.round(plate.pill.w - plate.textLength), 18, `${plate.text}: the pill is cut to its text`);
    assert.ok(Math.abs(plate.textAt - (plate.pill.x + plate.pill.w / 2)) < 0.01, `${plate.text}: and the text is centred in it`);
  }
  // Text and pill are one group scaled as a whole; neither is sized by the zoom on its own.
  assert.match(render(map), /class="emc-frame-label" style="--fit:/);
});

test('a name plate stays inside its own compartment at every zoom', () => {
  const map = EditableMap.emptyFromCommunity(cell());
  const found = plates(render(map));
  const rectOf = (id) => (id === 's' ? map.environment.rect : map.cells[0].compartments.find((c) => c.id === id).rect);
  for (const plate of found) {
    const r = rectOf(plate.id);
    for (const zoom of ZOOMS) {
      const box = drawn(plate, zoom);
      assert.ok(box.x >= r.x - 0.01 && box.x + box.w <= r.x + r.w + 0.01,
        `${plate.text} at ${zoom * 100}%: ${Math.round(box.w)} wide in a compartment ${Math.round(r.w)} wide`);
    }
  }
});

test('plates are all one size on screen until a compartment is too small for that', () => {
  const map = EditableMap.emptyFromCommunity(cell());
  const found = plates(render(map));
  // At an ordinary working zoom nothing is cramped: every plate is drawn at the same scale.
  const at = (zoom) => found.map((plate) => drawn(plate, zoom).scale);
  assert.equal(new Set(at(1).map((v) => v.toFixed(3))).size, 1);
  assert.equal(new Set(at(0.8).map((v) => v.toFixed(3))).size, 1);
  // Further out the smaller compartments of this small cell start to hold theirs back.
  // Zoomed out they grow to keep their size on screen...
  assert.ok(Math.max(...at(0.1)) > Math.max(...at(1)) * 2);
  // ...each only as far as it has room, never past the others.
  for (const scale of at(0.05)) assert.ok(scale <= frameLabelScale(12 / 0.05) + 1e-9);
  assert.equal(frameLabelScale(12 / 2), 1, 'zoomed in, a plate is its base size and no smaller');
});

test('a plate keeps out of the compartments nested in its own', () => {
  const map = EditableMap.emptyFromCommunity(cell());
  const found = plates(render(map));
  const comps = map.cells[0].compartments;
  for (const plate of found.filter((p) => p.id !== 's')) {
    const comp = comps.find((c) => c.id === plate.id);
    const round = ['m', 'u', 'g', 'i'].includes(plate.id);
    if (round) continue;   // these sit astride their own top edge, see below
    for (const child of comps.filter((c) => c.parentId === comp.id)) {
      for (const zoom of ZOOMS) {
        const box = drawn(plate, zoom);
        const apart = box.x + box.w <= child.rect.x || child.rect.x + child.rect.w <= box.x
          || box.y + box.h <= child.rect.y || child.rect.y + child.rect.h <= box.y;
        assert.ok(apart, `${plate.text} runs into ${child.id} at ${zoom * 100}%`);
      }
    }
  }
});

test('a compartment is named even when one nested in it reaches its top edge', () => {
  // The cytosol keeps its name when an organelle stands within a plate's
  // height of its top, wherever along the top that organelle is.
  const map = EditableMap.emptyFromCommunity(cell());
  const comps = map.cells[0].compartments;
  const cytosol = comps.find((c) => c.id === 'c');
  const chloroplast = comps.find((c) => c.id === 'h');
  map.editGeometryRect('alga', 'h', { ...chloroplast.rect, y: cytosol.rect.y + 10 });
  assert.ok(chloroplast.rect.y - cytosol.rect.y < 27);
  assert.ok(plates(render(map)).some((p) => p.id === 'c' && p.text === 'Cytosol'));
});

test('two plates astride closely nested top edges share the band between them', () => {
  const map = EditableMap.emptyFromCommunity(cell());
  const found = plates(render(map));
  const comps = map.cells[0].compartments;
  const band = comps.find((c) => c.id === 'm').rect.y - comps.find((c) => c.id === 'i').rect.y;
  const outer = found.find((p) => p.id === 'i');
  const inner = found.find((p) => p.id === 'm');
  for (const zoom of ZOOMS) {
    const a = drawn(outer, zoom);
    const b = drawn(inner, zoom);
    assert.ok(a.y + a.h <= b.y + 0.01, `the two are written over each other at ${zoom * 100}%`);
    assert.ok((a.h + b.h) / 2 <= band + 0.01);
  }
});

test('the cell is named on a plate as wide as its name, held to the cell and the room above it', () => {
  const map = EditableMap.emptyFromCommunity(cell());
  const svg = render(map);
  const m = /data-cell-plate="alga" transform="translate\(([-\d.]+) ([-\d.]+)\)">\s*<g class="emc-frame-label" style="--fit:([\d.]+)">\s*<rect x="0" y="-30" width="([-\d.]+)" height="30"/.exec(svg);
  assert.ok(m, 'the plate is drawn');
  const [x, y, fit, width] = m.slice(1).map(Number);
  const r = map.cells[0].rect;
  assert.deepEqual([x, y], [Math.round(r.x * 100) / 100, Math.round((r.y - 10) * 100) / 100]);
  assert.ok(width * fit <= r.w + 0.01, 'never wider than the cell');
  assert.ok(30 * fit <= r.y - map.environment.rect.y, 'never taller than the medium above the cell');
  assert.match(svg, /class="emc-cell-name"[^>]*textLength=/);
});

test('an exported picture names its cells and compartments as large as the fitted view does', () => {
  const map = EditableMap.emptyFromCommunity(cell());
  // A map several screens wide, as a community map is.
  map.setCanvasSize(map.width * 4, map.height * 4);
  const k = exportFrameLabelScale(map);
  const zoom = Math.min(1160 / map.width, 760 / map.height);
  assert.equal(k, Number(frameLabelScale(12 / zoom).toFixed(3)));
  assert.ok(k > 3, `a large map's names are enlarged (${k}×)`);

  const exported = render(map, { forExport: true });
  // Browsers scale the plates by the stylesheet, from the root...
  assert.match(exported, new RegExp(`^<svg [^>]*style="--frame-label-scale:${k}"`));
  // ...and every plate states its scale too, for editors that do not read var() or min().
  const groups = [...exported.matchAll(/<g class="emc-frame-label" style="--fit:([\d.]+)" transform="scale\(([\d.]+)\)">/g)];
  assert.equal(groups.length, plates(render(map)).length + map.cells.length, 'every compartment, the medium and the cell');
  for (const [, fit, scale] of groups) assert.equal(Number(scale), Math.min(k, Number(fit)));

  // On screen the camera sets the scale; nothing is fixed in the markup.
  const screen = render(map);
  assert.doesNotMatch(screen, /emc-frame-label"[^>]*transform=/);
  assert.doesNotMatch(screen.slice(0, screen.indexOf('>')), /--frame-label-scale/);
  // A small map is exported with its names near their base size.
  assert.equal(exportFrameLabelScale({ width: 900, height: 600 }), 1);
});
