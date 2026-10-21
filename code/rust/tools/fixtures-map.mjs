// Reference outputs of src/layout and src/map for the Rust port's parity tests.
//
//   node rust/tools/fixtures-map.mjs [section ...]
//
// Writes rust/fixtures/map/ (small, committed) and rust/fixtures/large/map/
// (genome-scale or bulky, git-ignored). Sections: layouts, maps, edits,
// reconcile, documents, overview, yeast9, units. With no argument, all of them.
import { readFile, writeFile, mkdir, readdir, stat } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, join, basename } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { homedir } from 'node:os';

globalThis.crypto ??= (await import('node:crypto')).webcrypto;

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const outSmall = join(root, 'rust/fixtures/map');
const outLarge = join(root, 'rust/fixtures/large/map');
await mkdir(outSmall, { recursive: true });
await mkdir(outLarge, { recursive: true });

const { Model } = await import(pathToFileURL(join(root, 'src/core/model.js')).href);
const { CommunityModel } = await import(pathToFileURL(join(root, 'src/core/community.js')).href);
const { layoutCommunity } = await import(pathToFileURL(join(root, 'src/layout/layout.js')).href);
const { LAYOUT_PRESETS, layoutEngineOptions, layoutPreset } = await import(pathToFileURL(join(root, 'src/layout/layout-settings.js')).href);
const { EditableMap } = await import(pathToFileURL(join(root, 'src/map/editable-map.js')).href);

const sections = new Set(process.argv.slice(2));
const want = (name) => sections.size === 0 || sections.has(name);

/** Small files go to fixtures/map, big ones to fixtures/large/map. */
const LARGE = 400 * 1024;
async function write(name, value, { large = false, small = false } = {}) {
  const text = JSON.stringify(value);
  const dir = !small && (large || text.length > LARGE) ? outLarge : outSmall;
  await writeFile(join(dir, name), text);
  return dir === outLarge ? `large/map/${name}` : `map/${name}`;
}

// Communities

const models = new Map();
for (const f of (await readdir(join(root, 'models'))).filter((f) => f.endsWith('_mc.json'))) {
  models.set(f.replace(/\.json$/, ''), new Model(JSON.parse(await readFile(join(root, 'models', f), 'utf8'))));
}
const { scenarios } = JSON.parse(await readFile(join(root, 'models/scenarios.json'), 'utf8'));

/** The community of a bundled scenario, built as rust/tools/fixtures-core.mjs builds it. */
function scenarioCommunity(s) {
  const medium = {};
  for (const [base, limit] of Object.entries(s.medium ?? {})) medium[base] = { lower: -Math.abs(Number(limit) || 0), upper: 1000 };
  return new CommunityModel({
    name: s.name, objective: s.objective || 'equal-growth', productTarget: s.productTarget || null, medium,
    species: s.species.map((sp) => ({ id: sp.id, model: models.get(sp.model), abundance: sp.abundance ?? 1 / s.species.length, enabled: sp.enabled !== false })),
  });
}
const bundled = scenarios.filter((s) => !s.loader);
const communities = new Map(bundled.map((s) => [s.id, scenarioCommunity(s)]));

/** Each section lists what it wrote in its own index-<section>.json. */
async function writeIndex(section, entries) {
  await writeFile(join(outSmall, `index-${section}.json`), JSON.stringify(entries, null, 1));
  console.log(`${section}: ${entries.length ?? Object.keys(entries).length} entries`);
}

// Layouts

/** The full layouts kept in the repository; every other one is written to large/. */
const COMMITTED_LAYOUTS = new Set(['solo-ecoli-default', 'solo-levadura-compact', 'metanogenesis-spacious']);

if (want('layouts')) {
  const entries = [];
  const settings = [
    ['default', {}],
    ...Object.keys(LAYOUT_PRESETS).map((name) => [name, layoutEngineOptions(layoutPreset(name))]),
  ];
  for (const s of bundled) {
    const community = communities.get(s.id);
    for (const [name, opts] of settings) {
      const map = EditableMap.fromCommunity(community, opts);
      const file = await write(`layout-${s.id}-${name}.json`, map.toJSON(), { large: !COMMITTED_LAYOUTS.has(`${s.id}-${name}`) });
      entries.push({ scenario: s.id, settings: name, options: opts, file });
    }
    const empty = EditableMap.emptyFromCommunity(community);
    const file = await write(`layout-${s.id}-none.json`, empty.toJSON());
    entries.push({ scenario: s.id, settings: 'none', options: { contents: 'none' }, file });
  }
  // The four-E. coli community, at the default settings, is the biggest map
  // the bundled files make. It is what the performance numbers are taken on.
  await writeIndex('layouts', entries);
}

// Maps

const { parseMapDocument } = await import(pathToFileURL(join(root, 'src/map/map-document.js')).href);

/** Every saved map the tests read: the gut9 snapshots and the user's own. */
async function savedMaps() {
  const out = [];
  const gut = join(root, 'docs/gut9-map/maps');
  for (const f of (await readdir(gut)).sort()) out.push({ path: join(gut, f), large: false });
  const downloads = join(homedir(), 'Downloads');
  if (existsSync(downloads)) {
    for (const f of (await readdir(downloads)).sort()) {
      if (f.includes('coxeter-map') && f.endsWith('.json')) out.push({ path: join(downloads, f), large: true });
    }
  }
  return out;
}

/** Where a test finds an input: under the repository, or under $HOME. */
const inputRef = (path, file) => (path.startsWith(root)
  ? { input: path.slice(root.length + 1), home: false, file }
  : { input: path.slice(homedir().length + 1), home: true, file });

const slug = (path) => basename(path, '.json').replace(/[^A-Za-z0-9_.-]+/g, '_');

if (want('maps')) {
  const entries = [];
  for (const { path, large } of await savedMaps()) {
    let parsed;
    try { parsed = parseMapDocument(await readFile(path, 'utf8')); } catch { continue; } // not a map
    if (!Array.isArray(parsed.map?.nodes)) continue;
    const map = EditableMap.fromJSON(parsed.map);
    const file = await write(`roundtrip-${slug(path)}.json`, map.toJSON(), { large: large || path.includes('gut9') });
    entries.push(inputRef(path, file));
  }
  await writeIndex('maps', entries);
}

// Edits

/**
 * Apply one recorded edit to a live map. The ops and their arguments are the
 * EditableMap methods and their JS arguments. The Rust test replays the same
 * list. Returns [map, summary]: some ops hand back a new map.
 */
function applyEdit(map, community, op, a) {
  const yes = (v) => (v === null || v === undefined ? null : true);
  const sel = (s) => ({ nodes: new Set(s.nodes ?? []), reactions: new Set(s.reactions ?? []), texts: new Set(s.texts ?? []) });
  switch (op) {
    case 'addReaction': {
      const r = map.addReaction(community, a.id, a.opts ?? {});
      return [map, r ? { nodes: r.nodes, segments: r.segments.length, anchorNode: r.anchorNode?.id ?? null } : null];
    }
    case 'removeReaction': {
      const r = map.removeReaction(a.id);
      return [map, r ? { nodes: r.nodes } : null];
    }
    case 'moveNode': return [map, yes(map.moveNode(a.id, a.x, a.y))];
    case 'moveReaction': return [map, yes(map.moveReaction(a.id, a.x, a.y))];
    case 'rotateReaction': return [map, yes(map.rotateReaction(a.id, a.angle))];
    case 'setReactionAngle': return [map, yes(map.setReactionAngle(a.id, a.angle))];
    case 'flipReaction': return [map, yes(map.flipReaction(a.id))];
    case 'mirrorReaction': return [map, yes(map.mirrorReaction(a.id))];
    case 'alignReaction': return [map, yes(map.alignReaction(a.id, a.direction, a.ends ?? {}))];
    case 'tidyReaction': return [map, yes(map.tidyReaction(a.id))];
    case 'setReactionLinear': return [map, yes(map.setReactionLinear(a.id, a.on, a.ends ?? {}))];
    case 'setReactionLineEnd': return [map, yes(map.setReactionLineEnd(a.id, a.nodeId))];
    case 'reactionLineEnds': return [map, map.reactionLineEnds(a.id)];
    case 'swapReactionMetabolites': return [map, yes(map.swapReactionMetabolites(a.id, a.a, a.b))];
    case 'moveBranchControl': return [map, map.moveBranchControl(a.id, a.nodeId, a.handle, a.x, a.y)];
    case 'reactionControlPoints': return [map, map.reactionControlPoints(a.id)];
    case 'canRotateReaction': return [map, map.canRotateReaction(a.id)];
    case 'canLinearizeReaction': return [map, map.canLinearizeReaction(a.id)];
    case 'reactionAngle': return [map, map.reactionAngle(a.id)];
    case 'reactionSides': {
      const s = map.reactionSides(a.id);
      const strip = (l) => l.map(({ nodeId, label, group }) => ({ nodeId, label, group }));
      return [map, { substrates: strip(s.substrates), products: strip(s.products) }];
    }
    case 'selectableInRect': {
      const s = map.selectableInRect(a.rect, a.options ?? {});
      return [map, { nodes: [...s.nodes], reactions: [...s.reactions], texts: [...(s.texts ?? [])] }];
    }
    case 'translateSelection': {
      const r = map.translateSelection(sel(a.selection), a.dx, a.dy);
      return [map, r ? { nodes: r.nodes, reactions: r.reactions, texts: r.texts ?? [] } : null];
    }
    case 'alignSelection': return [map, map.alignSelection(sel(a.selection), a.mode)];
    case 'distributeSelection': return [map, map.distributeSelection(sel(a.selection), a.axis)];
    case 'alignSelectionInLine': return [map, map.alignSelectionInLine(sel(a.selection), a.mode)];
    case 'editGeometryRect': return [map, map.editGeometryRect(a.cellId, a.compartmentId, a.rect)];
    case 'setCanvasSize': return [map, map.setCanvasSize(a.width, a.height, a.move ?? {})];
    case 'addCell': {
      const r = map.addCell(a.spec);
      return [map, r ? { cell: r.cell.id, compartment: r.compartment.id, boundary: r.boundary?.id ?? null } : null];
    }
    case 'addCompartment': {
      const r = map.addCompartment(a.spec);
      return [map, r ? { cell: r.cell.id, compartment: r.compartment.id } : null];
    }
    case 'addText': return [map, map.addText(a.text, a.x, a.y, a.opts ?? {})];
    case 'updateText': return [map, yes(map.updateText(a.id, a.text))];
    case 'styleText': return [map, yes(map.styleText(a.id, a.style))];
    case 'moveText': return [map, yes(map.moveText(a.id, a.x, a.y))];
    case 'removeText': return [map, yes(map.removeText(a.id))];
    case 'duplicateMetaboliteNode': return [map, map.duplicateMetaboliteNode(a.id, a.opts ?? {})?.id ?? null];
    case 'mergeMetaboliteNodes': {
      const r = map.mergeMetaboliteNodes(a.source, a.target);
      return [map, r ? { rewired: r.rewired } : null];
    }
    case 'mergeOverlappingMetabolite': {
      const r = map.mergeOverlappingMetabolite(a.id);
      return [map, r ? { rewired: r.rewired, target: r.target.id } : null];
    }
    case 'overlappingMetaboliteTarget': return [map, map.overlappingMetaboliteTarget(a.id)?.id ?? null];
    case 'addMetabolite': return [map, map.addMetabolite(community, a.mergedId, { x: a.x, y: a.y })?.id ?? null];
    case 'setNodePrimary': return [map, yes(map.setNodePrimary(a.id, a.primary))];
    case 'setNodeRadius': return [map, yes(map.setNodeRadius(a.id, a.radius))];
    case 'setNodeHidden': return [map, yes(map.setNodeHidden(a.id, a.hidden))];
    case 'showAllHiddenNodes': return [map, map.showAllHiddenNodes()];
    case 'setMetaboliteLabelAngle': return [map, yes(map.setMetaboliteLabelAngle(a.id, a.angle))];
    case 'setCellColors': return [map, yes(map.setCellColors(a.cellId, a.colors))];
    case 'setCompartmentColors': return [map, yes(map.setCompartmentColors(a.cellId, a.compartmentId, a.colors))];
    case 'setCompartmentLabelPosition': return [map, map.setCompartmentLabelPosition(a.cellId, a.compartmentId, a.x, a.y)];
    case 'removeNode': return [map, yes(map.removeNode(a.id))];
    case 'stats': return [map, map.stats()];
    case 'boundaries': return [map, map.boundaries];
    case 'minimumCanvasSize': return [map, map.minimumCanvasSize()];
    case 'reload': return [EditableMap.fromJSON(JSON.parse(JSON.stringify(map.toJSON()))), null];
    case 'subsetByReactions': return [map.subsetByReactions(a.ids), null];
    case 'cleared': return [map.cleared(), null];
    default: throw new Error(`unknown edit op ${op}`);
  }
}

/**
 * What changed between two map documents, small enough to keep one per step:
 * whole top-level fields that differ, and for nodes, reactions and texts the
 * entries that differ (plus the order of ids when that changed), and segments by
 * index. `full` carries a whole map. The Rust test applies these in turn.
 */
function mapDelta(prev, next) {
  if (!prev) return { full: next };
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const d = {};
  for (const k of ['version', 'canvas', 'environment', 'cells', 'medium', 'labelSettings']) {
    if (!same(prev[k], next[k])) d[k] = next[k] ?? null;
  }
  for (const k of ['nodes', 'reactions', 'texts']) {
    const before = new Map(prev[k].map((x) => [x.id, JSON.stringify(x)]));
    const set = next[k].filter((x) => before.get(x.id) !== JSON.stringify(x));
    const order = next[k].map((x) => x.id);
    const sameOrder = same(order, prev[k].map((x) => x.id));
    if (set.length || !sameOrder) d[k] = { set, ...(sameOrder ? {} : { order }) };
  }
  const set = {};
  next.segments.forEach((seg, i) => { if (!same(prev.segments[i], seg)) set[i] = seg; });
  if (Object.keys(set).length || prev.segments.length !== next.segments.length) {
    d.segments = { length: next.segments.length, set };
  }
  return d;
}

/** Run a script of edits, recording what the map became after every step. */
function recordEdits(map, community, build) {
  const steps = [];
  let previous = null;
  const run = (op, args = {}) => {
    const [next, result] = applyEdit(map, community, op, args);
    map = next;
    const now = map.toJSON();
    steps.push({ op, args, result: JSON.parse(JSON.stringify(result ?? null)), delta: mapDelta(previous, now) });
    previous = now;
    return result;
  };
  build({ run, map: () => map });
  return steps;
}

/** The node a reaction drew for a metabolite, by merged id. */
function nodeFor(map, reactionId, mergedId) {
  for (const seg of map.segments) {
    if (seg.reactionId !== reactionId || !seg.nodeId) continue;
    const node = map.nodes.get(seg.nodeId);
    if (node?.mergedId === mergedId) return node.id;
  }
  return null;
}

function branchesOf(map, reactionId) {
  return map.segments.filter((s) => s.reactionId === reactionId && s.nodeId && !['axis', 'boundary', 'transport-lane'].includes(s.kind));
}

/** Sequence A: a map drawn by hand on the shell of the E. coli scenario. */
function handDrawnScript({ run, map }) {
  const m = () => map();
  run('addReaction', { id: 'eco@PGI' });
  run('addReaction', { id: 'eco@PFK', opts: { anchorNodeId: nodeFor(m(), 'eco@PGI', 'eco@f6p_c'), placementDirection: { x: 1, y: 0 } } });
  run('addReaction', { id: 'eco@FBA', opts: { anchorNodeId: nodeFor(m(), 'eco@PFK', 'eco@fdp_c'), placementDirection: { x: 0, y: 1 } } });
  run('addReaction', { id: 'eco@TPI', opts: { anchorNodeId: nodeFor(m(), 'eco@FBA', 'eco@dhap_c'), placementDirection: { x: -1, y: 0 } } });
  run('addReaction', { id: 'eco@GAPD', opts: { anchorNodeId: nodeFor(m(), 'eco@FBA', 'eco@g3p_c'), placementDirection: { x: 0, y: -1 }, terminalMetaboliteId: 'eco@13dpg_c' } });
  run('addReaction', { id: 'eco@PGK', opts: { anchorNodeId: nodeFor(m(), 'eco@GAPD', 'eco@13dpg_c'), placementDirection: { x: 0.6, y: 0.8 } } });
  run('addReaction', { id: 'eco@ENO', opts: { anchor: { x: 300, y: 300 } } });
  run('addReaction', { id: 'eco@GLCptspp' });
  run('addReaction', { id: 'eco@ACt2rpp' });
  run('addReaction', { id: 'eco@PIt2rpp', opts: { anchor: { x: 200, y: 900 } } });
  run('addReaction', { id: 'eco@GLCtex' });
  run('addReaction', { id: 'eco@EX_glc__D_e' });
  run('addReaction', { id: 'EX_glc__D_s' });
  run('addReaction', { id: 'eco@NADH16pp' });
  run('addReaction', { id: 'eco@PGI' }); // already drawn: refused
  run('stats');
  run('boundaries');

  // Dragging metabolites, cofactors and medium nodes.
  const g6p = nodeFor(m(), 'eco@PGI', 'eco@g6p_c');
  run('moveNode', { id: g6p, x: 420, y: 260 });
  run('moveNode', { id: g6p, x: -500, y: 40000 });
  const atp = nodeFor(m(), 'eco@PFK', 'eco@atp_c');
  run('moveNode', { id: atp, x: 777, y: 333 });
  const glcS = nodeFor(m(), 'EX_glc__D_s', 'M@glc__D');
  if (glcS) run('moveNode', { id: glcS, x: 30, y: 30 });

  // Reactions: internal, transporter, exchange.
  run('moveReaction', { id: 'eco@PGI', x: 480, y: 380 });
  run('moveReaction', { id: 'eco@GLCptspp', x: 10, y: 600 });
  run('moveReaction', { id: 'eco@ACt2rpp', x: 900, y: 50 });
  run('moveReaction', { id: 'EX_glc__D_s', x: 5000, y: 10 });
  run('reactionAngle', { id: 'eco@PGI' });
  run('canRotateReaction', { id: 'eco@PGI' });
  run('rotateReaction', { id: 'eco@PGI', angle: 0.7 });
  run('setReactionAngle', { id: 'eco@ENO', angle: -2.4 });
  run('flipReaction', { id: 'eco@PFK' });
  run('mirrorReaction', { id: 'eco@FBA' });
  run('mirrorReaction', { id: 'eco@ACt2rpp' });
  run('flipReaction', { id: 'eco@ACt2rpp' });
  run('reactionSides', { id: 'eco@GAPD' });
  run('alignReaction', { id: 'eco@GAPD', direction: 'E' });
  run('alignReaction', { id: 'eco@PGK', direction: 'SW' });
  run('moveNode', { id: nodeFor(m(), 'eco@PGK', 'eco@3pg_c'), x: 650, y: 700 });
  run('moveReaction', { id: 'eco@PGK', x: 0, y: 0 });
  run('canLinearizeReaction', { id: 'eco@TPI' });
  run('setReactionLinear', { id: 'eco@TPI', on: true });
  run('reactionLineEnds', { id: 'eco@TPI' });
  run('moveNode', { id: nodeFor(m(), 'eco@FBA', 'eco@dhap_c'), x: 250, y: 520 });
  run('setReactionLinear', { id: 'eco@TPI', on: false });
  const fbaSubs = branchesOf(m(), 'eco@FBA').filter((s) => s.side === 'product').map((s) => s.nodeId);
  if (fbaSubs.length >= 2) run('swapReactionMetabolites', { id: 'eco@FBA', a: fbaSubs[0], b: fbaSubs[1] });
  run('setReactionLineEnd', { id: 'eco@FBA', nodeId: fbaSubs[1] ?? fbaSubs[0] });
  run('tidyReaction', { id: 'eco@FBA' });
  const pgiBranch = branchesOf(m(), 'eco@PGI')[0];
  run('moveBranchControl', { id: 'eco@PGI', nodeId: pgiBranch.nodeId, handle: 'b1', x: 333, y: 222 });
  run('moveBranchControl', { id: 'eco@PGI', nodeId: pgiBranch.nodeId, handle: 'anchor', x: 470, y: 370 });
  run('moveBranchControl', { id: 'eco@PGI', nodeId: pgiBranch.nodeId, handle: 'b2', x: 471, y: 371 });
  run('reactionControlPoints', { id: 'eco@ENO' });

  // Copies of a metabolite, and merging them back.
  const f6p = nodeFor(m(), 'eco@PGI', 'eco@f6p_c');
  const copy = run('duplicateMetaboliteNode', { id: f6p, opts: { offset: 60 } });
  run('duplicateMetaboliteNode', { id: copy });
  run('moveNode', { id: copy, x: m().nodes.get(f6p).x + 3, y: m().nodes.get(f6p).y + 4 });
  run('overlappingMetaboliteTarget', { id: copy });
  run('mergeOverlappingMetabolite', { id: copy });
  const pep = nodeFor(m(), 'eco@ENO', 'eco@pep_c');
  const manual = run('addMetabolite', { mergedId: 'eco@pep_c', x: 610, y: 410 });
  if (pep && manual) run('mergeMetaboliteNodes', { source: pep, target: manual });
  run('setNodePrimary', { id: manual, primary: false });
  run('setNodeRadius', { id: manual, radius: 12 });
  run('setNodeHidden', { id: manual, hidden: true });
  run('showAllHiddenNodes');
  run('setMetaboliteLabelAngle', { id: manual, angle: 7.5 });

  // Selections.
  run('selectableInRect', { rect: { x: 0, y: 0, w: 700, h: 600 } });
  const box = (() => {
    const s = m().selectableInRect({ x: 0, y: 0, w: 700, h: 600 });
    return { nodes: [...s.nodes], reactions: [...s.reactions], texts: [...s.texts] };
  })();
  run('translateSelection', { selection: box, dx: 25, dy: -15 });
  run('alignSelection', { selection: box, mode: 'left' });
  run('distributeSelection', { selection: box, axis: 'vertical' });
  run('alignSelection', { selection: { reactions: ['eco@PGI', 'eco@ENO', 'eco@GAPD'] }, mode: 'horizontal' });
  run('alignSelectionInLine', { selection: { reactions: ['eco@PGI', 'eco@PFK', 'eco@FBA'] }, mode: 'vertical' });
  run('alignSelectionInLine', { selection: { reactions: ['eco@GAPD', 'eco@PGK'], nodes: [manual] }, mode: 'horizontal' });

  // Texts.
  const t1 = run('addText', { text: '  Glycolysis ', x: 120, y: 80, opts: { fontSize: 31.6, fontFamily: 'serif' } })?.id;
  run('addText', { text: 'Second', x: 300, y: 90 });
  run('updateText', { id: t1, text: 'Upper glycolysis' });
  run('styleText', { id: t1, style: { fontSize: 200 } });
  run('moveText', { id: t1, x: 140, y: 60 });
  run('translateSelection', { selection: { texts: [t1, 'text-2'], nodes: [manual] }, dx: 5, dy: 5 });
  run('alignSelection', { selection: { texts: [t1, 'text-2'] }, mode: 'bottom' });
  run('removeText', { id: 'text-2' });

  // Cells, compartments and the canvas.
  run('setCellColors', { cellId: 'eco', colors: { fill: '#A0B0C0', stroke: 'nope' } });
  run('setCellColors', { cellId: 'eco', colors: { fill: null } });
  run('setCompartmentColors', { cellId: 'eco', compartmentId: 'c', colors: { stroke: '#123456' } });
  run('setCompartmentLabelPosition', { cellId: 'eco', compartmentId: 'p', x: 0, y: 0 });
  run('setCompartmentLabelPosition', { cellId: null, compartmentId: 's', x: 99999, y: 50 });
  const c = m().cells[0].compartments.find((k) => k.id === 'c').rect;
  run('editGeometryRect', { cellId: 'eco', compartmentId: 'c', rect: { x: c.x + 40, y: c.y + 30, w: c.w, h: c.h } });
  run('editGeometryRect', { cellId: 'eco', compartmentId: 'c', rect: { x: c.x, y: c.y, w: c.w * 0.6, h: c.h * 0.7 } });
  const e = m().cells[0].compartments.find((k) => k.id === 'e').rect;
  run('editGeometryRect', { cellId: 'eco', compartmentId: 'e', rect: { x: e.x - 20, y: e.y + 10, w: e.w, h: e.h } });
  run('editGeometryRect', { cellId: 'eco', compartmentId: 'e', rect: { x: e.x, y: e.y, w: e.w + 100, h: e.h - 50 } });
  const env = m().environment.rect;
  run('editGeometryRect', { cellId: null, compartmentId: 's', rect: { x: env.x + 10, y: env.y, w: env.w, h: env.h } });
  run('editGeometryRect', { cellId: null, compartmentId: 's', rect: { x: env.x, y: env.y, w: env.w + 200, h: env.h - 100 } });
  run('minimumCanvasSize');
  run('setCanvasSize', { width: 100, height: 100 });
  run('setCanvasSize', { width: m().width + 300, height: m().height + 100, move: { shiftX: 40, shiftY: 20 } });
  run('addCell', { spec: { speciesId: 'new', outerLabel: 'outside' } });
  run('addCell', { spec: { speciesId: 'drawn', rect: { x: 3000, y: 50, w: -400, h: 300 }, outerCompartmentId: 'e', color: '#ff0000' } });
  run('addCompartment', { spec: { speciesId: 'drawn', compartmentId: 'c', parentId: 'e', label: 'cytoplasm' } });
  run('addCompartment', { spec: { speciesId: 'drawn', compartmentId: 'm', parentId: 'c', rect: { x: 0, y: 0, w: 50, h: 50 } } });
  run('addCompartment', { spec: { speciesId: 'drawn', compartmentId: 'x', parentId: 'c' } });
  run('addCompartment', { spec: { speciesId: 'fresh', compartmentId: 'e' } });
  run('addCompartment', { spec: { speciesId: 'fresh', compartmentId: 'w' } });
  run('boundaries');

  // Saving and opening again, then editing what was opened.
  run('reload');
  run('moveReaction', { id: 'EX_glc__D_s', x: 100, y: 100 });
  run('moveReaction', { id: 'eco@GAPD', x: 520, y: 640 });
  run('rotateReaction', { id: 'eco@ENO', angle: -1 });
  run('flipReaction', { id: 'eco@PIt2rpp' });

  // Taking things off the map.
  run('removeReaction', { id: 'eco@TPI' });
  run('removeReaction', { id: 'eco@ACt2rpp' });
  run('removeNode', { id: manual });
  run('addReaction', { id: 'eco@ACt2rpp', opts: { boundaryId: 'eco/c' } });
  run('subsetByReactions', { ids: ['eco@PGI', 'eco@PFK', 'eco@ACt2rpp', 'EX_glc__D_s'] });
  run('moveNode', { id: nodeFor(m(), 'eco@PGI', 'eco@f6p_c'), x: 500, y: 500 });
  run('cleared');
  run('stats');
}

/** Sequence B: edits of a full automatic layout, which shares points the way the layout drew them. */
function layoutScript({ run, map }) {
  const m = () => map();
  const reactions = [...m().reactions.values()];
  const transporter = reactions.find((r) => r.onMembrane && r.kind !== 'transfer');
  const transfer = reactions.find((r) => r.kind === 'transfer');
  const exchange = reactions.find((r) => r.kind === 'exchange' && r.boundary);
  const internal = reactions.filter((r) => !r.onMembrane && r.cellId);
  run('moveReaction', { id: transporter.id, x: transporter.x + 30, y: transporter.y + 20 });
  run('moveReaction', { id: transporter.id, x: transporter.x - 10, y: transporter.y + 5 });
  run('moveReaction', { id: transfer.id, x: transfer.x + 12, y: transfer.y - 9 });
  run('moveReaction', { id: exchange.id, x: exchange.x + 40, y: exchange.y + 40 });
  run('moveReaction', { id: exchange.id, x: exchange.x - 15, y: exchange.y });
  run('rotateReaction', { id: internal[0].id, angle: 1.1 });
  run('flipReaction', { id: internal[1].id });
  run('mirrorReaction', { id: internal[2].id });
  run('mirrorReaction', { id: transporter.id });
  run('tidyReaction', { id: internal[3].id });
  run('tidyReaction', { id: transporter.id });
  run('alignReaction', { id: internal[4].id, direction: 'NE' });
  run('setReactionLinear', { id: internal[5].id, on: true });
  run('moveReaction', { id: internal[5].id, x: internal[5].x + 50, y: internal[5].y });
  const medium = [...m().nodes.values()].find((n) => n.kind === 'medium');
  run('moveNode', { id: medium.id, x: medium.x + 300, y: medium.y - 200 });
  const cur = [...m().nodes.values()].find((n) => n.kind === 'currency');
  run('moveNode', { id: cur.id, x: cur.x + 9, y: cur.y + 9 });
  const cell = m().cells[0];
  const outer = cell.compartments.find((c) => c.parentId === null);
  run('editGeometryRect', { cellId: cell.id, compartmentId: outer.id, rect: { ...outer.rect, x: outer.rect.x + 25, y: outer.rect.y - 15 } });
  run('moveReaction', { id: internal[6].id, x: internal[6].x + 33, y: internal[6].y + 11 });
  run('moveReaction', { id: transporter.id, x: transporter.x + 5, y: transporter.y + 5 });
  const inner = cell.compartments.find((c) => c.parentId !== null && cell.compartments.every((o) => o.parentId !== c.id));
  run('editGeometryRect', { cellId: cell.id, compartmentId: inner.id, rect: { ...inner.rect, w: inner.rect.w * 0.8 } });
  const s = m().selectableInRect({ x: cell.rect.x, y: cell.rect.y, w: cell.rect.w / 2, h: cell.rect.h / 2 });
  const selection = { nodes: [...s.nodes], reactions: [...s.reactions], texts: [...s.texts] };
  run('translateSelection', { selection, dx: -12, dy: 18 });
  run('alignSelection', { selection, mode: 'top' });
  run('removeReaction', { id: internal[7].id });
  run('addReaction', { id: internal[7].id, opts: { anchor: { x: internal[7].x, y: internal[7].y } } });
  run('reload');
  run('moveReaction', { id: exchange.id, x: exchange.x, y: exchange.y });
  run('stats');
}

/** Sequence C: edits that need no model, on whatever a saved map holds. */
function savedMapScript({ run, map }) {
  const m = () => map();
  const every = (list, n) => list.filter((_, i) => i % Math.max(1, Math.floor(list.length / n)) === 0).slice(0, n);
  const reactions = () => [...m().reactions.values()];
  run('stats');
  run('boundaries');
  for (const r of every(reactions(), 12)) run('moveReaction', { id: r.id, x: r.x + 21, y: r.y - 13 });
  const nodes = every([...m().nodes.values()], 12);
  for (const n of nodes) run('moveNode', { id: n.id, x: n.x - 17, y: n.y + 9 });
  const internal = () => reactions().filter((r) => !r.onMembrane && r.axis);
  for (const r of every(internal(), 4)) run('rotateReaction', { id: r.id, angle: 0.6 });
  for (const r of every(internal(), 5).slice(1)) run('flipReaction', { id: r.id });
  for (const r of every(reactions().filter((x) => x.axis), 6).slice(1)) run('mirrorReaction', { id: r.id });
  for (const r of every(internal(), 7).slice(1, 5)) run('tidyReaction', { id: r.id });
  for (const [i, r] of every(internal(), 9).slice(2, 6).entries()) run('alignReaction', { id: r.id, direction: ['N', 'SE', 'W', 'NE'][i] });
  for (const r of every(internal(), 11).slice(3, 6)) run('setReactionLinear', { id: r.id, on: !r.linear });
  for (const r of every(internal(), 13).slice(1, 4)) run('reactionControlPoints', { id: r.id });
  const cell = m().cells[0];
  if (cell) {
    const s = m().selectableInRect({ x: cell.rect.x, y: cell.rect.y, w: cell.rect.w * 0.6, h: cell.rect.h * 0.6 });
    const selection = { nodes: [...s.nodes], reactions: [...s.reactions], texts: [...s.texts] };
    run('translateSelection', { selection, dx: 14, dy: 22 });
    const some = { nodes: selection.nodes.slice(0, 6), reactions: selection.reactions.slice(0, 4), texts: selection.texts.slice(0, 3) };
    run('alignSelection', { selection: some, mode: 'right' });
    run('distributeSelection', { selection: some, axis: 'horizontal' });
    run('alignSelectionInLine', { selection: { reactions: selection.reactions.slice(4, 8) }, mode: 'horizontal' });
    const inner = cell.compartments.find((c) => c.parentId !== null);
    if (inner) {
      run('editGeometryRect', { cellId: cell.id, compartmentId: inner.id, rect: { ...inner.rect, x: inner.rect.x + 18, y: inner.rect.y + 12 } });
      run('editGeometryRect', { cellId: cell.id, compartmentId: inner.id, rect: { ...inner.rect, w: inner.rect.w * 0.9, h: inner.rect.h * 1.05 } });
    }
    const outer = cell.compartments.find((c) => c.parentId === null);
    run('editGeometryRect', { cellId: cell.id, compartmentId: outer.id, rect: { ...outer.rect, x: outer.rect.x + 30, y: outer.rect.y + 10 } });
  }
  const env = m().environment;
  run('editGeometryRect', { cellId: null, compartmentId: env.id, rect: { ...env.rect, w: env.rect.w + 150, h: env.rect.h + 80 } });
  run('setCanvasSize', { width: m().width + 200, height: m().height + 200, move: { shiftX: 60, shiftY: 35 } });
  const copyable = [...m().nodes.values()].find((n) => n.kind === 'metabolite' && !n.hidden);
  if (copyable) {
    const copy = run('duplicateMetaboliteNode', { id: copyable.id });
    run('mergeMetaboliteNodes', { source: copy, target: copyable.id });
  }
  for (const r of every(reactions(), 5).slice(1, 4)) run('removeReaction', { id: r.id });
  run('reload');
  for (const r of every(reactions(), 6)) run('moveReaction', { id: r.id, x: r.x - 9, y: r.y + 16 });
  run('stats');
}

if (want('edits')) {
  const entries = [];
  const shell = EditableMap.emptyFromCommunity(communities.get('solo-ecoli'));
  const a = recordEdits(shell, communities.get('solo-ecoli'), handDrawnScript);
  entries.push({ name: 'hand-drawn', scenario: 'solo-ecoli', start: 'empty', file: await write('edits-hand-drawn.json', a, { small: true }) });
  const full = EditableMap.fromCommunity(communities.get('aerobio-mixto'));
  const b = recordEdits(full, communities.get('aerobio-mixto'), layoutScript);
  entries.push({ name: 'layout', scenario: 'aerobio-mixto', start: 'layout', file: await write('edits-layout.json', b, { small: true }) });
  // Sequence C: the same kinds of edit on maps drawn by hand in the app, which
  // carry everything a real file carries (straight reactions, texts, copies,
  // lanes written by earlier builds).
  for (const { path } of await savedMaps()) {
    if (!/uploaded-models-coxeter-map\(5\) \(1\)|yeast-GEM-cytosol-mito|12-phase7-Ba-Bt-envelope/.test(path)) continue;
    const parsed = parseMapDocument(await readFile(path, 'utf8'));
    const steps = recordEdits(EditableMap.fromJSON(parsed.map), null, savedMapScript);
    const file = await write(`edits-saved-${slug(path)}.json`, steps, { large: true });
    // A thousand-reaction map compared after each of some eighty steps takes a
    // debug build minutes: the Rust test runs it only with COXETER_SLOW_TESTS=1.
    entries.push({ name: `saved-${slug(path)}`, start: 'file', slow: parsed.map.reactions.length > 500, ...inputRef(path, file) });
  }
  await writeIndex('edits', entries);
}

// Reconcile

const { pruneMapToModel, reconcileCompartments, describeReconciliation } = await import(pathToFileURL(join(root, 'src/map/reconcile-map.js')).href);
const { mapSpeciesIds, renameMapSpecies, createMapDocument, validateMapContext } = await import(pathToFileURL(join(root, 'src/map/map-document.js')).href);
const { prepareImportedModels } = await import(pathToFileURL(join(root, 'src/core/model-import.js')).href);

/**
 * A model file of the user, read as the app reads an uploaded file, then
 * written as COBRA JSON (Model#toJSON) and read back: the Rust test starts from
 * that same JSON, so the two sides hold the same model whoever parsed the SBML.
 */
async function uploadedModel(path, name) {
  const prepared = prepareImportedModels([{ name: basename(path), text: await readFile(path, 'utf8') }]);
  const entry = prepared.entries[0];
  const json = JSON.parse(JSON.stringify(entry.model.toJSON()));
  const file = await write(`model-${name}.json`, json, { large: true });
  return { model: new Model(json), speciesId: entry.speciesId, modelKey: entry.modelKey, file };
}

/** Open a saved map over a community the way the store does, and report it. */
function reconcileCase(parsed, community, { speciesId = null, modelKey = null, keepSpecies = [] } = {}) {
  const saved = mapSpeciesIds(parsed);
  let renamedFrom = null;
  if (speciesId && saved.length === 1 && saved[0] !== speciesId) {
    parsed = renameMapSpecies(parsed, saved[0], speciesId, { model: modelKey });
    renamedFrom = saved[0];
  }
  const map = EditableMap.fromJSON(JSON.parse(JSON.stringify(parsed.map)));
  const contents = pruneMapToModel(map, community, { keepSpecies: new Set(keepSpecies) });
  const pruned = map.stats();
  const shell = reconcileCompartments(map, community);
  return {
    renamedFrom, keepSpecies, contents, prunedStats: pruned, shell,
    description: describeReconciliation(contents, shell),
    map: map.toJSON(),
  };
}

if (want('reconcile')) {
  const entries = [];
  const home = homedir();
  const mito = join(home, 'Downloads/uploaded-models-coxeter-map(5) (1).json');
  const userCases = [
    // BiGG metabolite ids: every reaction is drawn afresh.
    { name: 'yeast-mito-bigg', model: join(home, 'Downloads/yeastgem_v9.1.1-17.40.json'), modelName: 'yeast9-bigg', map: mito },
    // The native ids the map was drawn on: nothing is redrawn.
    { name: 'yeast-mito-native', model: join(home, 'Documents/GitHub/pc_model_porcine/other_models/yeast9/yeast-GEM.xml'), modelName: 'yeast9-native', map: mito },
    { name: 'yeast-cytosol-mito-native', model: join(home, 'Documents/GitHub/pc_model_porcine/other_models/yeast9/yeast-GEM.xml'), modelName: 'yeast9-native', map: join(home, 'Downloads/yeast-GEM-cytosol-mito-coxeter-map.json') },
  ];
  const loaded = new Map();
  for (const c of userCases) {
    if (!existsSync(c.model) || !existsSync(c.map)) continue;
    if (!loaded.has(c.modelName)) loaded.set(c.modelName, await uploadedModel(c.model, c.modelName));
    const up = loaded.get(c.modelName);
    const community = new CommunityModel({ species: [{ id: up.speciesId, model: up.model, abundance: 1, enabled: true }], medium: {} });
    const parsed = parseMapDocument(await readFile(c.map, 'utf8'));
    const result = reconcileCase(parsed, community, up);
    const file = await write(`reconcile-${c.name}.json`, result, { large: true });
    entries.push({
      name: c.name, kind: 'upload', model: up.file, speciesId: up.speciesId, modelKey: up.modelKey,
      ...inputRef(c.map, file),
    });
    console.log(`  ${c.name}: dropped ${result.contents.dropped.length}, redrawn ${result.contents.redrawn.length}, changed ${result.contents.changed.length}, nodes ${result.contents.nodes};${result.description}`);
  }

  // Bundled scenarios, with what a stale file gets wrong written into the map.
  const yeast = communities.get('solo-levadura');
  const fresh = () => EditableMap.fromCommunity(yeast).toJSON();
  const comp = (doc, id) => doc.cells[0].compartments.find((c) => c.id === id);
  const synthetic = [];
  {
    // The mitochondrion drawn beside the cell: it moves into the cytosol as it is.
    const doc = fresh();
    const m = comp(doc, 'm');
    if (m) {
      const cell = doc.cells[0];
      const dx = cell.rect.x + cell.rect.w + 60 - m.rect.x;
      for (const node of doc.nodes) if (node.cellId === cell.id && node.compartment === 'm') node.x += dx;
      m.rect = { ...m.rect, x: m.rect.x + dx };
      m.parentId = null;
      doc.canvas.width += m.rect.w + 200;
      doc.environment.rect.w += m.rect.w + 200;
      synthetic.push({ name: 'organelle-beside-cell', scenario: 'solo-levadura', doc });
    }
  }
  {
    // The compartment around the matrix drawn beside the cell, and the matrix
    // straight in its grandparent: the missing layer goes around it.
    const doc = fresh();
    const m = comp(doc, 'm');
    const mid = m ? comp(doc, m.parentId) : null;
    if (m && mid && mid.parentId) {
      const cell = doc.cells[0];
      m.parentId = mid.parentId;
      for (const node of doc.nodes) if (node.cellId === cell.id && node.compartment === mid.id) node.x += cell.rect.w + 200;
      mid.rect = { x: cell.rect.x + cell.rect.w + 80, y: cell.rect.y, w: 300, h: 240 };
      mid.parentId = null;
      doc.canvas.width += 600;
      doc.environment.rect.w += 600;
      synthetic.push({ name: 'compartment-goes-around', scenario: 'solo-levadura', doc });
    }
  }
  {
    // That layer not drawn at all: it is created around the matrix.
    const doc = fresh();
    const m = comp(doc, 'm');
    const mid = m ? comp(doc, m.parentId) : null;
    if (m && mid && mid.parentId) {
      m.parentId = mid.parentId;
      doc.cells[0].compartments = doc.cells[0].compartments.filter((k) => k.id !== mid.id);
      synthetic.push({ name: 'missing-compartment-created', scenario: 'solo-levadura', doc });
    }
  }
  {
    // A map of two species opened where only one is in the model, and where
    // the other is switched off rather than gone.
    const doc = EditableMap.fromCommunity(communities.get('aerobio-mixto')).toJSON();
    synthetic.push({ name: 'species-gone', scenario: 'solo-ecoli', doc });
    synthetic.push({ name: 'species-switched-off', scenario: 'solo-ecoli', doc, keepSpecies: ['sce'] });
  }
  {
    // A metabolite and a reaction the model does not have, and a medium node
    // filed under a cell.
    const doc = EditableMap.fromCommunity(communities.get('solo-ecoli')).toJSON();
    const node = doc.nodes.find((n) => n.kind === 'metabolite');
    doc.nodes.push({ ...node, id: 'met:eco@ghost_c', mergedId: 'eco@ghost_c', x: node.x + 40 });
    const rxn = doc.reactions.find((r) => r.id === 'eco@PGI');
    doc.reactions.push({ ...rxn, id: 'eco@GONE' });
    for (const seg of doc.segments.filter((sg) => sg.reactionId === 'eco@PGI')) doc.segments.push({ ...seg, reactionId: 'eco@GONE' });
    const pfk = doc.segments.find((sg) => sg.reactionId === 'eco@PFK' && sg.kind === 'primary');
    doc.nodes.push({ ...node, id: 'met:eco@lost_c', mergedId: 'eco@lost_c', x: node.x - 40 });
    doc.segments.push({ ...pfk, nodeId: 'met:eco@lost_c' });
    const medium = doc.nodes.find((n) => n.kind === 'medium');
    medium.cellId = 'eco';
    synthetic.push({ name: 'stale-contents', scenario: 'solo-ecoli', doc });
  }
  for (const c of synthetic) {
    const result = reconcileCase({ map: c.doc, metadata: null }, communities.get(c.scenario), { keepSpecies: c.keepSpecies ?? [] });
    const input = await write(`reconcile-${c.name}-input.json`, c.doc, { large: true });
    const file = await write(`reconcile-${c.name}.json`, result, { large: true });
    entries.push({ name: c.name, kind: 'scenario', scenario: c.scenario, keepSpecies: c.keepSpecies ?? [], inputFixture: input, file });
    console.log(`  ${c.name}:${result.description || ' nothing to do'}`);
  }
  await writeIndex('reconcile', entries);
}

// Documents

if (want('documents')) {
  const { normalizeFluxColorStops } = await import(pathToFileURL(join(root, 'src/render/theme.js')).href);
  const community = communities.get('solo-ecoli');
  const map = EditableMap.fromCommunity(community, { reactions: ['eco@PGI', 'eco@PFK', 'eco@GLCptspp', 'EX_glc__D_s'] });
  map.addText('Title', 100, 100);
  map.labelSettings = { metabolite: 'name' };
  const context = {
    species: [
      { id: 'eco', file: 'ecoli_core_mc', enabled: true },
      { id: 'zzz', model: 'off', enabled: false },
      { id: 'Abc', model: 'published/x/preview-Abc' },
    ],
    scenario: 'solo-ecoli',
    pathwayManager: { groups: [{ id: 'g1', name: 'eco@ glycolysis', reactionIds: ['eco@PGI', 'eco@PFK'] }], active: 'eco@PGI' },
    fluxColorStops: [{ value: 5, color: ' #FF0000 ' }, { value: -0, color: '#00ff00' }, { value: -2.5, color: '#0000FF' }],
    study: { id: 'demo', nested: { n: 1 } },
  };
  const created = createMapDocument(map, context);
  const plain = createMapDocument(map, {});

  const attempt = (fn) => {
    try { return { ok: JSON.parse(JSON.stringify(fn() ?? null)) }; } catch (error) { return { error: error.message }; }
  };
  const legacy = map.toJSON();
  const parseInputs = [
    created, plain, legacy, { ...legacy, version: '7', fluxColorStops: [{ value: 1, color: '#abcdef' }] },
    5, null, [], 'text', {}, { format: 'Other' }, { reactions: [], metabolites: [] },
    { format: 'EscherMCMCMap' }, { format: 'EscherMCMCMap', version: 0, map: {} }, { format: 'EscherMCMCMap', version: 1.5, map: {} },
    { format: 'EscherMCMCMap', version: '2', map: {} }, { format: 'EscherMCMCMap', version: 99, map: {} },
    { format: 'EscherMCMCMap', version: 2 }, { format: 'EscherMCMCMap', version: 2, map: 'x' },
    { format: 'EscherMCMCMap', version: 1, map: { cells: [{ id: 'a' }, { id: 'b' }, { id: 'a' }] }, metadata: 'note' },
    { format: 'EscherMCMCMap', version: 3, map: {}, fluxColorStops: [] },
    { format: 'EscherMCMCMap', version: 3, map: {}, fluxColorStops: [{ value: 'x', color: '#000000' }] },
    { format: 'EscherMCMCMap', version: 3, map: {}, fluxColorStops: [{ value: 1, color: 'red' }] },
    { format: 'EscherMCMCMap', version: 3, map: {}, fluxColorStops: [{ value: 1, color: '#000000' }, { value: 1, color: '#111111' }] },
    { format: 'EscherMCMCMap', version: 3, map: {}, fluxColorStops: [{ value: 0, color: '#000000' }, { value: -0, color: '#111111' }] },
  ];
  const parses = parseInputs.map((input) => ({ input, result: attempt(() => parseMapDocument(input)) }));
  const texts = ['{"format":"EscherMCMCMap","version":3,"map":{"nodes":[]}}', 'not json'].map((text) => ({
    text, result: text === 'not json' ? { error: 'invalid JSON' } : attempt(() => parseMapDocument(text)),
  }));

  const parsed = parseMapDocument(created);
  const meta = (species) => ({ ...parsed, metadata: { ...parsed.metadata, species } });
  const validations = [
    { parsed, current: { species: context.species } },
    { parsed, current: { species: [{ id: 'eco', file: 'ecoli_core_mc' }] } },
    { parsed, current: { species: [{ id: 'eco', file: 'ecoli_core_mc' }, { id: 'Abd', model: 'published/x/model-Abc' }] } },
    { parsed, current: { species: [{ id: 'eco', file: 'other' }, { id: 'Abc', model: 'published/x/model-Abc' }] } },
    { parsed, current: { species: [{ id: 'eco', file: 'ecoli_core_mc' }, { id: 'Abc', model: 'published/x/model-Abc' }] } },
    { parsed: meta([{ id: 'a', model: 'upload:one' }]), current: { species: [{ id: 'a', file: 'upload:two' }] } },
    { parsed: meta([{ id: 'a', model: 'upload:one' }]), current: { species: [{ id: 'a', file: 'bundled' }] } },
    { parsed: meta([{ id: 'a', model: null }]), current: { species: [{ id: 'a', file: 'bundled' }] } },
    { parsed: meta([]), current: { species: [] } },
    { parsed: { ...parsed, metadata: null }, current: { species: [{ id: 'q' }] } },
  ].map((c) => ({ ...c, result: attempt(() => validateMapContext(c.parsed, c.current)) }));

  const speciesIds = [
    parsed, { ...parsed, metadata: null }, meta([{ id: 'b' }, { id: 'a' }, { id: 'b' }, { model: 'x' }]),
    { map: {}, metadata: null }, parseMapDocument(legacy),
  ].map((p) => ({ parsed: p, result: mapSpeciesIds(p) }));

  const renames = [
    { parsed, from: 'eco', to: 'ecoli2', options: { model: 'upload:ecoli2' } },
    { parsed, from: 'eco', to: 'eco', options: {} },
    { parsed: parseMapDocument(legacy), from: 'eco', to: 'K-12', options: {} },
    { parsed: { ...parsed, pathwayManager: null, map: { ...parsed.map, texts: undefined } }, from: 'eco', to: 'x', options: { model: null } },
  ].map((c) => ({ ...c, result: JSON.parse(JSON.stringify(renameMapSpecies(c.parsed, c.from, c.to, c.options))) }));

  const stops = [null, [], 'x', [{ value: 3, color: '#ABCDEF' }, { value: -1, color: '#000000' }], [null], [{ value: NaN, color: '#000000' }]]
    .map((input) => ({ input, result: attempt(() => normalizeFluxColorStops(input)) }));

  await write('documents.json', {
    map: map.toJSON(), context, created, plain, parses, texts, validations, speciesIds, renames, stops,
  }, { small: true });
  await writeIndex('documents', [{ file: 'map/documents.json' }]);
}

// Overview

if (want('overview')) {
  const { buildCoreMetabolismOverview, coreOverviewSupport, CORE_OVERVIEW_GROUPS } = await import(pathToFileURL(join(root, 'src/map/core-metabolism-overview.js')).href);
  const { reactionsInCompartment } = await import(pathToFileURL(join(root, 'src/map/compartment-reactions.js')).href);
  const entries = [];
  for (const [id, settings] of [['solo-ecoli', {}], ['aerobio-mixto', layoutPreset('compact')], ['todas', layoutPreset('spacious')], ['metanogenesis', {}]]) {
    const community = communities.get(id);
    const built = buildCoreMetabolismOverview(community, settings);
    const file = await write(`overview-${id}.json`, {
      map: built.map.toJSON(), pathwayGroups: built.pathwayGroups, placed: built.placed, perCell: built.perCell, support: built.support,
    }, { large: id !== 'solo-ecoli' });
    entries.push({ scenario: id, settings, file });
  }
  const support = bundled.map((s) => ({ scenario: s.id, support: coreOverviewSupport(communities.get(s.id)) }));
  const inCompartment = [];
  for (const s of bundled) {
    const community = communities.get(s.id);
    for (const sp of community.activeSpecies) {
      for (const c of [...sp.model.compartmentIds(), 'nope']) {
        inCompartment.push({ scenario: s.id, cellId: sp.id, compartmentId: c, reactions: reactionsInCompartment(community, sp.id, c) });
      }
    }
  }
  await write('overview-support.json', { groups: CORE_OVERVIEW_GROUPS, support, inCompartment }, { small: true });
  await writeIndex('overview', entries);
}

// Yeast9

if (want('yeast9')) {
  const { yeast9PreviewSpecs } = await import(pathToFileURL(join(root, 'src/core/yeast9.js')).href);
  const { namespaced } = await import(pathToFileURL(join(root, 'src/core/community.js')).href);
  const entries = [];
  const source = join(homedir(), 'Downloads/yeastgem_v9.1.1-17.40.json');
  if (existsSync(source)) {
    const up = await uploadedModel(source, 'yeast9-bigg');
    const community = new CommunityModel({ species: [{ id: 'sce', model: up.model, abundance: 1, enabled: true }], medium: {} });
    // What the store does when the Yeast9 scenario is opened (store.js:
    // useYeast9Scenario and populateYeast9Preview, with yeastPreviewAnchors).
    const map = EditableMap.emptyFromCommunity(community, { width: 2100, height: 1550, ...layoutEngineOptions(layoutPreset('spacious')) });
    const pointInsideRect = (p, r, pad = 0) => p.x > r.x - pad && p.x < r.x + r.w + pad && p.y > r.y - pad && p.y < r.y + r.h + pad;
    const anchorsFor = (compartmentId, count) => {
      const cell = map.cells.find((item) => item.id === 'sce');
      const compartment = cell?.compartments.find((item) => item.id === compartmentId);
      if (!cell || !compartment || count <= 0) return [];
      const rect = compartment.rect;
      const children = cell.compartments.filter((item) => item.parentId === compartmentId).map((item) => item.rect);
      const candidates = [];
      for (let row = 0; row < 6; row += 1) {
        for (let col = 0; col < 8; col += 1) {
          const point = { x: rect.x + rect.w * ((col + 1) / 9), y: rect.y + rect.h * ((row + 1) / 7) };
          const childPadding = Math.min(rect.w, rect.h) * 0.018;
          if (!children.some((child) => pointInsideRect(point, child, childPadding))) candidates.push(point);
        }
      }
      const pool = candidates.length >= count ? candidates : Array.from({ length: count }, (_, index) => ({
        x: rect.x + rect.w * (((index % 4) + 1) / 5),
        y: rect.y + rect.h * ((Math.floor(index / 4) + 1) / (Math.ceil(count / 4) + 1)),
      }));
      if (count === 1) return [pool[Math.floor(pool.length / 2)]];
      return Array.from({ length: count }, (_, index) => pool[Math.round(index * (pool.length - 1) / (count - 1))]);
    };
    const specs = yeast9PreviewSpecs(up.model);
    const byCompartment = new Map();
    for (const spec of specs) {
      if (!byCompartment.has(spec.compartment)) byCompartment.set(spec.compartment, []);
      byCompartment.get(spec.compartment).push(spec);
    }
    const shell = map.toJSON();
    let placed = 0;
    const covered = new Set();
    const missing = [];
    const anchors = {};
    for (const [compartmentId, group] of byCompartment) {
      anchors[compartmentId] = anchorsFor(compartmentId, group.length);
      group.forEach((spec, index) => {
        const result = map.addReaction(community, namespaced('sce', spec.reactionId), { anchor: anchors[compartmentId][index] ?? null });
        if (result) { placed += 1; covered.add(compartmentId); } else missing.push(spec.reactionId);
      });
    }
    const file = await write('yeast9-preview.json', {
      specs, anchors, shell, placed, coveredCompartments: covered.size, missing, map: map.toJSON(),
    }, { large: true });
    entries.push({ name: 'preview', model: up.file, file });
  }
  await writeIndex('yeast9', entries);
}

// Perf

// Genome-scale layouts, for the performance numbers and to prove the Rust
// neighbour grid gives the same bits at scale. They take minutes in Node, so
// they only run when asked for: `node rust/tools/fixtures-map.mjs perf` (one
// E. coli) or `perf4` (the four-E. coli community).
if (sections.has('perf') || sections.has('perf4')) {
  const dir = join(root, 'models/published/steadycom-ecoli-auxotrophs');
  const member = async (id) => new Model(JSON.parse(await readFile(join(dir, `model-${id}.json`), 'utf8')));
  const cases = [];
  if (sections.has('perf')) cases.push({ name: 'ecoli1', ids: ['Ec1'] });
  if (sections.has('perf4')) cases.push({ name: 'ecoli4', ids: ['Ec1', 'Ec2', 'Ec3', 'Ec4'] });
  const previous = existsSync(join(outSmall, 'index-perf.json')) ? JSON.parse(await readFile(join(outSmall, 'index-perf.json'), 'utf8')) : [];
  const entries = previous.filter((e) => !cases.some((c) => c.name === e.name));
  for (const c of cases) {
    const species = [];
    for (const id of c.ids) species.push({ id, model: await member(id), abundance: 1 / c.ids.length, enabled: true });
    const community = new CommunityModel({ species, medium: {} });
    const t = performance.now();
    const map = EditableMap.fromCommunity(community);
    const ms = performance.now() - t;
    const stats = map.stats();
    const file = await write(`layout-big-${c.name}.json`, map.toJSON(), { large: true });
    entries.push({ name: c.name, members: c.ids, nodeMs: Math.round(ms), reactions: stats.reactions, nodes: stats.nodes, segments: stats.segments, node: process.version, file });
    console.log(`  ${c.name}: ${stats.reactions} reactions, ${stats.nodes} nodes, ${(ms / 1000).toFixed(1)} s in Node ${process.version}`);
  }
  await writeIndex('perf', entries);
}
