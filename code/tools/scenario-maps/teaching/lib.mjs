/**
 * Drawing kit for the curated maps of the teaching scenarios.
 *
 * A cell template (one per species, see the files next to this one) says in
 * the cell's own coordinates where its compartments, its primary metabolites
 * and its reactions stand. `instantiate` turns it into map nodes, reactions and
 * segments for one species at one place on the canvas, reading every equation
 * from the model, so the same template draws the same cell in every scenario.
 *
 * Conventions, as in the hand-made maps (the Yeast9 map of coxeter_mapping,
 * tools/zuniga/build-map.mjs):
 *   - a reaction is a short axis (half length AXIS) from its substrates' end
 *     (a1) to its products' end (a2); branches leave it along the axis;
 *   - a cycle is a true circle: metabolites on it, each reaction halfway
 *     between two of them with its axis along the tangent and its branches
 *     drawn as circular arcs (handles 4/3·tan(Δθ/4)·R);
 *   - a transporter stands on the membrane it crosses, its axis square to it;
 *   - every metabolite of an equation is drawn: the carbon backbone through
 *     shared primary nodes, the rest as small secondary nodes beside the axis
 *     (rows COFACTOR_ROWS, the same everywhere), so no reaction loses a
 *     participant and the map reads exactly as the model is written;
 *   - the biomass reaction stands upright with its precursors stacked in two
 *     short columns above it and what it gives back in two below, every branch
 *     a rung into the reaction's stem, so that it stays smaller than the
 *     pathways that feed it.
 * Angles are in degrees, 0 to the right and 90 down the page.
 */

import { sharedMetabolite, communityExchange } from '../../../src/core/community.js';
import {
  cellCompartmentName, cellCompartmentVisualId, membraneStyle,
} from '../../../src/core/compartments.js';
import { boundaryKey } from '../../../src/map/boundaries.js';

export const AXIS = 17;
export const PRIMARY_R = 13;
export const SECONDARY_R = 7;
export const MEDIUM_GAP = 58;      // a cell's medium node stands this far outside its boundary

// Rows of the cofactor template: (along the axis from the marker's centre,
// across from the axis), from the inside out, so that their curves nest.
const COFACTOR_ROWS = [[48, 36], [34, 62], [20, 88]];
// The same carriers take the same places on every reaction.
const COFACTOR_ORDER = ['atp', 'gtp', 'adp', 'gdp', 'amp', 'nadph', 'nadp', 'nadh', 'nad', 'q8h2', 'q8',
  'q6h2', 'q6', 'pqh2', 'pq', 'coa', 'photon', 'o2', 'co2', 'hco3', 'nh4', 'h2', 'ppi', 'pi', 'h2o', 'h'];
// The order a biomass reaction's precursors are stacked in: the carbon
// skeletons in pathway order, then nitrogen, then the carriers.
const STACK_ORDER = ['g6p', 'f6p', 'r5p', 'e4p', 'g3p', '3pg', 'pep', 'pyr', 'accoa', 'oaa', 'akg', 'glu__L',
  'gln__L', 'nh4', 'atp', 'adp', 'pi', 'h', 'nadph', 'nadp', 'nad', 'nadh', 'coa', 'h2o'];
// Metabolites drawn as secondary nodes unless a reaction asks for its primary.
export const CURRENCY = new Set(['atp', 'adp', 'amp', 'gtp', 'gdp', 'nad', 'nadh', 'nadp', 'nadph', 'h', 'h2o',
  'pi', 'ppi', 'co2', 'coa', 'q8', 'q8h2', 'q6', 'q6h2', 'pq', 'pqh2', 'o2', 'nh4', 'hco3', 'photon', 'h2']);

const COMPASS = {
  N: [0, -1], S: [0, 1], E: [1, 0], W: [-1, 0],
  NE: [1, -1], NW: [-1, -1], SE: [1, 1], SW: [-1, 1],
};
const NORMALS = { top: [0, -1], bottom: [0, 1], left: [-1, 0], right: [1, 0] };

// Geometry

const r2 = (v) => Math.round(v * 100) / 100;
export const pt = (x, y) => ({ x: r2(x), y: r2(y) });
const unit = (dx, dy) => { const l = Math.hypot(dx, dy) || 1; return [dx / l, dy / l]; };
const lerp = (a, b, t) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
const wrap = (a) => { let x = a; while (x > 180) x -= 360; while (x <= -180) x += 360; return x; };
const rad = (d) => (d * Math.PI) / 180;
export const onCircle = (c, r, a) => [c[0] + r * Math.cos(rad(a)), c[1] + r * Math.sin(rad(a))];
const tangent = (a) => [-Math.sin(rad(a)), Math.cos(rad(a))];
const angleOf = (c, p) => (Math.atan2(p[1] - c[1], p[0] - c[0]) * 180) / Math.PI;
const arcHandle = (span, r) => (4 / 3) * Math.tan(rad(Math.abs(span)) / 4) * r;
const baseOf = (mid) => mid.replace(/_[a-z]+$/, '');

/** Label angle in radians from a compass point or a number of degrees. */
function labelAngle(spec) {
  if (spec === undefined || spec === null) return undefined;
  if (typeof spec === 'number') return rad(spec);
  const [x, y] = COMPASS[spec];
  return Math.atan2(y, x);
}

/** Handles on the straight line from the metabolite to the reaction. */
function straight(node, anchor) { return [lerp(node, anchor, 0.34), lerp(node, anchor, 0.72)]; }

/**
 * Coxeter's shapeBranch(): out of the reaction along its axis (u points away
 * from the marker), then square across to a metabolite standing beside it.
 */
function sideCurve(node, anchor, u) {
  const vx = node[0] - anchor[0], vy = node[1] - anchor[1];
  const along = vx * u[0] + vy * u[1];
  const side = -vx * u[1] + vy * u[0];
  const across = Math.abs(side);
  if (across < 4 && along > 0) return straight(node, anchor);
  const hand = side < 0 ? -1 : 1;
  const n = [-u[1] * hand, u[0] * hand];
  const reach = Math.max(along, 0.5 * across, 14);
  return [[node[0] - n[0] * across * 0.55, node[1] - n[1] * across * 0.55],
    [anchor[0] + u[0] * reach * 0.8, anchor[1] + u[1] * reach * 0.8]];
}

/**
 * A primary metabolite ahead of the reaction and off its axis (a fork): out of
 * the marker along the axis for a short way, then on a gentle single bend
 * straight at the node, so two such branches part like a Y.
 */
function forkCurve(node, anchor, u) {
  const k = Math.min(Math.hypot(node[0] - anchor[0], node[1] - anchor[1]) * 0.3, 44);
  const b2 = [anchor[0] + u[0] * k, anchor[1] + u[1] * k];
  return [lerp(node, b2, 0.35), b2];
}

/**
 * A stacked biomass metabolite: a rung straight across to the reaction's stem
 * at the node's own height, then along the stem into the axis, so the branches
 * of one column join the stem in order and never cross.
 */
function rungCurve(node, anchor, u) {
  const along = (node[0] - anchor[0]) * u[0] + (node[1] - anchor[1]) * u[1];
  const onStem = [anchor[0] + u[0] * along, anchor[1] + u[1] * along];
  return [onStem, [anchor[0] + u[0] * along * 0.45, anchor[1] + u[1] * along * 0.45]];
}

/** Handles of the circular arc of radius `R` round `centre` from `p` to `q` (the shorter way). */
export function arcHandles(centre, R, p, q) {
  const ap = angleOf(centre, p), aq = angleOf(centre, q);
  const span = wrap(aq - ap);
  const h = arcHandle(span, R);
  const s = span < 0 ? -1 : 1;
  const tp = tangent(ap), tq = tangent(aq);
  return [[p[0] + tp[0] * s * h, p[1] + tp[1] * s * h], [q[0] - tq[0] * s * h, q[1] - tq[1] * s * h]];
}

/** A line metabolite beside the axis (a reaction drawn as an arc beside its line). */
function arcCurve(node, anchor, u) {
  const vx = node[0] - anchor[0], vy = node[1] - anchor[1];
  const along = vx * u[0] + vy * u[1];
  const b2 = [anchor[0] + u[0] * along * 0.5, anchor[1] + u[1] * along * 0.5];
  return [lerp(node, b2, 0.45), b2];
}

// Templates

/**
 * The drawing of one species' cell, in its own coordinates (0,0 is the top
 * left of the cell). Built by a template file and read by `instantiate`.
 */
export class Template {
  constructor(model, { width, height }) {
    this.model = model;
    this.width = width;
    this.height = height;
    this.boxes = new Map();      // compartment id -> {x, y, w, h, ...extra}
    this.nodes = new Map();      // key -> {mid, x, y, label}
    this.reactions = new Map();  // reaction id -> spec
    this.rings = new Map();
    this.ringInfo = new Map();
    this.texts = [];
    this.exchangeSides = new Map();  // EX reaction id -> edge
  }

  /** A compartment's box. The cell's outer compartment is the whole cell. */
  box(id, x, y, w, h, extra = {}) { this.boxes.set(id, { x, y, w, h, ...extra }); return this; }

  /** A primary metabolite node; `key` names a second drawing of the same metabolite. */
  met(mid, x, y, { label = 'N', key = mid } = {}) {
    if (!this.model.metabolites.has(mid)) throw new Error(`${this.model.id}: no metabolite ${mid}`);
    if (this.nodes.has(key)) throw new Error(`${this.model.id}: node ${key} declared twice`);
    this.nodes.set(key, { key, mid, x, y, label });
    return this;
  }

  node(key) { return this.nodes.get(key); }

  /**
   * A reaction. `spec` is one of
   *   {line: [from, to], t, bend, offset}  on the line between two primary nodes
   *   {at: [x, y], dir: [dx, dy]}           at an absolute place
   *   {edge, along}                         a transporter on its membrane
   * and may add `stack` (biomass: every metabolite secondary, stacked in two
   * columns either side of the stem, see STACK_ORDER), `cof` (compass side of
   * its own cofactors), `use` ({metabolite: node key} links beyond the
   * default), `sec` (metabolites kept secondary),
   * `pos` ({metabolite: [x, y]} for a secondary node), `curves` ({metabolite:
   * [[b1], [b2]]}), `label` (reaction label nudges are not supported: keep room).
   */
  rxn(id, spec) {
    if (!this.model.reactions.has(id)) throw new Error(`${this.model.id}: no reaction ${id}`);
    if (this.reactions.has(id)) throw new Error(`${this.model.id}: reaction ${id} declared twice`);
    this.reactions.set(id, { id, ...spec });
    return this;
  }

  /**
   * A cycle drawn as a circle. `cycle` alternates metabolites (node keys) and
   * reactions, closing back on the first metabolite; `anchor: [key, angle]`
   * and `turn` (1 clockwise on the page, -1 against) or `angles` place the
   * metabolites; `inside` lists reactions whose cofactors stand inside;
   * `radius` defaults to the circumference `step` per step.
   */
  ring(name, { centre, radius, cycle, anchor, turn = 1, angles = null, inside = [], step = 160, label = {}, open = false }) {
    const mets = cycle.filter((_, i) => i % 2 === 0);
    const rxns = cycle.filter((_, i) => i % 2 === 1);
    const steps = rxns.length + (open ? 1 : 0);
    const R = radius ?? (Math.max(steps, mets.length) * step) / (2 * Math.PI);
    this.ringInfo.set(name, { centre, R });
    const at = new Map();
    if (angles) for (const k of mets) at.set(k, angles[k]);
    else {
      const [key, a0] = anchor;
      const k0 = mets.indexOf(key);
      mets.forEach((k, i) => at.set(k, a0 + (turn * (i - k0) * 360) / Math.max(steps, mets.length)));
    }
    for (const k of mets) {
      const [x, y] = onCircle(centre, R, at.get(k));
      const mid = k.includes('#') ? k.split('#')[0] : k;
      this.met(mid, x, y, { key: k, label: label[k] ?? at.get(k) });
    }
    rxns.forEach((rid, i) => {
      const a = mets[i], b = mets[(i + 1) % mets.length];
      this.rings.set(rid, { name, centre, R, a, b, inside: inside.includes(rid) });
      const extra = this.reactions.get(rid);
      if (extra) this.reactions.delete(rid);
      this.rxn(rid, { ...(extra ?? {}), ring: name });
    });
    return R;
  }

  /**
   * A circle through nodes already placed, for reactions drawn along it with
   * arcRxn(): their branches to nodes on it are arcs of it.
   */
  circle(name, centre, R) {
    this.ringInfo.set(name, { centre, R });
    return this;
  }

  /**
   * A reaction on a circle concentric with a ring, beside the step between
   * metabolites `a` and `b` it runs parallel to (outside for a positive
   * `offset`, inside for a negative one); its cofactors stand on its far side.
   */
  track(id, ringName, [a, b], offset, spec = {}) {
    const info = this.ringInfo.get(ringName);
    this.rings.set(id, { name: ringName, centre: info.centre, R: info.R, a, b, inside: offset < 0 });
    this.rxn(id, { ...spec, ring: ringName, track: offset });
    return this;
  }

  /**
   * A reaction standing on a ring's circle at `angle`, its axis along the
   * tangent the way `sense` says (1 clockwise on the page); branches to nodes on
   * the same circle are arcs of it. For a chain that leaves a ring.
   */
  arcRxn(id, ringName, angle, sense, spec = {}) {
    const info = this.ringInfo.get(ringName);
    this.rings.set(id, { name: ringName, centre: info.centre, R: info.R, a: null, b: null, angle, sense, inside: Boolean(spec.inside) });
    this.rxn(id, { ...spec, ring: ringName });
    return this;
  }

  /** Extra settings for a ring reaction declared by ring(). */
  ringRxn(id, spec) {
    const r = this.reactions.get(id);
    if (!r?.ring) throw new Error(`${id} is not on a ring`);
    Object.assign(r, spec);
    return this;
  }

  /**
   * A metabolite carried across the envelope: one node in each compartment it
   * passes through, and its transporters on the membranes between them, all on
   * one straight line square to `edge` at `along`. `chain` lists the reactions
   * from the inside out, the cell's exchange last; `mets` the metabolite ids
   * from the inside out (a node already declared is reused where it is); `depth`
   * is how far inside its compartment the innermost node stands, when new.
   */
  column({ edge, along, chain, mets, depth = 70, label }) {
    const n = NORMALS[edge];
    const horizontal = edge === 'top' || edge === 'bottom';
    const place = (offsetFromEdge, box) => {
      // A point at `along` on the line square to `edge`, `offsetFromEdge` inside `box`'s edge.
      const e = edge === 'top' ? box.y : edge === 'bottom' ? box.y + box.h : edge === 'left' ? box.x : box.x + box.w;
      const v = e - n[0] * offsetFromEdge - n[1] * offsetFromEdge;
      return horizontal ? [along, v] : [v, along];
    };
    const midOf = (key) => this.nodes.get(key)?.mid ?? key;
    mets.forEach((mid, i) => {
      if (this.nodes.has(mid)) return;
      const comp = this.model.metabolites.get(mid)?.compartment;
      const box = this.boxes.get(comp);
      if (!box) throw new Error(`${this.model.id}: no box for ${comp} (${mid})`);
      let p;
      if (i === 0) p = place(depth, box);
      else {
        // Halfway across the band between this compartment's edge and the next one in.
        // The band of this compartment: from its own edge in to the edge of its
        // child that holds the previous node.
        const tree = this.model.compartmentTree().byId;
        let child = this.model.metabolites.get(midOf(mets[i - 1])).compartment;
        while (tree.get(child)?.parent && tree.get(child).parent !== comp) child = tree.get(child).parent;
        const inner = this.boxes.get(child);
        const outerEdge = edgeCoord(box, edge), innerEdge = edgeCoord(inner, edge);
        const mid2 = (outerEdge + innerEdge) / 2;
        p = horizontal ? [along, mid2] : [mid2, along];
      }
      this.met(mid, p[0], p[1], { label: label ?? (horizontal ? 'E' : 'N') });
    });
    for (const rid of chain) {
      const prev = this.reactions.get(rid);
      this.reactions.delete(rid);
      this.rxn(rid, { ...(prev ?? {}), edge, along, colMets: mets });
      if (this.model.reactions.get(rid).kind === 'exchange') this.exchangeSides.set(rid, edge);
    }
    return this;
  }

  /** A module title and its one-line purpose, left end of the baseline at (x, y). */
  title(id, text, purpose, x, y, { size = 30, small = 19 } = {}) {
    this.texts.push({ id: `pathway:${id}`, text, x, y, fontSize: size });
    if (purpose) this.texts.push({ id: `purpose:${id}`, text: purpose, x, y: y + Math.round(size * 0.95), fontSize: small });
    return this;
  }
}

function edgeCoord(box, edge) {
  return edge === 'top' ? box.y : edge === 'bottom' ? box.y + box.h : edge === 'left' ? box.x : box.x + box.w;
}

// Instantiation

/**
 * Draw a template for species `sp` with its top left at (ox, oy).
 *
 * @param {Template} tpl
 * @param {object} ctx `sp` (species id), `ox`, `oy`, `community` (the merged
 *   CommunityModel), `abundance`, `mediumMode` ('own': the cell's medium node
 *   is the shared one, with the community exchange beside it; 'copy': a node of
 *   the cell's own that the cross-feeding lines start from)
 * @returns {{cell, nodes: Map, reactions: Map, segments: object[], texts: object[], medium: object[]}}
 */
export function instantiate(tpl, ctx) {
  const { sp, ox, oy, community } = ctx;
  const model = tpl.model;
  const merged = community.merged;
  const X = (p) => [p[0] + ox, p[1] + oy];
  const nodes = new Map();
  const reactions = new Map();
  const segments = [];
  const medium = [];

  // Shell: every compartment the model has, nested as its tree says.
  const tree = model.compartmentTree();
  const names = model.compartmentNames;
  const compartments = [];
  const subtree = new Map();
  const walk = (node) => {
    const set = new Set([node.id]);
    for (const ch of node.children) for (const id of walk(ch)) set.add(id);
    subtree.set(node.id, set);
    return set;
  };
  tree.roots.forEach(walk);
  for (const node of [...tree.byId.values()].sort((a, b) => a.depth - b.depth)) {
    const b = tpl.boxes.get(node.id);
    if (!b) throw new Error(`${model.id}: no box for compartment ${node.id}`);
    const visualId = cellCompartmentVisualId(node.id, names);
    const { x, y, w, h, ...extra } = b;
    compartments.push({
      id: node.id, rect: { x: x + ox, y: y + oy, w, h }, depth: node.depth, parentId: node.parent ?? null,
      label: cellCompartmentName(node.id, names), membrane: membraneStyle(visualId), visualId, ...extra,
    });
  }
  const outer = tree.roots[0].id;
  const cellRect = compartments.find((c) => c.id === outer).rect;
  const cell = {
    id: sp, name: model.cell?.name ?? sp, organism: model.cell?.organism, note: model.cell?.note,
    color: model.cell?.color, abundance: ctx.abundance, rect: { ...cellRect }, compartments, outerCompartment: outer,
  };
  const rectOf = (cid) => compartments.find((c) => c.id === cid)?.rect;

  // Primary nodes
  const keyId = (key) => {
    const n = tpl.nodes.get(key);
    return key === n.mid ? `met:${sp}@${n.mid}` : `met:${sp}@${n.mid}#${key.split('#')[1] ?? key}`;
  };
  const metInfo = (mid) => merged.metabolites.get(`${sp}@${mid}`);
  for (const [key, n] of tpl.nodes) {
    const m = metInfo(n.mid);
    const [x, y] = X([n.x, n.y]);
    nodes.set(keyId(key), {
      id: keyId(key), kind: 'metabolite', mergedId: `${sp}@${n.mid}`, localId: n.mid, base: m.base, name: m.name,
      formula: m.formula, compartment: m.compartment, cellId: sp, x: r2(x), y: r2(y), r: PRIMARY_R,
      primary: true, visualRole: 'primary', labelAngle: labelAngle(n.label),
    });
  }

  const nodeOf = (key) => nodes.get(keyId(key));
  const xy = (n) => [n.x, n.y];

  for (const [rid, spec] of tpl.reactions) {
    const rxn = model.reactions.get(rid);
    const mergedId = `${sp}@${rid}`;
    const mrxn = merged.reactions.get(mergedId);
    if (!mrxn) throw new Error(`${mergedId} is not in the community model`);
    const ring = spec.ring ? tpl.rings.get(rid) : null;
    const isExchange = rxn.kind === 'exchange';
    const onMembrane = Boolean(spec.edge);
    // The membrane a transporter sits on: the innermost compartment it touches,
    // as the automatic layout decides it; an exchange sits on the cell boundary.
    let membrane = null;
    if (onMembrane) {
      membrane = isExchange ? outer : rxn.compartments.reduce((best, c) =>
        (tree.byId.get(c).depth > tree.byId.get(best).depth ? c : best), rxn.compartments[0]);
    }

    // Which drawn node each metabolite goes to
    const links = new Map();   // mid -> node id
    const use = spec.use ?? {};
    const sec = new Set(spec.sec ?? []);
    for (const mid of Object.keys(rxn.metabolites)) {
      if (sec.has(mid) || (spec.stack && !use[mid])) continue;
      if (use[mid]) { links.set(mid, keyId(use[mid])); continue; }
      if (spec.line && spec.line.some((k) => tpl.nodes.get(k)?.mid === mid)) {
        links.set(mid, keyId(spec.line.find((k) => tpl.nodes.get(k).mid === mid)));
        continue;
      }
      if (ring) {
        const k = [ring.a, ring.b].filter(Boolean).find((kk) => tpl.nodes.get(kk).mid === mid);
        if (k) { links.set(mid, keyId(k)); continue; }
      }
      if (onMembrane && spec.colMets) {
        const key = spec.colMets.find((k) => tpl.nodes.get(k)?.mid === mid);
        if (key) links.set(mid, keyId(key));
        continue;
      }
      if (!CURRENCY.has(baseOf(mid)) && tpl.nodes.has(mid)) links.set(mid, keyId(mid));
    }

    // Centre and axis
    let c, d, normal = null;
    if (spec.line) {
      const A = xy(nodeOf(spec.line[0])), B = xy(nodeOf(spec.line[1]));
      d = unit(B[0] - A[0], B[1] - A[1]);
      // The GEM writes some steps against the drawn line.
      const aMid = tpl.nodes.get(spec.line[0]).mid;
      if (rxn.metabolites[aMid] > 0) d = [-d[0], -d[1]];
      c = lerp(A, B, spec.t ?? 0.5);
      if (spec.bend) {
        const s = COMPASS[spec.bend];
        const p = [-d[1], d[0]];
        const sign = p[0] * s[0] + p[1] * s[1] >= 0 ? 1 : -1;
        const off = spec.offset ?? 46;
        c = [c[0] + p[0] * off * sign, c[1] + p[1] * off * sign];
      }
    } else if (ring) {
      let mid, sense;
      if (ring.a) {
        const A = xy(nodeOf(ring.a)), B = xy(nodeOf(ring.b));
        const ca = angleOf(ring.centre.map((v, i) => v + (i ? oy : ox)), A);
        const cb = angleOf(ring.centre.map((v, i) => v + (i ? oy : ox)), B);
        mid = ca + wrap(cb - ca) / 2;
        sense = wrap(cb - ca) < 0 ? -1 : 1;
        if (rxn.metabolites[tpl.nodes.get(ring.a).mid] > 0) sense = -sense;
      } else {
        mid = ring.angle;
        sense = ring.sense;
      }
      const t = tangent(mid);
      const R = ring.R + (spec.track ?? 0);
      c = onCircle([ring.centre[0] + ox, ring.centre[1] + oy], R, mid);
      d = [t[0] * sense, t[1] * sense];
      ring.at = { angle: mid, R, sense, centre: [ring.centre[0] + ox, ring.centre[1] + oy] };
    } else if (spec.at) {
      c = X(spec.at);
      d = unit(...spec.dir);
    } else if (onMembrane) {
      const box = rectOf(membrane);
      normal = NORMALS[spec.edge];
      const e = edgeCoord(box, spec.edge);
      const along = spec.along + (spec.edge === 'top' || spec.edge === 'bottom' ? ox : oy);
      c = spec.edge === 'top' || spec.edge === 'bottom' ? [along, e] : [e, along];
      if (spec.shift) c = [c[0] + spec.shift[0], c[1] + spec.shift[1]];
      // The axis runs from the substrates' side to the products'.
      const inside = (mid) => subtree.get(membrane).has(model.metabolites.get(mid).compartment);
      const subs = Object.entries(rxn.metabolites).filter(([, v]) => v < 0).map(([m]) => m);
      const keyMet = subs.find((m) => links.has(m)) ?? subs[0];
      d = isExchange || !inside(keyMet) ? (isExchange ? normal : [-normal[0], -normal[1]]) : normal;
    } else {
      throw new Error(`${mergedId}: no placement`);
    }
    const a1 = [c[0] - d[0] * AXIS, c[1] - d[1] * AXIS];
    const a2 = [c[0] + d[0] * AXIS, c[1] + d[1] * AXIS];
    const compartmentsOf = [...new Set(Object.keys(rxn.metabolites).map((m) => model.metabolites.get(m).compartment))].sort();
    const r = {
      id: mergedId, localId: rid, name: mrxn.name, subsystem: mrxn.subsystem, cellId: sp,
      kind: isExchange ? 'transfer' : rxn.kind,
      compartments: isExchange ? [outer, 's'] : compartmentsOf,
      membrane, membraneVisualId: membrane ? cellCompartmentVisualId(membrane, names) : null,
      onMembrane, normal: normal ? { x: normal[0], y: normal[1] } : null,
      boundaryId: membrane ? boundaryKey(sp, membrane) : null,
      x: r2(c[0]), y: r2(c[1]), equation: mrxn.equation(),
      bounds: { lower: mrxn.lowerBound, upper: mrxn.upperBound },
      axis: { a1: pt(...a1), a2: pt(...a2), dir: { x: r2(d[0] * 1e4) / 1e4, y: r2(d[1] * 1e4) / 1e4 } },
    };
    if (spec.line && !spec.bend && (spec.t ?? 0.5) > 0) {
      r.linear = { from: keyId(spec.line[0]), to: keyId(spec.line[1]), t: spec.t ?? 0.5 };
    }
    reactions.set(mergedId, r);
    segments.push({ reactionId: mergedId, kind: 'axis', from: pt(...a1), to: pt(...a2), arrow: false });

    // The medium side of a cell's exchange
    if (isExchange) {
      const mid = Object.keys(rxn.metabolites)[0];
      const base = model.metabolites.get(mid).base;
      const shared = sharedMetabolite(base);
      const id = ctx.mediumMode === 'own' ? `met:${shared}` : `met:${mergedId}:${shared}`;
      const p = [c[0] + normal[0] * (AXIS + MEDIUM_GAP), c[1] + normal[1] * (AXIS + MEDIUM_GAP)];
      const info = merged.metabolites.get(shared);
      nodes.set(id, {
        id, kind: 'medium', mergedId: shared, localId: base, base, name: info.name, formula: info.formula,
        compartment: 's', cellId: null, x: r2(p[0]), y: r2(p[1]), r: PRIMARY_R, primary: true, visualRole: 'primary',
        labelAngle: labelAngle(spec.mediumLabel ?? (normal[1] === 0 ? 'N' : 'E')),
        ...(ctx.mediumMode === 'own' ? {} : { reactionLocalFor: mergedId, reactionLocal: true }),
      });
      links.set(`M@${base}`, id);
      medium.push({ base, nodeId: id, normal, at: p });
    }

    // Branches
    const placed = { substrate: [], product: [] };
    const ownSecondary = [];
    for (const [mid0, coef] of Object.entries(mrxn.metabolites)) {
      const met = merged.metabolites.get(mid0);
      if (met.compartment === 'link') continue;
      const local = mid0.startsWith(`${sp}@`) ? mid0.slice(sp.length + 1) : mid0;
      const side = coef < 0 ? 'substrate' : 'product';
      const nodeId = links.get(local);
      if (!nodeId) { ownSecondary.push({ local, mid0, coef, side, met }); continue; }
      placed[side].push({ local, nodeId, coef });
    }

    // Secondary nodes, by the template unless placed by hand
    const p = [-d[1], d[0]];
    let hand = 1;
    if (ring) hand = ring.inside ? -1 : 1;
    else if (spec.cof) {
      const s = COMPASS[spec.cof];
      hand = (onMembrane ? (-normal[1] * s[0] + normal[0] * s[1]) : (p[0] * s[0] + p[1] * s[1])) >= 0 ? 1 : -1;
    }
    const order = spec.stack ? STACK_ORDER : COFACTOR_ORDER;
    const rank = (b) => { const i = order.indexOf(b); return i < 0 ? order.length : i; };
    const groups = new Map();
    for (const item of ownSecondary) {
      let g;
      if (onMembrane) g = subtree.get(membrane).has(item.met.compartment) ? 'in' : 'out';
      else g = item.side;
      if (!groups.has(g)) groups.set(g, []);
      groups.get(g).push(item);
    }
    for (const [g, list] of groups) {
      list.sort((a, b) => rank(a.met.base) - rank(b.met.base) || a.local.localeCompare(b.local));
      list.forEach((item, i) => {
        let row = i, h = hand;
        if (row >= COFACTOR_ROWS.length) { row -= COFACTOR_ROWS.length; h = -hand; }
        const [along, across] = COFACTOR_ROWS[Math.min(row, COFACTOR_ROWS.length - 1)];
        const extraAcross = i >= 2 * COFACTOR_ROWS.length ? 26 * (i - 2 * COFACTOR_ROWS.length + 1) : 0;
        let q;
        if (spec.pos?.[item.local]) q = X(spec.pos[item.local]);
        else if (spec.stack) {
          // Biomass: two columns either side of the stem, filled left column
          // first, top to bottom; the substrates stand behind the axis' first
          // end and the products beyond its second, `lead` from it.
          const { step = 22, dx = 40, lead = 34 } = spec.stack;
          const sign = g === 'substrate' ? -1 : 1;
          const end = sign < 0 ? a1 : a2;
          const rows = Math.ceil(list.length / 2);
          const col = i < rows ? 0 : 1;
          const row = col ? i - rows : i;
          // Rows run away from the axis from the one nearest it.
          const k = sign < 0 ? rows - 1 - row : row;
          const out = lead + k * step;
          const side = col ? 1 : -1;
          q = [end[0] + d[0] * out * sign - p[0] * dx * side, end[1] + d[1] * out * sign - p[1] * dx * side];
        }
        else if (onMembrane) {
          const k = g === 'out' ? 1 : -1;
          const pp = [-normal[1], normal[0]];
          // A side that is a thin band (the intermembrane space, or the cytosol
          // beyond it) takes its cofactors at the depth given for it.
          const depth = (g === 'out' ? spec.outAlong : spec.inAlong) ?? along;
          q = [c[0] + normal[0] * depth * k + pp[0] * (across + extraAcross) * h,
            c[1] + normal[1] * depth * k + pp[1] * (across + extraAcross) * h];
        } else if (ring) {
          const { angle, R, sense, centre } = ring.at;
          const sign = g === 'substrate' ? -1 : 1;
          const turn = (along / R) * (180 / Math.PI) * sign * sense;
          q = onCircle(centre, R + (across + extraAcross) * h, angle + turn);
        } else {
          const sign = g === 'substrate' ? -1 : 1;
          q = [c[0] + d[0] * along * sign + p[0] * (across + extraAcross) * h,
            c[1] + d[1] * along * sign + p[1] * (across + extraAcross) * h];
        }
        const id = `cur:${mergedId}:${item.mid0}`;
        const away = Math.atan2(q[1] - c[1], q[0] - c[0]);
        nodes.set(id, {
          id, kind: 'currency', mergedId: item.mid0, localId: item.local, base: item.met.base, name: item.met.name,
          formula: item.met.formula, compartment: item.met.compartment, cellId: sp, reactionId: mergedId,
          x: r2(q[0]), y: r2(q[1]), r: SECONDARY_R, primary: false, visualRole: 'secondary',
          labelAngle: spec.secLabel?.[item.local] !== undefined ? labelAngle(spec.secLabel[item.local])
            : spec.stack ? stackLabel(q, c, p) : snapAngle(away),
        });
        placed[item.side].push({ local: item.local, nodeId: id, coef: item.coef, secondary: true });
      });
    }

    for (const side of ['substrate', 'product']) {
      for (const item of placed[side]) {
        const n = nodes.get(item.nodeId);
        const here = [n.x, n.y];
        let anchor, u;
        if (onMembrane) {
          const inside = n.kind !== 'medium' && subtree.get(membrane).has(n.compartment);
          anchor = inside ? [c[0] - normal[0] * AXIS, c[1] - normal[1] * AXIS] : [c[0] + normal[0] * AXIS, c[1] + normal[1] * AXIS];
          u = inside ? [-normal[0], -normal[1]] : normal;
        } else if (side === 'substrate') { anchor = a1; u = [-d[0], -d[1]]; }
        else { anchor = a2; u = d; }
        let b1, b2;
        const custom = spec.curves?.[item.local];
        const onLine = spec.line && [nodeOf(spec.line[0]).id, nodeOf(spec.line[1]).id].includes(item.nodeId);
        const member = ring && ring.a && [keyId(ring.a), keyId(ring.b)].includes(item.nodeId);
        // A node on the reaction's own circle is reached along the circle.
        const onCircleNow = ring && Math.abs(Math.hypot(here[0] - ring.at.centre[0], here[1] - ring.at.centre[1]) - ring.at.R) < 1;
        if (custom) { b1 = X(custom[0]); b2 = X(custom[1]); }
        else if (spec.fork?.includes(item.local)) [b1, b2] = forkCurve(here, anchor, u);
        else if (spec.stack && item.secondary) [b1, b2] = rungCurve(here, anchor, u);
        else if (onCircleNow && !item.secondary) {
          const { centre, R } = ring.at;
          const na = angleOf(centre, here);
          const span = wrap(angleOf(centre, anchor) - na);
          const hl = arcHandle(span, R);
          const t = tangent(na);
          const toward = span < 0 ? -1 : 1;
          b1 = [here[0] + t[0] * toward * hl, here[1] + t[1] * toward * hl];
          b2 = [anchor[0] + u[0] * hl, anchor[1] + u[1] * hl];
        } else if ((onLine && spec.bend) || (spec.track && member && !item.secondary)) [b1, b2] = arcCurve(here, anchor, u);
        else if (onLine || spec.stack) [b1, b2] = straight(here, anchor);
        else [b1, b2] = sideCurve(here, anchor, u);
        const seg = {
          reactionId: mergedId, kind: item.secondary ? 'currency' : 'primary', nodeId: item.nodeId,
          coefficient: item.coef, side, arrow: side === 'product',
          from: side === 'substrate' ? pt(...here) : pt(...anchor),
          to: side === 'substrate' ? pt(...anchor) : pt(...here),
          b1: pt(...b1), b2: pt(...b2), backboneAnchor: true,
        };
        segments.push(seg);
      }
    }
  }

  // Every reaction of the model, and nothing it does not have
  const missing = [...model.reactions.keys()].filter((id) => !tpl.reactions.has(id));
  const texts = tpl.texts.map((t) => ({ ...t, id: `${sp}:${t.id}`, x: t.x + ox, y: t.y + oy }));
  return { cell, nodes, reactions, segments, texts, medium, missing };
}

/** A stacked metabolite's label, on the far side of its column from the stem (`p` across the axis). */
function stackLabel(q, c, p) {
  const s = (q[0] - c[0]) * p[0] + (q[1] - c[1]) * p[1] >= 0 ? 1 : -1;
  return snapAngle(Math.atan2(p[1] * s, p[0] * s));
}

/** Label straight out from a secondary node, snapped to the nearest of eight directions. */
function snapAngle(a) {
  const step = Math.PI / 4;
  return Math.round(a / step) * step;
}

/**
 * The community exchange of a medium pool: a short stub from the node out to
 * the edge of the system, as the automatic layout draws it.
 */
export function communityExchangeGeometry(community, base, node, direction) {
  const id = communityExchange(base);
  const rxn = community.merged.reactions.get(id);
  if (!rxn) return null;
  const [nx, ny] = direction;
  const c = [node.x + nx * 48, node.y + ny * 48];
  const a1 = [c[0] - nx * AXIS, c[1] - ny * AXIS];
  const a2 = [c[0] + nx * AXIS, c[1] + ny * AXIS];
  const boundary = [node.x + nx * 96, node.y + ny * 96];
  const reaction = {
    id, localId: id, name: rxn.name, subsystem: 'Community exchange', cellId: null, kind: 'exchange',
    compartments: ['s'], membrane: null, onMembrane: false, normal: { x: nx, y: ny },
    x: r2(c[0]), y: r2(c[1]), equation: rxn.equation(), bounds: { lower: rxn.lowerBound, upper: rxn.upperBound },
    boundary: pt(...boundary), axis: { a1: pt(...a1), a2: pt(...a2), dir: { x: nx, y: ny } },
  };
  const segments = [
    { reactionId: id, kind: 'axis', from: pt(...a1), to: pt(...a2), arrow: false },
    {
      reactionId: id, kind: 'primary', nodeId: node.id, coefficient: -1, side: 'substrate', arrow: false,
      from: pt(node.x, node.y), to: pt(...a1),
      b1: pt(...lerp([node.x, node.y], a1, 0.34)), b2: pt(...lerp([node.x, node.y], a1, 0.72)), backboneAnchor: true,
    },
    { reactionId: id, kind: 'boundary', from: pt(...a2), to: pt(...boundary), arrow: true },
  ];
  return { reaction, segments };
}
