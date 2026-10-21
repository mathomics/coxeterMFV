#!/usr/bin/env node
/**
 * Build the curated map of the Stolyar et al. (2007) syntrophic pair,
 * Desulfovibrio vulgaris (Dv) and Methanococcus maripaludis (Mm).
 *
 *   node tools/scenario-maps/build-stolyar.mjs [--check] [--verbose] [--out=<file>] [--force]
 *
 * --out writes the map elsewhere and leaves the manifest alone; --force writes
 * it even when the checks find a problem; --verbose lists the crossings, the
 * crowded spots and the labels without a clear place.
 *
 * Reads the two member models the app loads for the published community
 * (models/published/stolyar-2007-syntrophy/preview-*.json), builds the merged
 * community the way the app does, and writes
 * models/published/stolyar-2007-syntrophy/map.json and the manifest's `map`
 * entry, which is how the app finds it. Every reaction of both models is drawn
 * with its full stoichiometry, the medium exchanges included; --check writes
 * nothing and only reports. The layout is given in absolute map units (see
 * stolyar-layout.mjs), so the output is the same on every run.
 *
 * Drawing rules (docs/gut9-map/COXETER_REGLAS_CONSTRUCCION_MAPAS.md and the
 * hand-made Yeast9 and Zuñiga maps): one shared node for each
 * intermediate of a continuous route, cofactors as small nodes beside their
 * reaction, transporters on the membrane they cross, cycles on true circles
 * (each reaction halfway between its metabolites with its axis on the tangent,
 * each branch a circular arc), and cross-feeding only through the shared
 * medium. A metabolite a reaction uses far from where its route is drawn gets a
 * small copy beside that reaction instead of a long line; the biomass lumps take
 * all their precursors that way, as one column each whose branches nest without
 * crossing.
 */
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Model } from '../../src/core/model.js';
import { CommunityModel, OBJECTIVES, LINK_COMPARTMENT, SHARED_COMPARTMENT } from '../../src/core/community.js';
import { EditableMap } from '../../src/map/editable-map.js';
import { reactionBoundaryKeys, boundaryKey } from '../../src/map/boundaries.js';
import { MAP_DOCUMENT_FORMAT, MAP_DOCUMENT_VERSION } from '../../src/map/map-document.js';
import { LAYOUT } from './stolyar-layout.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const COMMUNITY = 'stolyar-2007-syntrophy';
const DIR = join(root, 'models', 'published', COMMUNITY);
const option = (name) => process.argv.slice(2).find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3) ?? null;
const OUT = option('out') ?? join(DIR, 'map.json');
// A draft written elsewhere leaves the manifest alone.
const SET_MANIFEST = !option('out');
const CHECK_ONLY = process.argv.includes('--check');
const FORCE = process.argv.includes('--force');

const AXIS = 17;            // half length of a reaction marker (SIZES.reactionAxis)
const PR = 13;              // primary metabolite radius
const SR = 7;               // cofactor radius
const STUB = 46;            // community exchange: node to marker, marker to boundary tip
const BLOCK_STEP = 19;      // spacing of a biomass block's precursors down their column
// Cofactor template, as (along the axis from the marker's centre, across it),
// rows from the inside out, as the hand-made maps place them.
const COF_ROWS = [[54, 40], [36, 68], [20, 96]];
// Carriers first, protons last, so the same cofactor takes the same place everywhere.
const COF_ORDER = ['ATP', 'ADP', 'NADPH', 'NADH', 'FADH', 'Fdx(red)', 'F420(red)', 'C3(red)', 'QH2', 'CoM-SH',
  'H2', 'CO2', 'NH3', 'Glu', 'aKG', 'Gln', 'PRPP', 'mTHF', 'Me-H4MPT', 'H+'];

// Community

async function community() {
  const manifest = JSON.parse(await readFile(join(DIR, 'manifest.json'), 'utf8'));
  const species = [];
  for (const member of manifest.members) {
    const doc = JSON.parse(await readFile(join(DIR, member.preview), 'utf8'));
    species.push({ id: member.id, model: new Model(doc), abundance: 1 / manifest.members.length, enabled: true });
  }
  // The medium as the app's Store builds it from the manifest (mediumSpec()).
  const medium = Object.fromEntries(Object.entries(manifest.medium ?? {})
    .map(([base, limit]) => [base, { lower: -Math.abs(Number(limit) || 0), upper: 1000 }]));
  return { manifest, community: new CommunityModel({ name: 'Stolyar 2007', objective: OBJECTIVES.EQUAL_GROWTH, species, medium }) };
}

// Geometry helpers

const P = (x, y) => ({ x: Math.round(x * 100) / 100, y: Math.round(y * 100) / 100 });
const unit = (x, y) => { const l = Math.hypot(x, y) || 1; return { x: x / l, y: y / l }; };
const lerp = (a, b, t) => ({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t });
const rad = (deg) => (deg * Math.PI) / 180;
const onCircle = (c, r, deg) => ({ x: c.x + r * Math.cos(rad(deg)), y: c.y + r * Math.sin(rad(deg)) });
const tangent = (deg) => ({ x: -Math.sin(rad(deg)), y: Math.cos(rad(deg)) });
const wrap = (a) => { while (a > 180) a -= 360; while (a <= -180) a += 360; return a; };
const angleOf = (c, p) => (Math.atan2(p.y - c.y, p.x - c.x) * 180) / Math.PI;
const arcHandle = (span, r) => (4 / 3) * Math.tan(rad(Math.abs(span)) / 4) * r;
const COMPASS = { N: [0, -1], S: [0, 1], E: [1, 0], W: [-1, 0], NE: [1, -1], NW: [-1, -1], SE: [1, 1], SW: [-1, 1] };
const NORMAL = { right: { x: 1, y: 0 }, left: { x: -1, y: 0 }, top: { x: 0, y: -1 }, bottom: { x: 0, y: 1 } };
const dirOf = (d) => (typeof d === 'string' ? unit(...COMPASS[d]) : unit(d.x ?? d[0], d.y ?? d[1]));

function straightControls(node, anchor) { return [lerp(node, anchor, 0.34), lerp(node, anchor, 0.72)]; }

/** Coxeter's shapeBranch(): leave the marker along its axis (u), then turn square across to the node. */
function sideControls(node, anchor, u) {
  const vx = node.x - anchor.x, vy = node.y - anchor.y;
  const along = vx * u.x + vy * u.y;
  const side = -vx * u.y + vy * u.x;
  const across = Math.abs(side);
  if (across < 4 && along > 0) return straightControls(node, anchor);
  const hand = side < 0 ? -1 : 1;
  const n = { x: -u.y * hand, y: u.x * hand };
  const reach = Math.max(along, 0.5 * across, 14);
  return [{ x: node.x - n.x * across * 0.55, y: node.y - n.y * across * 0.55 },
    { x: anchor.x + u.x * reach * 0.8, y: anchor.y + u.y * reach * 0.8 }];
}

/** A line node beside the axis (a parallel reaction drawn as an arc): on along the axis, then into the node. */
function lensControls(node, anchor, u) {
  const along = (node.x - anchor.x) * u.x + (node.y - anchor.y) * u.y;
  const b2 = { x: anchor.x + u.x * along * 0.5, y: anchor.y + u.y * along * 0.5 };
  return [lerp(node, b2, 0.45), b2];
}

function bezierPoints(seg, steps = 10) {
  const p0 = seg.from, p3 = seg.to;
  const [p1, p2] = seg.side === 'substrate' ? [seg.b1, seg.b2] : [seg.b2, seg.b1];
  const out = [];
  for (let i = 0; i <= steps; i++) {
    const t = i / steps, a = (1 - t) ** 3, b = 3 * (1 - t) ** 2 * t, c = 3 * (1 - t) * t * t, d = t ** 3;
    out.push({ x: a * p0.x + b * p1.x + c * p2.x + d * p3.x, y: a * p0.y + b * p1.y + c * p2.y + d * p3.y });
  }
  return out;
}

const pointSegDist = (p, a, b) => {
  const dx = b.x - a.x, dy = b.y - a.y, l2 = dx * dx + dy * dy;
  const t = l2 ? Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / l2)) : 0;
  return Math.hypot(p.x - a.x - t * dx, p.y - a.y - t * dy);
};
const cross = (p1, p2, p3, p4) => {
  const o = (a, b, c) => (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
  return o(p1, p2, p3) * o(p1, p2, p4) < 0 && o(p3, p4, p1) * o(p3, p4, p2) < 0;
};

// The map

async function build() {
  const { community: comm } = await community();
  const merged = comm.merged;
  const shell = EditableMap.emptyFromCommunity(comm);
  const cells = shell.cells.map((cell) => {
    const spec = LAYOUT.cells[cell.id];
    const outer = { ...spec.e };
    // The band between the boundary and the cytosol: one width, or one per edge.
    const b = typeof spec.band === 'number' ? { left: spec.band, right: spec.band, top: spec.band, bottom: spec.band } : spec.band;
    const inner = { x: outer.x + b.left, y: outer.y + b.top, w: outer.w - b.left - b.right, h: outer.h - b.top - b.bottom };
    return {
      id: cell.id, name: spec.name, organism: cell.organism, note: cell.note, color: cell.color, abundance: cell.abundance,
      rect: { ...outer }, outerCompartment: 'e',
      compartments: cell.compartments.map((c) => ({
        ...c, rect: c.id === 'e' ? { ...outer } : { ...inner },
        label: c.id === 'e' ? 'Extracellular (own copy)' : 'Cytosol',
        ...(spec.labelPosition?.[c.id] ? { labelPosition: spec.labelPosition[c.id] } : {}),
      })),
    };
  });
  const rectOf = (cellId, comp) => {
    if (!cellId) return LAYOUT.environment;
    return cells.find((c) => c.id === cellId).compartments.find((c) => c.id === comp).rect;
  };

  const nodes = new Map();
  const reactions = new Map();
  const segments = [];
  const problems = [];
  const owner = (mid) => comm.metaboliteOwner.get(mid) ?? { speciesId: null, compartment: SHARED_COMPARTMENT };
  const metOf = (mid) => merged.metabolites.get(mid);
  const localOf = (mid) => mid.slice(mid.indexOf('@') + 1);
  const label = (mid, sp) => {
    const local = localOf(mid);
    const table = { ...LAYOUT.labels.common, ...(LAYOUT.labels[sp] ?? {}) };
    const bare = local.replace(/^ZM_/, '');
    return table[local] ?? table[bare] ?? bare;
  };

  // Medium nodes of metabolites that pass from one cell to the other keep the
  // model's own id as their label: the cross-feeding lines find their node by it.
  const crossFed = new Set(Object.entries(LAYOUT.medium).filter(([, v]) => v.keepId).map(([k]) => k));

  function primary(mid, at, opts = {}) {
    const id = `met:${mid}`;
    if (nodes.has(id)) throw new Error(`node ${id} placed twice`);
    const met = metOf(mid);
    if (!met) throw new Error(`no metabolite ${mid}`);
    const own = owner(mid);
    const medium = own.speciesId === null;
    const base = medium && crossFed.has(localOf(mid)) ? localOf(mid) : label(mid, own.speciesId);
    const n = {
      id, kind: medium ? 'medium' : 'metabolite', mergedId: mid, localId: localOf(mid), base, name: met.name ?? localOf(mid),
      formula: '', compartment: own.compartment, cellId: own.speciesId ?? null, x: at.x, y: at.y, r: PR,
      primary: true, visualRole: 'primary',
    };
    if (opts.label !== undefined) n.labelAngle = opts.label;
    nodes.set(id, n);
    return n;
  }

  function satellite(rid, mid, at) {
    let id = `cur:${rid}:${mid}`;
    for (let k = 2; nodes.has(id); k++) id = `cur:${rid}:${mid}#${k}`;
    const met = metOf(mid);
    const own = owner(mid);
    const n = {
      id, kind: 'currency', mergedId: mid, localId: localOf(mid), base: label(mid, own.speciesId), name: met.name ?? localOf(mid),
      formula: '', compartment: own.compartment, cellId: own.speciesId ?? null, reactionId: rid,
      x: at.x, y: at.y, r: SR, primary: false, visualRole: 'secondary',
    };
    nodes.set(id, n);
    return n;
  }

  // Primary nodes
  for (const [sp, spec] of Object.entries(LAYOUT.species)) {
    for (const [local, value] of Object.entries(spec.nodes)) {
      const [x, y, lab] = value;
      primary(`${sp}@${local}`, { x, y }, { label: lab === undefined ? undefined : rad(lab) });
    }
  }
  for (const [base, value] of Object.entries(LAYOUT.medium)) {
    primary(`M@${base}`, { x: value.at[0], y: value.at[1] }, { label: value.label === undefined ? undefined : rad(value.label) });
  }
  const nodeFor = (mid) => nodes.get(`met:${mid}`);

  // Reactions
  const rings = {};
  for (const [sp, spec] of Object.entries(LAYOUT.species)) {
    for (const [name, ring] of Object.entries(spec.rings ?? {})) {
      const centre = { x: ring.c[0], y: ring.c[1] };
      rings[`${sp}:${name}`] = { centre, r: ring.r, angles: Object.fromEntries(Object.entries(ring.nodes).map(([k, a]) => [`${sp}@${k}`, a])) };
      for (const [local, angle] of Object.entries(ring.nodes)) {
        const node = nodeFor(`${sp}@${local}`) ?? primary(`${sp}@${local}`, onCircle(centre, ring.r, angle));
        Object.assign(node, P(onCircle(centre, ring.r, angle).x, onCircle(centre, ring.r, angle).y));
        node.ring = `${sp}:${name}`;
        // A ring whose cofactors stand inside names its metabolites outside,
        // straight out from the centre unless the layout says otherwise.
        if (ring.labelsOut && node.labelAngle === undefined) {
          node.labelAngle = rad(ring.label?.[local] ?? Math.round(angle / 45) * 45);
        }
      }
    }
  }

  const drawn = new Set();
  const placeReaction = (rid, spec, sp) => {
    const model = merged.reactions.get(rid);
    if (!model) throw new Error(`no reaction ${rid}`);
    const own = comm.reactionOwner.get(rid) ?? {};
    const entries = Object.entries(model.metabolites)
      .filter(([mid]) => metOf(mid)?.compartment !== LINK_COMPARTMENT && owner(mid).compartment !== LINK_COMPARTMENT);
    const compartments = [...new Set(entries.map(([mid]) => owner(mid).compartment))];
    const rxn = {
      id: rid, localId: own.originalId ?? localOf(rid), name: model.name ?? rid, subsystem: model.subsystem ?? '',
      cellId: own.speciesId ?? null, kind: own.role ?? 'metabolic', compartments,
      membrane: null, boundaryId: null, onMembrane: false, normal: null,
      equation: typeof model.equation === 'function' ? model.equation() : '',
      bounds: { lower: model.lowerBound, upper: model.upperBound },
    };
    const local = (m) => `${sp}@${m}`;
    const primaries = new Set((spec.prim ?? []).map((m) => (m.includes('@') ? m : local(m))));
    let centre, dir, lineNodes = [];
    let ring = null, ringAngle = 0, ringRadius = 0, ringSense = 1;

    if (spec.line) {
      const [a, b] = spec.line.map((m) => nodeFor(m.includes('@') ? m : local(m)));
      if (!a || !b) throw new Error(`${rid}: line node missing`);
      lineNodes = [a, b];
      a.mergedId in model.metabolites || problems.push(`${rid}: ${a.mergedId} not in reaction`);
      b.mergedId in model.metabolites || problems.push(`${rid}: ${b.mergedId} not in reaction`);
      // The axis runs from the model's substrate to its product.
      const [s, p] = model.metabolites[a.mergedId] < 0 ? [a, b] : [b, a];
      primaries.add(a.mergedId); primaries.add(b.mergedId);
      if (spec.ring) {
        ring = rings[`${sp}:${spec.ring}`];
        const angA = ring.angles[a.mergedId], angB = ring.angles[b.mergedId];
        const mid = angA + wrap(angB - angA) / 2;
        ringRadius = ring.r + (spec.track ?? 0);
        ringAngle = mid;
        centre = onCircle(ring.centre, ringRadius, mid);
        const t = tangent(mid);
        const toward = wrap(ring.angles[p.mergedId] - mid) > 0 ? 1 : -1;
        ringSense = toward;
        dir = { x: t.x * toward, y: t.y * toward };
      } else {
        dir = unit(p.x - s.x, p.y - s.y);
        const t = spec.t ?? 0.5;
        const base = lerp(s, p, model.metabolites[a.mergedId] < 0 ? t : 1 - t);
        const off = spec.off ?? 0;   // positive: to the right of the a -> b direction on the page
        const ab = unit(b.x - a.x, b.y - a.y);
        centre = { x: base.x - ab.y * off, y: base.y + ab.x * off };
        if (spec.at) centre = { x: spec.at[0], y: spec.at[1] };
      }
    } else if (spec.at) {
      centre = { x: spec.at[0], y: spec.at[1] };
      dir = dirOf(spec.dir ?? 'S');
    } else if (spec.mem || spec.ex) {
      const [edge, pos] = spec.mem ?? spec.ex;
      const comp = spec.mem ? 'c' : 'e';
      const rect = rectOf(rxn.cellId, comp);
      const normal = NORMAL[edge];
      centre = edge === 'right' ? { x: rect.x + rect.w, y: pos } : edge === 'left' ? { x: rect.x, y: pos }
        : edge === 'top' ? { x: pos, y: rect.y } : { x: pos, y: rect.y + rect.h };
      rxn.membrane = comp; rxn.onMembrane = true; rxn.normal = { ...normal };
      rxn.boundaryId = boundaryKey(rxn.cellId, comp);
      const keys = reactionBoundaryKeys(comm, rid);
      if (!keys.includes(rxn.boundaryId)) problems.push(`${rid}: does not cross ${rxn.boundaryId} (${keys.join(', ')})`);
      const inside = (mid) => (comp === 'c' ? owner(mid).compartment === 'c' : owner(mid).speciesId !== null);
      const subsOut = entries.some(([mid, c]) => c < 0 && !inside(mid));
      dir = subsOut ? { x: -normal.x, y: -normal.y } : { ...normal };
      rxn.inside = inside;
    } else if (spec.medium) {
      const node = nodeFor(entries[0][0]);
      const normal = dirOf(spec.medium);
      centre = { x: node.x + normal.x * STUB, y: node.y + normal.y * STUB };
      dir = { x: -normal.x, y: -normal.y };
      rxn.normal = P(normal.x, normal.y);
      rxn.boundary = P(centre.x + normal.x * STUB, centre.y + normal.y * STUB);
      for (const m of entries) primaries.add(m[0]);
      // An exchange's axis runs from the medium node towards the boundary.
      dir = normal;
    } else {
      throw new Error(`${rid}: no placement`);
    }
    if (spec.dir && !spec.at) dir = dirOf(spec.dir);
    if (spec.mem || spec.ex) for (const m of spec.prim ?? []) primaries.add(m.includes('@') ? m : local(m));

    rxn.x = Math.round(centre.x * 100) / 100; rxn.y = Math.round(centre.y * 100) / 100;
    const a1 = P(centre.x - dir.x * AXIS, centre.y - dir.y * AXIS), a2 = P(centre.x + dir.x * AXIS, centre.y + dir.y * AXIS);
    rxn.axis = { dir: P(dir.x, dir.y), a1, a2 };
    if (spec.line && !spec.ring && !spec.off && !spec.at) rxn.linear = { from: lineNodes[0].id, to: lineNodes[1].id, t: spec.t ?? 0.5 };
    const inside = rxn.inside; delete rxn.inside;
    reactions.set(rid, rxn);
    drawn.add(rid);
    segments.push({ reactionId: rid, kind: 'axis', from: a1, to: a2, arrow: false });

    // Cofactor places
    const perp = { x: -dir.y, y: dir.x };
    let hand = 1;
    if (spec.side && !ring && !rxn.onMembrane) {
      const s = COMPASS[spec.side];
      hand = perp.x * s[0] + perp.y * s[1] < 0 ? -1 : 1;
    }
    if (ring) hand = spec.side === 'in' ? -1 : 1;
    const satNodes = [];
    const rank = (mid) => { const i = COF_ORDER.indexOf(localOf(mid).replace(/^ZM_/, '')); return i < 0 ? COF_ORDER.length : i; };
    const sides = { substrate: [], product: [] };
    for (const [mid, coef] of entries) {
      if (primaries.has(mid)) continue;
      sides[coef < 0 ? 'substrate' : 'product'].push(mid);
    }
    const placeAt = {};
    const gridLabel = {};
    const membraneCount = { in: 0, out: 0 };
    for (const side of ['substrate', 'product']) {
      const list = sides[side].sort((x, y) => rank(x) - rank(y) || x.localeCompare(y));
      const sign = side === 'substrate' ? -1 : 1;
      list.forEach((mid, i) => {
        const key = localOf(mid);
        const manual = spec.sat?.[key] ?? spec.sat?.[`${side}:${key}`];
        if (manual) { placeAt[mid] = { x: manual[0], y: manual[1] }; return; }
        if (spec.grid && side === spec.grid.side) {
          const g = spec.grid;
          const cols = g.cols ?? 1;
          const col = i % cols, row = Math.floor(i / cols);
          const rows = Math.ceil(list.length / cols);
          placeAt[mid] = { x: g.x + col * (g.dx ?? 0), y: g.y + (row - (rows - 1) / 2) * (g.dy ?? BLOCK_STEP) };
          // A block's precursors are named on the side away from the reaction, in columns.
          gridLabel[mid] = rad(g.label ?? 180);
          return;
        }
        let row = i, h = spec.hands?.[side] ?? hand;
        const rows = spec.rows?.[side] ?? spec.rows?.all ?? COF_ROWS;
        if (row >= rows.length) { row -= rows.length; h = -h; }
        const [along, across] = rows[Math.min(row, rows.length - 1)];
        if (rxn.onMembrane) {
          const isIn = inside(mid);
          const inward = { x: -rxn.normal.x, y: -rxn.normal.y };
          const p2 = { x: -inward.y, y: inward.x };
          const hh = spec.hands?.[isIn ? 'in' : 'out'] ?? (side === 'substrate' ? -1 : 1);
          const k = membraneCount[isIn ? 'in' : 'out']++;
          const rowsIn = [[48, 34], [40, 64], [32, 94], [24, 124]];
          const [al, ac] = isIn ? rowsIn[Math.min(k, 3)] : [-32, 30 + 28 * k];
          const hs = isIn ? hh : (spec.hands?.out ?? 1);
          placeAt[mid] = { x: rxn.x + inward.x * al + p2.x * ac * hs, y: rxn.y + inward.y * al + p2.y * ac * hs };
          return;
        }
        if (ring) {
          const turn = (along / ringRadius) * (180 / Math.PI) * sign * ringSense;
          placeAt[mid] = onCircle(ring.centre, ringRadius + across * h, ringAngle + turn);
          return;
        }
        placeAt[mid] = { x: centre.x + dir.x * along * sign + perp.x * across * h, y: centre.y + dir.y * along * sign + perp.y * across * h };
      });
    }

    // Branches
    for (const [mid, coef] of entries) {
      const side = coef < 0 ? 'substrate' : 'product';
      let node = primaries.has(mid) ? nodeFor(mid) : null;
      if (primaries.has(mid) && !node) { problems.push(`${rid}: primary ${mid} has no node`); }
      if (!node) {
        node = satellite(rid, mid, P(placeAt[mid].x, placeAt[mid].y));
        if (gridLabel[mid] !== undefined) node.labelAngle = gridLabel[mid];
        const lab = spec.label?.[localOf(mid)];
        if (lab !== undefined) node.labelAngle = rad(lab);
      }
      if (!node.primary) satNodes.push(node);
      let anchor, u;
      if (rxn.onMembrane) {
        const isIn = inside(mid);
        const inward = { x: -rxn.normal.x, y: -rxn.normal.y };
        anchor = isIn ? P(centre.x + inward.x * AXIS, centre.y + inward.y * AXIS) : P(centre.x - inward.x * AXIS, centre.y - inward.y * AXIS);
        u = isIn ? inward : rxn.normal;
      } else {
        anchor = side === 'substrate' ? a1 : a2;
        u = side === 'substrate' ? { x: -dir.x, y: -dir.y } : dir;
      }
      const here = { x: node.x, y: node.y };
      let b1, b2;
      const custom = spec.curve?.[localOf(mid)];
      const across = Math.abs(-(here.x - anchor.x) * u.y + (here.y - anchor.y) * u.x);
      const sameRing = ring && node.ring && rings[node.ring] === ring && !spec.track;
      if (custom) {
        b1 = { x: custom[0], y: custom[1] }; b2 = { x: custom[2], y: custom[3] };
      } else if (gridLabel[mid] !== undefined || (spec.merge && node.primary && !lineNodes.includes(node))) {
        // A block's precursor (or a node of a column a lump `merge`s) leaves
        // its column square to it, towards the marker, and turns once into the
        // axis end. Every branch of a column is the same curve scaled across
        // it, so they nest and never cross.
        const toward = Math.sign(anchor.x - here.x) || 1;
        const reach = Math.abs(anchor.x - here.x);
        b1 = { x: here.x + toward * reach * 0.5, y: here.y };
        b2 = { x: anchor.x + u.x * reach * 0.4, y: anchor.y + u.y * reach * 0.4 };
      } else if (sameRing) {
        const nodeAngle = ring.angles[mid];
        const span = wrap(angleOf(ring.centre, anchor) - nodeAngle);
        const h = arcHandle(span, ring.r);
        const t = tangent(nodeAngle);
        const toward = span < 0 ? -1 : 1;
        b1 = { x: here.x + t.x * toward * h, y: here.y + t.y * toward * h };
        b2 = { x: anchor.x + u.x * h, y: anchor.y + u.y * h };
      } else if (lineNodes.includes(node) && across >= 1) {
        [b1, b2] = lensControls(here, anchor, u);
      } else if (lineNodes.includes(node)) {
        [b1, b2] = straightControls(here, anchor);
      } else {
        [b1, b2] = sideControls(here, anchor, u);
      }
      segments.push({
        reactionId: rid, nodeId: node.id, kind: node.primary ? 'primary' : 'currency', coefficient: coef, side,
        from: side === 'substrate' ? P(here.x, here.y) : anchor, to: side === 'substrate' ? anchor : P(here.x, here.y),
        backboneAnchor: true, arrow: side === 'product', b1: P(b1.x, b1.y), b2: P(b2.x, b2.y),
      });
    }
    if (rxn.boundary) segments.push({ reactionId: rid, kind: 'boundary', from: a2, to: rxn.boundary, arrow: true });
  };

  for (const [sp, spec] of Object.entries(LAYOUT.species)) {
    for (const [local, rs] of Object.entries(spec.rx)) placeReaction(`${sp}@${local}`, rs, sp);
  }
  for (const [base, value] of Object.entries(LAYOUT.medium)) {
    const rid = `EX_${base}_s`;
    if (merged.reactions.has(rid)) placeReaction(rid, { medium: value.stub ?? 'S' }, null);
  }

  // What is not drawn
  const missing = [...merged.reactions.keys()].filter((id) => !drawn.has(id) && id !== 'COMMUNITY_GROWTH');
  for (const id of missing) problems.push(`not drawn: ${id}`);

  for (const n of nodes.values()) delete n.ring;

  // Labels
  const texts = (LAYOUT.texts ?? []).map(([id, text, x, y, fontSize]) => ({ id, text, x, y, fontSize }));
  placeLabels(nodes, reactions, segments, texts);

  const env = LAYOUT.environment;
  const map = {
    version: 1,
    canvas: { width: LAYOUT.canvas.width, height: LAYOUT.canvas.height },
    environment: { rect: { ...env }, id: SHARED_COMPARTMENT, label: 'Shared medium' },
    cells,
    medium: {
      nodes: [...nodes.values()].filter((n) => n.kind === 'medium').map((n) => n.id),
      exchanges: [...reactions.values()].filter((r) => r.kind === 'exchange').map((r) => r.id),
    },
    nodes: [...nodes.values()],
    reactions: [...reactions.values()],
    segments,
    texts,
  };
  checkMap(map, comm, problems);
  return { map, problems, comm };
}

// Labels: the given angle, or the first place clear of lines, nodes, markers and other labels

function labelBox(n, angle, text) {
  const secondary = n.kind === 'currency' || n.primary === false;
  const size = n.kind === 'medium' ? 15 : secondary ? 9.5 : 12.5;
  const w = text.length * size * (n.kind === 'medium' ? 0.6 : 0.56), h = size * 1.1;
  const d = (secondary ? 7 : 13) + (secondary ? 3.5 : 5);
  const c = Math.cos(angle), s = Math.sin(angle);
  const x = n.x + d * (Math.abs(c) < 1e-9 ? 0 : c), y = n.y + d * (Math.abs(s) < 1e-9 ? 0 : s);
  const left = c > 0.3 ? x : c < -0.3 ? x - w : x - w / 2;
  const top = s < -0.3 ? y - h : s > 0.3 ? y : y - h / 2;
  return { l: left, t: top, r: left + w, b: top + h };
}
const boxHitsSeg = (b, p, q, pad) => {
  for (let i = 0; i <= 6; i++) {
    const t = i / 6, x = p.x + (q.x - p.x) * t, y = p.y + (q.y - p.y) * t;
    if (x > b.l - pad && x < b.r + pad && y > b.t - pad && y < b.b + pad) return true;
  }
  return false;
};
const boxHitsCircle = (b, c, r) => {
  const x = Math.max(b.l, Math.min(c.x, b.r)), y = Math.max(b.t, Math.min(c.y, b.b));
  return Math.hypot(x - c.x, y - c.y) < r;
};
const boxesOverlap = (a, b, pad = 1.5) => a.l < b.r + pad && b.l < a.r + pad && a.t < b.b + pad && b.t < a.b + pad;

function placeLabels(nodes, reactions, segments, texts) {
  const pieces = [];
  for (const s of segments) {
    if (!s.nodeId) { pieces.push([s.from, s.to]); continue; }
    const pts = bezierPoints(s, 8);
    for (let i = 1; i < pts.length; i++) pieces.push([pts[i - 1], pts[i]]);
  }
  const placed = texts.map((t) => ({ l: t.x, t: t.y - t.fontSize, r: t.x + t.text.length * t.fontSize * 0.55, b: t.y + 4 }));
  // Reaction labels sit to the right of their marker (above or below a straight line lying on its side).
  for (const r of reactions.values()) {
    const w = (String(r.localId).length + 6) * 11 * 0.55;
    if (r.linear && Math.abs(r.axis.dir.y) < 0.5) placed.push({ l: r.x - w / 2, t: r.y - 26, r: r.x + w / 2, b: r.y - 12 });
    else placed.push({ l: r.x + 12, t: r.y - 7, r: r.x + 12 + w, b: r.y + 7 });
  }
  const users = new Map();
  for (const s of segments) if (s.nodeId) { if (!users.has(s.nodeId)) users.set(s.nodeId, []); users.get(s.nodeId).push(s.reactionId); }
  const order = [...nodes.values()].sort((a, b) => (a.kind === 'currency') - (b.kind === 'currency') || a.id.localeCompare(b.id));
  const base = [-90, 90, 0, 180, -45, -135, 45, 135].map(rad);
  for (const n of order) {
    let candidates = base;
    if (n.labelAngle !== undefined) candidates = [n.labelAngle];
    else if (n.kind === 'currency') {
      const r = reactions.get(n.reactionId);
      const away = Math.atan2(n.y - r.y, n.x - r.x);
      const snapped = Math.round(away / (Math.PI / 4)) * (Math.PI / 4);
      candidates = [snapped, ...base.filter((a) => Math.abs(Math.cos(a) - Math.cos(snapped)) + Math.abs(Math.sin(a) - Math.sin(snapped)) > 1e-6)];
    }
    let best = null;
    for (const angle of candidates) {
      const box = labelBox(n, angle, n.base);
      let score = 0;
      for (const [p, q] of pieces) if (boxHitsSeg(box, p, q, 2)) score += 3;
      for (const o of nodes.values()) if (o !== n && Math.abs(o.x - n.x) < 260 && Math.abs(o.y - n.y) < 80 && boxHitsCircle(box, o, o.r + 1.5)) score += 3;
      for (const r of reactions.values()) if (Math.abs(r.x - n.x) < 260 && boxHitsCircle(box, r, 10)) score += 3;
      for (const b of placed) if (boxesOverlap(box, b)) score += 3;
      if (!best || score < best.score) best = { score, angle, box };
      if (score === 0) break;
    }
    let a = best.angle;
    if (a >= Math.PI) a -= 2 * Math.PI;
    n.labelAngle = Math.round(a * 1e6) / 1e6;
    if (n.labelAngle >= Math.PI) n.labelAngle = -Math.PI;
    placed.push(best.box);
    if (best.score > 0) n._labelProblem = best.score;
  }
}

// Checks: every reaction drawn as the model writes it, everything in its compartment, nothing on top of anything

function checkMap(map, comm, problems) {
  const merged = comm.merged;
  const nodes = new Map(map.nodes.map((n) => [n.id, n]));
  const byRxn = new Map();
  for (const s of map.segments) if (s.nodeId) { if (!byRxn.has(s.reactionId)) byRxn.set(s.reactionId, []); byRxn.get(s.reactionId).push(s); }
  for (const r of map.reactions) {
    const model = merged.reactions.get(r.id);
    const want = Object.keys(model.metabolites).filter((m) => merged.metabolites.get(m)?.compartment !== LINK_COMPARTMENT).sort();
    const got = (byRxn.get(r.id) ?? []).map((s) => nodes.get(s.nodeId).mergedId).sort();
    if (JSON.stringify(want) !== JSON.stringify(got)) problems.push(`${r.id}: drawn ${got.join(' ')} but the model has ${want.join(' ')}`);
    if (r.bounds.lower !== model.lowerBound || r.bounds.upper !== model.upperBound) problems.push(`${r.id}: bounds differ`);
    for (const s of byRxn.get(r.id) ?? []) {
      const c = model.metabolites[nodes.get(s.nodeId).mergedId];
      if (c !== s.coefficient) problems.push(`${r.id}: coefficient of ${s.nodeId}`);
    }
  }
  const inRect = (p, k, m = 0) => p.x >= k.x + m && p.x <= k.x + k.w - m && p.y >= k.y + m && p.y <= k.y + k.h - m;
  const cells = new Map(map.cells.map((c) => [c.id, c]));
  const comp = (cellId, id) => cells.get(cellId).compartments.find((c) => c.id === id).rect;
  for (const n of map.nodes) {
    if (!n.cellId) {
      if (!inRect(n, map.environment.rect, n.r + 4) || map.cells.some((c) => inRect(n, c.rect, -n.r - 4))) problems.push(`${n.id} is not in the medium`);
    } else if (n.compartment === 'c') {
      if (!inRect(n, comp(n.cellId, 'c'), n.r + 3)) problems.push(`${n.id} is not inside the cytosol`);
    } else if (!inRect(n, comp(n.cellId, 'e'), n.r - 1) || inRect(n, comp(n.cellId, 'c'), -n.r - 1)) problems.push(`${n.id} is not in the extracellular band`);
  }
  for (const r of map.reactions) {
    if (r.onMembrane || !r.cellId) continue;
    if (!inRect(r, comp(r.cellId, r.compartments.includes('c') ? 'c' : 'e'), 10)) problems.push(`reaction ${r.id} is outside its compartment`);
  }
  const list = [...map.nodes].sort((a, b) => a.x - b.x);
  for (let i = 0; i < list.length; i++) for (let j = i + 1; j < list.length && list[j].x - list[i].x < 40; j++) {
    const a = list[i], b = list[j];
    if (Math.hypot(a.x - b.x, a.y - b.y) < a.r + b.r + 5) problems.push(`overlap: ${a.id} / ${b.id}`);
  }
  const users = new Map();
  for (const s of map.segments) if (s.nodeId) { if (!users.has(s.nodeId)) users.set(s.nodeId, new Set()); users.get(s.nodeId).add(s.reactionId); }
  for (const n of map.nodes) for (const r of map.reactions) {
    if (users.get(n.id)?.has(r.id)) continue;
    if (Math.hypot(n.x - r.x, n.y - r.y) < n.r + 15) problems.push(`${n.id} lies on ${r.id}`);
  }
  // Branches crossing branches of other reactions, and nodes crowding a branch that is not theirs
  const pieces = [];
  for (const s of map.segments) {
    if (!s.nodeId) continue;
    const pts = bezierPoints(s, 10);
    for (let i = 1; i < pts.length; i++) pieces.push({ a: pts[i - 1], b: pts[i], r: s.reactionId, n: s.nodeId });
  }
  const crossings = new Set();
  for (let i = 0; i < pieces.length; i++) for (let j = i + 1; j < pieces.length; j++) {
    const p = pieces[i], q = pieces[j];
    if (p.r === q.r || p.n === q.n) continue;
    if (Math.max(p.a.x, p.b.x) < Math.min(q.a.x, q.b.x) || Math.max(q.a.x, q.b.x) < Math.min(p.a.x, p.b.x)) continue;
    if (cross(p.a, p.b, q.a, q.b)) crossings.add([p.r, q.r].sort().join(' x '));
  }
  const crowded = new Set();
  for (const n of map.nodes) for (const p of pieces) {
    if (p.n === n.id || users.get(n.id)?.has(p.r)) continue;
    if (Math.abs(p.a.x - n.x) > 60 || Math.abs(p.a.y - n.y) > 60) continue;
    if (pointSegDist(n, p.a, p.b) < n.r + 6) crowded.add(`${n.id} on a branch of ${p.r}`);
  }
  // Titles clear of every line and node
  const titled = [];
  for (const t of map.texts) {
    const box = { l: t.x - 2, t: t.y - t.fontSize * 0.8, r: t.x + t.text.length * t.fontSize * 0.52, b: t.y + t.fontSize * 0.25 };
    const hit = pieces.some((p) => boxHitsSeg(box, p.a, p.b, 2)) || map.nodes.some((n) => boxHitsCircle(box, n, n.r + 2))
      || map.reactions.some((r) => boxHitsCircle(box, r, 12));
    if (hit) titled.push(t.id);
  }
  map._report = { crossings: [...crossings].sort(), crowded: [...crowded].sort(),
    labels: map.nodes.filter((n) => n._labelProblem).map((n) => n.id), titles: titled };
  for (const n of map.nodes) delete n._labelProblem;
}

const { map, problems, comm } = await build();
const report = map._report;
delete map._report;
const members = [...comm.species].map((s) => s.id);
console.log(`${map.reactions.length} reactions (${[...comm.merged.reactions.keys()].length - 1} in the community, COMMUNITY_GROWTH is the solver's), ${map.nodes.length} nodes, ${map.segments.length} segments`);
console.log(`${report.crossings.length} crossings, ${report.crowded.length} crowded, ${report.labels.length} labels without a clear place, ${report.titles.length} titles on a line or node`);
if (process.argv.includes('--verbose')) {
  for (const c of report.crossings) console.log('  cross', c);
  for (const c of report.crowded) console.log('  crowded', c);
  for (const c of report.labels) console.log('  label', c);
  for (const c of report.titles) console.log('  title', c);
}
for (const p of problems) console.log('  PROBLEM', p);
if (problems.length) process.exitCode = 1;
if (!CHECK_ONLY && (!problems.length || FORCE)) {
  const doc = {
    format: MAP_DOCUMENT_FORMAT,
    version: MAP_DOCUMENT_VERSION,
    metadata: {
      createdAt: null,
      scenario: `published:${COMMUNITY}`,
      species: members.map((id) => ({ id, model: `published/${COMMUNITY}/model-${id}` })),
      reactionCount: map.reactions.length,
      nodeCount: map.nodes.length,
      segmentCount: map.segments.length,
      textCount: map.texts.length,
      builder: 'tools/scenario-maps/build-stolyar.mjs',
    },
    pathwayManager: null,
    fluxColorStops: null,
    map,
  };
  await writeFile(OUT, JSON.stringify(doc, null, 1) + '\n');
  console.log(`wrote ${OUT}`);
  if (SET_MANIFEST) {
    // Point the manifest at the map, leaving the rest of it as it is.
    const manifestPath = join(DIR, 'manifest.json');
    const raw = await readFile(manifestPath, 'utf8');
    const manifest = JSON.parse(raw);
    manifest.map = 'map.json';
    const eol = raw.includes('\r\n') ? '\r\n' : '\n';
    const end = /\r?\n$/.test(raw) ? eol : '';
    await writeFile(manifestPath, JSON.stringify(manifest, null, 2).replace(/\n/g, eol) + end);
  }
}
