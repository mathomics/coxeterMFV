/**
 * A genome-scale model that somebody else wrote, uploaded and mapped by hand.
 *
 * The bundled core models are written for this tool: their compartments are
 * named, nested in a chain and use only letters the registry knows. A draft
 * reconstruction exported from cobrapy is none of those things, and everything
 * here is a way that difference showed on the map.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../src/core/model.js';
import { CommunityModel } from '../src/core/community.js';
import {
  buildCompartmentTree, compartmentName, cellCompartmentName, cellCompartmentVisualId,
  compartmentVisualId, membraneStyle,
} from '../src/core/compartments.js';
import { prepareImportedModels } from '../src/core/model-import.js';
import { EditableMap } from '../src/map/editable-map.js';
import { reactionBoundaryKeys } from '../src/map/boundaries.js';
import { parseMapDocument, validateMapContext, renameMapSpecies } from '../src/map/map-document.js';
import { primaryMetabolitesForSide } from '../src/ui/add-reaction-panel.js';
import { Store } from '../src/ui/store.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/*
 * An alga in miniature, shaped like a cobrapy export of an iRC1080 descendant:
 * every compartment is named after its own id, `s` is the eyespot, `i` the
 * mitochondrial intermembrane space, and there is a flagellum beside the cell.
 */
function algaDocument({ id = 'sbml_model', big = 0 } = {}) {
  const compartments = ['c', 'e', 'f', 'g', 'h', 'i', 'm', 's', 'u'];
  const met = (mid, compartment, name = mid) => ({ id: mid, name, compartment, formula: '', charge: 0 });
  const rxn = (rid, metabolites, extra = {}) => ({
    id: rid, name: rid, metabolites, lower_bound: -1000, upper_bound: 1000, ...extra,
  });
  const metabolites = [
    met('g6p_c', 'c'), met('f6p_c', 'c'), met('atp_c', 'c'), met('adp_c', 'c'),
    met('g3p_c', 'c'), met('g3p_h', 'h'), met('pi_c', 'c'), met('pi_h', 'h'),
    met('h_i', 'i'), met('h_m', 'm'), met('atp_m', 'm'), met('adp_m', 'm'), met('pi_m', 'm'),
    met('retinol_s', 's'), met('dhretinol_s', 's'),
    met('udpgal_c', 'c'), met('udpgal_g', 'g'),
    met('tub_f', 'f'), met('tubp_f', 'f'),
    met('pq_u', 'u'), met('pqh2_u', 'u'),
    met('glc_e', 'e'),
  ];
  const reactions = [
    rxn('PGI', { g6p_c: -1, f6p_c: 1 }),
    rxn('TPT', { g3p_h: -1, pi_c: -1, g3p_c: 1, pi_h: 1 }),
    rxn('ATPSm', { adp_m: -1, pi_m: -1, h_i: -4, atp_m: 1, h_m: 4 }),
    rxn('RETSAT', { retinol_s: -1, dhretinol_s: 1 }),
    rxn('UDPGALtg', { udpgal_c: -1, udpgal_g: 1 }),
    rxn('TUBK', { tub_f: -1, tubp_f: 1 }),
    rxn('PQR', { pq_u: -1, pqh2_u: 1 }),
    rxn('GLCt', { glc_e: -1, g6p_c: 1 }),
  ];
  // Padding for the chloroplast, to make it a great deal larger than the Golgi.
  for (let k = 0; k < big; k += 1) {
    metabolites.push(met(`x${k}_h`, 'h'), met(`y${k}_h`, 'h'));
    reactions.push(rxn(`HR${k}`, { [`x${k}_h`]: -1, [`y${k}_h`]: 1 }));
  }
  return {
    id,
    name: 'A small alga',
    compartments: Object.fromEntries(compartments.map((c) => [c, c])),
    metabolites,
    reactions,
    genes: [],
  };
}

function alga(options) {
  const model = new Model(algaDocument(options));
  const community = new CommunityModel({
    species: [{ id: model.id, model, abundance: 1, enabled: true }], medium: {},
  });
  return { model, community };
}

// Compartments

test('the matrix is inside the intermembrane space, whichever way that is spelt', () => {
  const { model } = alga();
  const tree = model.compartmentTree();
  assert.equal(tree.byId.get('i').parent, 'c');
  assert.equal(tree.byId.get('m').parent, 'i', 'the matrix sits inside `i`');
  assert.equal(tree.byId.get('s').parent, 'c', 'a cell`s own `s` is an organelle, not a root');
  assert.deepEqual(tree.roots.map((r) => r.id), ['e', 'f']);

  // The other spelling, and neither: a model that declares only one still nests.
  assert.equal(buildCompartmentTree(['e', 'c', 'im', 'm']).byId.get('m').parent, 'im');
  assert.equal(buildCompartmentTree(['e', 'c', 'im', 'm']).byId.get('im').parent, 'c');
  assert.equal(buildCompartmentTree(['e', 'c', 'm']).byId.get('m').parent, 'c');
});

test('ATP synthase crosses the one membrane it sits in', () => {
  // With `i` drawn beside the cell it was reported as crossing four.
  const { community } = alga();
  assert.deepEqual(reactionBoundaryKeys(community, 'sbml_model@ATPSm'), ['sbml_model/m']);
});

test('the shared medium keeps the letter it shares with the eyespot', () => {
  const { community } = alga();
  // The merged model is the one that really contains the medium, and says so.
  const tree = community.merged.compartmentTree();
  assert.equal(tree.byId.get('s').parent, null);
  assert.equal(compartmentName('s', { s: 'Shared medium' }), 'Shared medium');
  assert.equal(compartmentVisualId('s'), 's');
  assert.equal(membraneStyle('s'), 'open');

  assert.equal(cellCompartmentVisualId('s', { s: 's' }), 'es');
  assert.equal(cellCompartmentName('s', { s: 's' }), 'Eyespot');
  assert.equal(membraneStyle(cellCompartmentVisualId('s', { s: 's' })), 'bilayer');
  assert.equal(cellCompartmentVisualId('s', community.merged.compartmentNames), 's');
});

test('a compartment named after its own id is given its real name', () => {
  const names = { c: 'c', h: 'h', i: 'i', u: '', x: 'Glyoxysome' };
  assert.equal(compartmentName('c', names), 'Cytosol');
  assert.equal(compartmentName('h', names), 'Chloroplast stroma');
  assert.equal(compartmentName('i', names), 'Intermembrane space');
  assert.equal(compartmentName('u', names), 'Thylakoid lumen');
  assert.equal(compartmentName('x', names), 'Glyoxysome', 'a name the model does supply is kept');
  assert.equal(compartmentName('zz', { zz: 'zz' }), 'zz', 'and an unknown letter stays as it is');
});

test('the drawn shell names its compartments and nests them', () => {
  const { community } = alga();
  const map = EditableMap.emptyFromCommunity(community);
  const comp = (id) => map.cells[0].compartments.find((c) => c.id === id);

  assert.equal(comp('c').label, 'Cytosol');
  assert.equal(comp('s').label, 'Eyespot');
  assert.equal(comp('s').membrane, 'bilayer');
  assert.equal(comp('s').visualId, 'es');
  assert.equal(comp('i').visualId, 'im');
  assert.equal(comp('m').parentId, 'i');

  const inside = (a, b) => a.x >= b.x && a.y >= b.y && a.x + a.w <= b.x + b.w && a.y + a.h <= b.y + b.h;
  assert.ok(inside(comp('m').rect, comp('i').rect));
  assert.ok(inside(comp('i').rect, comp('c').rect));
  assert.ok(inside(comp('s').rect, comp('c').rect));
});

test('sibling compartments get room in proportion to what they hold', () => {
  const { community } = alga({ big: 120 });
  const map = EditableMap.emptyFromCommunity(community);
  const area = (id) => {
    const r = map.cells[0].compartments.find((c) => c.id === id).rect;
    return r.w * r.h;
  };
  // 240 metabolites against one: from equal grid cells the Golgi came out three
  // quarters the size of the chloroplast.
  assert.ok(area('h') > area('g') * 8, `chloroplast ${Math.round(area('h'))} vs Golgi ${Math.round(area('g'))}`);
  // A flagellum declared beside the cell took half of it.
  assert.ok(area('e') > area('f') * 8, `cell ${Math.round(area('e'))} vs flagellum ${Math.round(area('f'))}`);

  for (const c of map.cells[0].compartments) {
    assert.ok(c.rect.w >= 60 && c.rect.h >= 60, `${c.id} is still a box something fits in`);
    const aspect = Math.max(c.rect.w / c.rect.h, c.rect.h / c.rect.w);
    if (c.parentId !== null) assert.ok(aspect < 3.2, `${c.id} is not a sliver (${aspect.toFixed(1)}:1)`);
  }
  // Siblings do not overlap.
  const siblings = map.cells[0].compartments.filter((c) => c.parentId === 'c');
  for (const a of siblings) {
    for (const b of siblings) {
      if (a === b) continue;
      const apart = a.rect.x + a.rect.w <= b.rect.x || b.rect.x + b.rect.w <= a.rect.x
        || a.rect.y + a.rect.h <= b.rect.y || b.rect.y + b.rect.h <= a.rect.y;
      assert.ok(apart, `${a.id} and ${b.id} overlap`);
    }
  }
});

// Eyespot is not medium

test('a reaction in the cell`s own `s` is drawn in that compartment, not in the medium', () => {
  const { community } = alga();
  const map = EditableMap.emptyFromCommunity(community);
  const placed = map.addReaction(community, 'sbml_model@RETSAT');
  assert.ok(placed);

  const box = map.cells[0].compartments.find((c) => c.id === 's').rect;
  const within = (p) => p.x > box.x && p.x < box.x + box.w && p.y > box.y && p.y < box.y + box.h;
  assert.ok(within(placed.reaction), 'the marker is inside the eyespot');
  for (const id of placed.nodes) {
    const node = map.nodes.get(id);
    assert.equal(node.kind, 'metabolite', `${node.base} is an ordinary metabolite`);
    assert.equal(node.cellId, 'sbml_model');
    assert.ok(within(node), `${node.base} is inside the eyespot`);
  }
  assert.deepEqual(map.medium.nodes, [], 'nothing was registered with the shared medium');

  // And it can be dragged about in there, rather than being held out of every cell.
  const node = map.nodes.get(placed.nodes[0]);
  const moved = map.moveNode(node.id, box.x + box.w / 2, box.y + box.h / 2);
  assert.ok(within(moved));
});

test('a real shared-medium metabolite is still a medium node with no cell', async () => {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const community = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }], medium: {},
  });
  const map = EditableMap.emptyFromCommunity(community);
  const transfer = [...community.reactionOwner].find(([, owner]) => owner.role === 'transfer')[0];
  const placed = map.addReaction(community, transfer);
  const medium = placed.nodes.map((id) => map.nodes.get(id)).find((n) => n.kind === 'medium');
  assert.ok(medium, 'the transfer has an end in the shared medium');
  assert.equal(medium.cellId, null);
  assert.ok(map.medium.nodes.includes(medium.id));
  assert.doesNotMatch(medium.base, /@/);
});

// Labels

test('a metabolite is labelled with its own id, not the model`s', () => {
  const { community } = alga();
  const map = EditableMap.emptyFromCommunity(community);
  map.addReaction(community, 'sbml_model@PGI');
  map.addReaction(community, 'sbml_model@TPT');
  map.addMetabolite(community, 'sbml_model@udpgal_c', { x: 400, y: 300 });

  const labels = [...map.nodes.values()].map((n) => n.base);
  assert.ok(labels.length >= 7);
  for (const label of labels) assert.doesNotMatch(label, /@/, `${label} carries the model name`);
  assert.ok(labels.includes('g6p') && labels.includes('udpgal'));

  // The merged model knows the plain name too, which is what keeps the cofactor
  // list from being bypassed by `sbml_model@atp`.
  assert.equal(community.merged.metabolites.get('sbml_model@atp_c').base, 'atp');
  assert.equal(community.merged.metabolites.get('sbml_model@g6p_c').base, 'g6p');
});

test('the automatic layout labels cofactors without the model name too', async () => {
  const raw = JSON.parse(await readFile(join(root, 'models', 'ecoli_core_mc.json'), 'utf8'));
  const community = new CommunityModel({
    species: [{ id: 'eco', model: new Model(raw), abundance: 1 }], medium: {},
  });
  const map = EditableMap.fromCommunity(community);
  const cofactors = [...map.nodes.values()].filter((n) => n.kind === 'currency');
  assert.ok(cofactors.length > 50);
  for (const node of cofactors) assert.doesNotMatch(node.base, /@/);

  // Add Reaction leaves cofactors out of the metabolites to draw towards.
  const store = { community };
  const offered = primaryMetabolitesForSide(store, 'eco@PFK', 'product').map((m) => m.label);
  assert.deepEqual(offered, ['fdp_c']);
});

test('a map saved with the model name in its labels is mended when it is opened', () => {
  const { community } = alga();
  const map = EditableMap.emptyFromCommunity(community);
  map.addReaction(community, 'sbml_model@PGI');
  const doc = JSON.parse(JSON.stringify(map.toJSON()));
  for (const node of doc.nodes) node.base = `sbml_model@${node.base}`;
  // And the bare letters the compartments were stored with.
  for (const comp of doc.cells[0].compartments) { comp.label = comp.id; delete comp.visualId; }
  doc.cells[0].compartments.find((c) => c.id === 's').membrane = 'open';

  const loaded = EditableMap.fromJSON(doc);
  assert.deepEqual([...loaded.nodes.values()].map((n) => n.base).sort(), ['f6p', 'g6p']);
  const comp = (id) => loaded.cells[0].compartments.find((c) => c.id === id);
  assert.equal(comp('c').label, 'Cytosol');
  assert.equal(comp('s').label, 'Eyespot');
  assert.equal(comp('s').membrane, 'bilayer');
});

// Maps across versions

async function uploadedStore(document, filename) {
  const store = new Store('.');
  await store.importModelFiles([{ name: filename, text: JSON.stringify(document) }]);
  return store;
}

/** The same, starting from the empty shell a model too large to draw opens on. */
async function handBuiltStore(document, filename) {
  const store = await uploadedStore(document, filename);
  store.map = EditableMap.emptyFromCommunity(store.community);
  store.mapHistory.clear();
  return store;
}

test('a map still opens after the model file is renamed', async () => {
  const store = await handBuiltStore(algaDocument(), 'alga_stage16.json');
  assert.ok(store.addMapReaction('sbml_model@PGI'));
  const saved = store.mapJSON();

  // The same model, exported again under the next version's file name.
  const next = await uploadedStore(algaDocument(), 'alga_stage17.json');
  await next.loadMapDocument(saved);
  assert.ok(next.map.reactions.has('sbml_model@PGI'));

  // Bundled models are still held to the model they were drawn over.
  assert.throws(() => validateMapContext(
    { metadata: { species: [{ id: 'eco', model: 'ecoli_core_mc' }] } },
    { species: [{ id: 'eco', file: 'yeast_core_mc', enabled: true }] },
  ), /different model/);
});

test('a map follows its model when the model comes back under another id', async () => {
  const first = await handBuiltStore(algaDocument({ id: 'iAlga_stage15' }), 'alga.json');
  assert.ok(first.addMapReaction('iAlga_stage15@PGI'));
  assert.ok(first.addMapReaction('iAlga_stage15@TPT'));
  assert.ok(first.setMapReactionLinear('iAlga_stage15@PGI', true));
  const saved = first.mapJSON();
  assert.match(saved, /iAlga_stage15@PGI/);

  const second = await uploadedStore(algaDocument({ id: 'sbml_model' }), 'alga.json');
  await second.loadMapDocument(saved);

  assert.deepEqual([...second.map.reactions.keys()].sort(), ['sbml_model@PGI', 'sbml_model@TPT']);
  assert.equal(second.map.cells[0].id, 'sbml_model');
  const text = JSON.stringify(second.map.toJSON());
  assert.doesNotMatch(text, /iAlga_stage15/, 'no identifier still names the old model');
  assert.match(second.mapLoadNote, /iAlga_stage15.*sbml_model/);
  // The line a straight reaction is kept on still names nodes that exist.
  const linear = second.map.reactions.get('sbml_model@PGI').linear;
  assert.ok(second.map.nodes.has(linear.from) && second.map.nodes.has(linear.to));
  // The transporter is still on the membrane it was put on.
  assert.equal(second.map.reactions.get('sbml_model@TPT').boundaryId, 'sbml_model/h');
});

test('a map of a genuinely different model is not drawn over it, and says which reaction', async () => {
  const first = await handBuiltStore(algaDocument({ id: 'iAlga_stage15' }), 'alga.json');
  assert.ok(first.addMapReaction('iAlga_stage15@RETSAT'));
  const saved = first.mapJSON();

  const other = algaDocument({ id: 'sbml_model' });
  other.reactions = other.reactions.filter((r) => r.id !== 'RETSAT');
  const second = await uploadedStore(other, 'alga.json');
  const open = JSON.stringify(second.map.toJSON());
  // Kept for the model it belongs to, and drawn when that one is loaded.
  assert.equal(await second.loadMapDocument(saved), null);
  assert.match(second.waitingMap.reason, /sbml_model@RETSAT does not exist.*iAlga_stage15/);
  assert.equal(JSON.stringify(second.map.toJSON()), open);
});

test('renaming a species leaves names and free text alone', () => {
  const parsed = parseMapDocument({
    format: 'EscherMCMCMap', version: 3,
    metadata: { scenario: 'uploaded-models', species: [{ id: 'old', model: 'upload:a' }] },
    map: {
      cells: [{ id: 'old', name: 'old@name stays', compartments: [] }],
      nodes: [{ id: 'met:old@R:old@a_c', mergedId: 'old@a_c', cellId: 'old', name: 'old@ in a name', base: 'old@a' }],
      reactions: [{ id: 'old@R', cellId: 'old', boundaryId: 'old/h', equation: 'old@a_c <=> old@b_c', name: 'old@R name' }],
      segments: [{ reactionId: 'old@R', nodeId: 'met:old@R:old@a_c' }],
      texts: [{ id: 'text-1', text: 'old@note', x: 0, y: 0 }],
    },
  });
  const renamed = renameMapSpecies(parsed, 'old', 'new');
  assert.equal(renamed.map.nodes[0].id, 'met:new@R:new@a_c');
  assert.equal(renamed.map.reactions[0].boundaryId, 'new/h');
  assert.equal(renamed.map.reactions[0].equation, 'new@a_c <=> new@b_c');
  assert.equal(renamed.map.cells[0].name, 'old@name stays');
  assert.equal(renamed.map.nodes[0].name, 'old@ in a name');
  assert.equal(renamed.map.texts[0].text, 'old@note');
  // An id that merely ends the same way is another species.
  const other = renameMapSpecies(parseMapDocument({
    format: 'EscherMCMCMap', version: 3, metadata: null,
    map: { cells: [], nodes: [{ id: 'met:bold@R:bold@a_c' }], reactions: [], segments: [] },
  }), 'old', 'new');
  assert.equal(other.map.nodes[0].id, 'met:bold@R:bold@a_c');
});

test('a saved map can be opened onto a blank canvas', async () => {
  const store = await handBuiltStore(algaDocument(), 'alga.json');
  assert.ok(store.addMapReaction('sbml_model@PGI'));
  const saved = store.mapJSON();

  // Clear drawing leaves a canvas with no cells, and a map must still open on
  // it rather than be turned away as "created for a different set of cells".
  store.showEmptyMap();
  assert.equal(store.map.cells.length, 0);
  await store.loadMapDocument(saved);
  assert.ok(store.map.reactions.has('sbml_model@PGI'));
  assert.equal(store.map.cells.length, 1);
});

// Maps saved by earlier builds

/*
 * The nesting earlier builds read off this same model: `i` and `s` as boxes of
 * their own beside the cell, and the matrix straight in the cytosol.
 */
const OLD_TREE = { e: null, f: null, i: null, s: null, c: 'e', g: 'c', h: 'c', m: 'c', u: 'h' };

/**
 * A map as one of those builds saved it: the old nesting, the model's name in
 * front of every label, compartments labelled with their letters, and the
 * reaction of the cell's own `s` compartment drawn out in the shared medium
 * with its metabolites filed as medium nodes.
 */
function mapFromBefore({ reactions = ['PGI', 'TPT', 'ATPSm', 'RETSAT'], without = [] } = {}) {
  const model = new Model({ ...algaDocument(), compartment_tree: OLD_TREE });
  const community = new CommunityModel({
    species: [{ id: model.id, model, abundance: 1, enabled: true }], medium: {},
  });
  const map = EditableMap.emptyFromCommunity(community);
  for (const id of reactions) assert.ok(map.addReaction(community, `sbml_model@${id}`), `${id} is drawn`);
  const doc = JSON.parse(JSON.stringify(map.toJSON()));

  const stray = 'sbml_model@RETSAT';
  const rxn = doc.reactions.find((r) => r.id === stray);
  if (rxn) {
    const to = { x: doc.environment.rect.x + 70, y: doc.environment.rect.y + 70 };
    const dx = to.x - rxn.x;
    const dy = to.y - rxn.y;
    const shift = (p) => { if (p) { p.x += dx; p.y += dy; } };
    shift(rxn); shift(rxn.axis.a1); shift(rxn.axis.a2);
    doc.medium.nodes = [];
    for (const node of doc.nodes) {
      if (node.reactionLocalFor !== stray && node.reactionId !== stray) continue;
      shift(node);
      if (node.kind === 'metabolite') { node.kind = 'medium'; doc.medium.nodes.push(node.id); }
    }
    for (const seg of doc.segments) {
      if (seg.reactionId === stray) { shift(seg.from); shift(seg.to); shift(seg.b1); shift(seg.b2); }
    }
  }
  for (const node of doc.nodes) node.base = `sbml_model@${node.base}`;
  doc.cells[0].compartments = doc.cells[0].compartments.filter((c) => !without.includes(c.id));
  for (const comp of doc.cells[0].compartments) {
    comp.label = comp.id;
    delete comp.visualId;
    if (comp.id === 's') comp.membrane = 'open';
  }
  return {
    doc,
    text: JSON.stringify({
      format: 'EscherMCMCMap',
      version: 3,
      metadata: { scenario: 'uploaded-models', species: [{ id: 'sbml_model', model: 'upload:alga' }] },
      pathwayManager: null,
      fluxColorStops: null,
      map: doc,
    }),
  };
}

const rectOf = (map, id) => map.cells[0].compartments.find((c) => c.id === id)?.rect;
const within = (inner, outer) => inner.x >= outer.x && inner.y >= outer.y
  && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h;
const inBox = (p, r) => p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h;

test('a map drawn before opens with its compartments where the model nests them', async () => {
  const before = mapFromBefore();
  assert.deepEqual(
    before.doc.cells[0].compartments.filter((c) => c.parentId === null).map((c) => c.id).sort(),
    ['e', 'f', 'i', 's'], 'the file really has the old nesting',
  );
  const matrixWas = before.doc.cells[0].compartments.find((c) => c.id === 'm').rect;

  const store = await uploadedStore(algaDocument(), 'alga.json');
  await store.loadMapDocument(before.text);
  const map = store.map;

  const tree = store.models.get(store.species[0].file).compartmentTree();
  for (const comp of map.cells[0].compartments) {
    assert.equal(comp.parentId ?? null, tree.byId.get(comp.id).parent ?? null, `${comp.id} is nested as the model has it`);
    if (comp.parentId) assert.ok(within(comp.rect, rectOf(map, comp.parentId)), `${comp.id} is drawn inside ${comp.parentId}`);
  }

  // The intermembrane space takes the matrix's place and the matrix is drawn in
  // a little, so nothing beside them had to move. (A matrix as small as this
  // one cannot give up the whole band, and the space takes the rest outward.)
  const i = rectOf(map, 'i');
  const m = rectOf(map, 'm');
  assert.ok(within(matrixWas, i), 'the space covers where the matrix stood');
  assert.ok(i.w * i.h < matrixWas.w * matrixWas.h * 1.2, 'and hardly more than that');
  assert.ok(m.w < matrixWas.w && m.h < matrixWas.h, 'the matrix is drawn in');
  for (const band of [m.x - i.x, m.y - i.y, i.x + i.w - (m.x + m.w), i.y + i.h - (m.y + m.h)]) {
    assert.ok(band >= 44, `a metabolite fits between the two membranes (${band})`);
  }
  assert.match(store.mapLoadNote, /i around m/);
  assert.match(store.mapLoadNote, /s into c/);
});

test('what was drawn in the right place stays exactly where it was put', async () => {
  const before = mapFromBefore();
  const store = await uploadedStore(algaDocument(), 'alga.json');
  await store.loadMapDocument(before.text);

  for (const rid of ['sbml_model@PGI', 'sbml_model@TPT']) {
    const was = before.doc.reactions.find((r) => r.id === rid);
    const now = store.map.reactions.get(rid);
    assert.deepEqual([now.x, now.y], [was.x, was.y], `${rid} has not moved`);
    for (const node of before.doc.nodes.filter((n) => (n.reactionLocalFor ?? n.reactionId) === rid)) {
      const after = store.map.nodes.get(node.id);
      assert.deepEqual([after.x, after.y], [node.x, node.y], `${node.id} has not moved`);
    }
  }
  // Labels and names are mended on the way, as for any earlier file.
  for (const node of store.map.nodes.values()) assert.doesNotMatch(node.base, /@/);
  assert.equal(store.map.cells[0].compartments.find((c) => c.id === 'c').label, 'Cytosol');
});

test('what the old nesting had put in the wrong place is drawn where it belongs', async () => {
  const before = mapFromBefore();
  const store = await uploadedStore(algaDocument(), 'alga.json');
  await store.loadMapDocument(before.text);
  const map = store.map;

  // The eyespot reaction is in the eyespot, as ordinary metabolites.
  const retsat = map.reactions.get('sbml_model@RETSAT');
  assert.ok(inBox(retsat, rectOf(map, 's')), 'the reaction is inside the eyespot');
  const eyespot = [...map.nodes.values()].filter((n) => n.compartment === 's');
  assert.equal(eyespot.length, 2);
  for (const node of eyespot) {
    assert.equal(node.kind, 'metabolite');
    assert.ok(inBox(node, rectOf(map, 's')), `${node.base} is inside the eyespot`);
  }
  assert.deepEqual(map.medium.nodes, []);
  assert.ok(![...map.nodes.values()].some((n) => n.kind === 'medium'));

  // ATP synthase is on the membrane it crosses, with its protons on either side.
  const atps = map.reactions.get('sbml_model@ATPSm');
  assert.equal(atps.boundaryId, 'sbml_model/m');
  const proton = [...map.nodes.values()].find((n) => n.mergedId === 'sbml_model@h_i');
  assert.ok(inBox(proton, rectOf(map, 'i')) && !inBox(proton, rectOf(map, 'm')),
    'the proton outside the matrix is in the intermembrane space');
  for (const node of map.nodes.values()) {
    if (node.compartment === 'm') assert.ok(inBox(node, rectOf(map, 'm')), `${node.base} is in the matrix`);
  }
  // Every curve still ends on the metabolite it is drawn to.
  for (const seg of map.segments.filter((x) => x.nodeId)) {
    const node = map.nodes.get(seg.nodeId);
    const end = seg.side === 'substrate' ? seg.from : seg.to;
    assert.ok(node && Math.hypot(end.x - node.x, end.y - node.y) < 1e-6);
  }
});

test('opening an old map is one undo step, and saving it again leaves nothing to mend', async () => {
  const before = mapFromBefore();
  const store = await uploadedStore(algaDocument(), 'alga.json');
  const full = store.map.reactions.size;
  await store.loadMapDocument(before.text);
  assert.equal(store.mapHistory.undoStack.length, 1);

  const mended = store.mapJSON();
  const shell = JSON.stringify(store.map.toJSON());
  await store.loadMapDocument(mended);
  assert.equal(store.mapLoadNote, '', 'a map saved now needs nothing done to it');
  assert.equal(JSON.stringify(store.map.toJSON()), shell);

  store.undoMap();
  store.undoMap();
  assert.equal(store.map.reactions.size, full, 'undo brings back the map that was open before');
});

test('a compartment the model has put in between is drawn if the map never had it', async () => {
  // Cell, cytosol and matrix drawn by hand under the old nesting; no `i` at all.
  const before = mapFromBefore({ reactions: ['PGI'], without: ['i', 's', 'f'] });
  const store = await uploadedStore(algaDocument(), 'alga.json');
  await store.loadMapDocument(before.text);
  const map = store.map;

  const i = map.cells[0].compartments.find((c) => c.id === 'i');
  assert.ok(i, 'the intermembrane space is drawn');
  assert.equal(i.parentId, 'c');
  assert.equal(i.label, 'Intermembrane space');
  assert.equal(map.cells[0].compartments.find((c) => c.id === 'm').parentId, 'i');
  assert.ok(within(rectOf(map, 'm'), i.rect));
  assert.ok(map.boundary('sbml_model/i'), 'and it has a membrane to put reactions on');

  // The reaction that needs both can now be added and lands where it should.
  const placed = store.addMapReaction('sbml_model@ATPSm');
  assert.equal(placed.reaction.boundaryId, 'sbml_model/m');
});

test('a map that needs nothing comes through untouched', async () => {
  const store = await handBuiltStore(algaDocument(), 'alga.json');
  for (const id of ['PGI', 'TPT', 'ATPSm', 'RETSAT']) assert.ok(store.addMapReaction(`sbml_model@${id}`));
  const node = [...store.map.nodes.values()][0];
  store.moveNode(node.id, node.x + 31, node.y - 17);
  const saved = store.mapJSON();
  const drawn = JSON.stringify(store.map.toJSON());

  const next = await uploadedStore(algaDocument(), 'alga.json');
  await next.loadMapDocument(saved);
  assert.equal(JSON.stringify(next.map.toJSON()), drawn);
  assert.equal(next.mapLoadNote, '');
});

test('reactions the model has since lost are left out and the rest of the map is kept', async () => {
  const first = await handBuiltStore(algaDocument(), 'alga.json');
  for (const id of ['PGI', 'TPT', 'RETSAT', 'PQR']) assert.ok(first.addMapReaction(`sbml_model@${id}`));
  const saved = first.mapJSON();
  const tpt = first.map.reactions.get('sbml_model@TPT');
  const pqr = first.map.reactions.get('sbml_model@PQR');

  // The next export of the model: one reaction gone, one metabolite renamed,
  // one reaction given another metabolite.
  const next = algaDocument();
  next.reactions = next.reactions.filter((r) => r.id !== 'RETSAT');
  next.metabolites.find((m) => m.id === 'f6p_c').id = 'f6p_B_c';
  const pgi = next.reactions.find((r) => r.id === 'PGI');
  pgi.metabolites = { g6p_c: -1, f6p_B_c: 1 };
  next.reactions.find((r) => r.id === 'PQR').metabolites.atp_c = -1;

  const second = await uploadedStore(next, 'alga.json');
  await second.loadMapDocument(saved);
  const map = second.map;

  assert.deepEqual([...map.reactions.keys()].sort(),
    ['sbml_model@PGI', 'sbml_model@PQR', 'sbml_model@TPT']);
  assert.ok(![...map.nodes.values()].some((n) => n.compartment === 's'), 'the lost reaction took its metabolites with it');
  // The reaction whose metabolite was renamed is drawn again, with the new one.
  assert.ok([...map.nodes.values()].some((n) => n.mergedId === 'sbml_model@f6p_B_c'));
  assert.ok(![...map.nodes.values()].some((n) => n.mergedId === 'sbml_model@f6p_c'));
  // The ones the change did not reach are where they were.
  assert.deepEqual([map.reactions.get('sbml_model@TPT').x, map.reactions.get('sbml_model@TPT').y], [tpt.x, tpt.y]);
  assert.deepEqual([map.reactions.get('sbml_model@PQR').x, map.reactions.get('sbml_model@PQR').y], [pqr.x, pqr.y]);

  assert.match(second.mapLoadNote, /1 reaction the model no longer has was left out \(RETSAT\)/);
  assert.match(second.mapLoadNote, /1 drawn afresh because the model has changed their metabolites \(PGI\)/);
  assert.match(second.mapLoadNote, /1 still drawn with metabolites the model has since changed \(PQR\)/);
  for (const seg of map.segments) assert.ok(map.reactions.has(seg.reactionId), 'no curve is left without its reaction');
});

test('a map that shares nothing with the open model is not drawn over it', async () => {
  const first = await handBuiltStore(algaDocument(), 'alga.json');
  for (const id of ['PGI', 'TPT']) assert.ok(first.addMapReaction(`sbml_model@${id}`));
  const saved = first.mapJSON();

  const other = algaDocument();
  for (const rxn of other.reactions) rxn.id = `X_${rxn.id}`;
  const second = await uploadedStore(other, 'alga.json');
  const open = JSON.stringify(second.map.toJSON());
  assert.equal(await second.loadMapDocument(saved), null, 'it waits for its own model');
  assert.match(second.waitingMap.reason, /None of the 2 reactions on this map exist in the active model/);
  assert.equal(JSON.stringify(second.map.toJSON()), open, 'and the map that was open is still open');
});

// Layout presets

test('a layout preset does not lay a hand-built map of a large model out again', async () => {
  const store = await uploadedStore(algaDocument({ big: 400 }), 'alga.json');
  assert.equal(store.map.reactions.size, 0, 'too large to draw automatically: the shell is empty');
  store.addMapReaction('sbml_model@PGI');
  const node = [...store.map.nodes.values()][0];
  store.moveNode(node.id, node.x + 77, node.y + 33);
  const placed = { x: node.x, y: node.y };

  assert.match(store.relayoutBlocked(), /hand-built drawing/);
  store.setLayoutPreset('spacious');

  const after = store.map.nodes.get(node.id);
  assert.deepEqual({ x: after.x, y: after.y }, placed, 'what was placed by hand is where it was put');
  assert.match(store.status.message, /The drawing was kept/);
});

test('a preset on a large model with nothing drawn lays out the shell alone', async () => {
  const store = await uploadedStore(algaDocument({ big: 400 }), 'alga.json');
  assert.equal(store.map.reactions.size, 0);
  const width = store.map.width;
  // The full layout of a model this size is what the upload itself avoids.
  const original = EditableMap.fromCommunity;
  EditableMap.fromCommunity = () => { throw new Error('the full layout was run'); };
  try {
    store.setLayoutPreset('compact');
  } finally {
    EditableMap.fromCommunity = original;
  }
  assert.ok(store.map.width < width, 'the shell was laid out again, smaller');
  assert.equal(store.map.reactions.size, 0);
  assert.equal(store.map.cells.length, 1);
});

test('a small model is still laid out again by a preset, and one undo brings the old map back', async () => {
  const store = await uploadedStore(algaDocument(), 'alga.json');
  assert.equal(store.relayoutBlocked(), null);
  const width = store.map.width;
  const before = JSON.stringify(store.map.toJSON());
  store.setLayoutPreset('compact');
  assert.ok(store.map.width < width, 'the compact preset draws a smaller map');
  store.undoMap();
  assert.ok(JSON.stringify(store.map.toJSON()) === before, 'undo restores the map as it was');
});

// The import

test('an upload is named and counted as it was written', () => {
  const prepared = prepareImportedModels([{ name: 'alga.json', text: JSON.stringify(algaDocument()) }]);
  const entry = prepared.entries[0];
  assert.equal(entry.speciesId, 'sbml_model');
  assert.equal(entry.modelKey, 'upload:alga');
  assert.equal(entry.summary.compartments, 9);
});
