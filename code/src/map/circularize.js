/**
 * Lay a cycle of selected reactions out as a circle.
 *
 * Selected reactions close a cycle when their main metabolites join them end
 * to end and back to the start, as the TCA and Calvin cycles do. Metabolites
 * are matched by what they are (`mergedId`), not by which copy is drawn, so a
 * cycle whose metabolites are drawn more than once is still found; the copies
 * the cycle runs through are joined into one where the circle needs it.
 *
 * The circle puts the cycle's metabolites and reactions in turn at equal
 * steps, so it grows with the cycle. Metabolite names are written outside it,
 * reaction names inside, and every selected reaction's curves are tidied, the
 * cycle's own running along the circle. Geometry only: the model is untouched.
 */

import { isSecondaryNode, metaboliteLabelAngle } from './metabolite-label.js';
import { clampNodePosition, nodeFitsAt, updateConnectedSegments } from '../layout/drag.js';
import {
  moveReaction, setReactionAngle, setReactionLinear, straightenLinearReactions, tidyReaction,
} from '../layout/reaction-edit.js';

/**
 * The arc between two neighbours on the circle, a metabolite and a reaction:
 * what a hand-drawn TCA cycle of nine steps uses at a radius of 240.
 */
const RING_STEP = 84;
/** The smallest circle, so a short cycle still has room inside for its names. */
const RING_MIN_RADIUS = 140;
/**
 * How far outside the circle a second reaction between the same two
 * metabolites stands, and the next one twice as far: clear of the first
 * one's cofactors.
 */
const PARALLEL_OUT = 90;
/** How far out from its metabolite a spoke's marker stands; its other metabolites stand twice as far. */
const SPOKE = 90;
/** How far outside the circle a cycle reaction's other main metabolites stand. */
const EXTRA_OUT = 90;
/** How much further out each further one of those stands, a metabolite and its name apart. */
const EXTRA_STEP = 44;
/** How far inside the circle a reaction's name is written: clear of its inner cofactors. */
const NAME_IN = 48;
/** How far out from its marker the name of a reaction outside the circle is written. */
const NAME_OUT = 28;
/** Room kept between the circle and its compartment's edge when the circle has to be moved to fit. */
const ROOM_PAD = 30;
/** The grid the circle's middle is tried on when it has to be moved: half a metabolite's name apart. */
const ROOM_GRID = 40;
/** A selection with more reactions is not searched for a cycle: the search grows fast with their number. */
const MAX_REACTIONS = 80;
/** How many steps the cycle search takes at most, about a tenth of a second, before it keeps the longest found. */
const SEARCH_BUDGET = 200000;

/** Why a selection was not laid out as a circle. */
export const CIRCLE_REFUSAL = Object.freeze({
  cycle: 'cycle',
  room: 'room',
});

/**
 * The longest cycle the selected reactions close, or null. Of two as long,
 * the one drawn rounder now: a cycle already drawn as a ring keeps its ring.
 *
 * `reactions[i]` runs between `metabolites[i]` and `metabolites[i + 1]` (the
 * last back to the first), and `forward[i]` says whether `metabolites[i]` is on
 * its substrate side. Metabolites are `mergedId`s. Transporters are left out.
 *
 * @returns {{reactions: string[], metabolites: string[], forward: boolean[]}|null}
 */
export function selectionCycle(map, selection) {
  const ids = [...(selection?.reactions ?? [])].filter((id) => ringable(map, id)).sort(compare);
  if (ids.length < 3 || ids.length > MAX_REACTIONS) return null;

  // Each reaction joins every main substrate to every main product, both ways.
  const adjacency = new Map();
  const link = (from, to, reaction) => {
    if (!adjacency.has(from)) {
      adjacency.set(from, []);
    }
    adjacency.get(from).push({ to, reaction });
  };
  for (const id of ids) {
    const { substrates, products } = mainEnds(map, id);
    for (const a of substrates.keys()) {
      for (const b of products.keys()) {
        if (a === b) continue;
        link(a, b, id);
        link(b, a, id);
      }
    }
  }
  for (const list of adjacency.values()) {
    list.sort((p, q) => compare(p.to, q.to) || compare(p.reaction, q.reaction));
  }

  // Every simple cycle is met once from its smallest metabolite, so the search
  // from each start goes only through larger ones. It stops after a fixed
  // number of steps with the longest found so far.
  let budget = SEARCH_BUDGET;
  let best = null;
  const path = [];
  const reactions = [];
  const onPath = new Set();
  const used = new Set();
  const walk = (start, at) => {
    for (const { to, reaction } of adjacency.get(at) ?? []) {
      budget -= 1;
      if (budget < 0) return;
      if (used.has(reaction)) continue;
      if (to === start) {
        const length = reactions.length + 1;
        if (length >= 3 && (!best || length >= best.reactions.length)) {
          const found = { metabolites: [...path], reactions: [...reactions, reaction] };
          const round = roundness(map, found);
          if (!best || length > best.reactions.length || round < best.round - 1e-9) {
            best = { ...found, round };
          }
        }
        continue;
      }
      if (onPath.has(to) || compare(to, start) < 0) continue;
      path.push(to);
      reactions.push(reaction);
      onPath.add(to);
      used.add(reaction);
      walk(start, to);
      path.pop();
      reactions.pop();
      onPath.delete(to);
      used.delete(reaction);
    }
  };
  for (const start of [...adjacency.keys()].sort(compare)) {
    if (budget <= 0) break;
    path.push(start);
    onPath.add(start);
    walk(start, start);
    path.pop();
    onPath.delete(start);
  }
  if (!best) return null;

  const forward = [];
  for (let i = 0; i < best.reactions.length; i++) {
    forward.push(mainEnds(map, best.reactions[i]).substrates.has(best.metabolites[i]));
  }
  return { metabolites: best.metabolites, reactions: best.reactions, forward };
}

/**
 * How far a cycle is drawn from a circle now: the spread of its reactions'
 * and metabolites' distances from their middle, against the mean distance.
 * A metabolite drawn more than once counts where its first copy is.
 */
function roundness(map, cycle) {
  const points = [];
  for (let i = 0; i < cycle.reactions.length; i++) {
    const rxn = map.reactions.get(cycle.reactions[i]);
    points.push({ x: rxn.x, y: rxn.y });
    const node = map.nodes.get(nodesFor(map, cycle.reactions[i], cycle.metabolites[i])[0]);
    if (node) {
      points.push({ x: node.x, y: node.y });
    }
  }
  let cx = 0;
  let cy = 0;
  for (const p of points) {
    cx += p.x;
    cy += p.y;
  }
  cx /= points.length;
  cy /= points.length;
  let mean = 0;
  for (const p of points) {
    mean += Math.hypot(p.x - cx, p.y - cy);
  }
  mean /= points.length;
  if (mean < 1e-9) return 0;
  let variance = 0;
  for (const p of points) {
    const d = Math.hypot(p.x - cx, p.y - cy);
    variance += (d - mean) * (d - mean);
  }
  variance /= points.length;
  return Math.sqrt(variance) / mean;
}

/**
 * Lay the cycle the selected reactions close out as a circle, where it is
 * drawn now: about its middle (ringMiddle), running the way its metabolites
 * and reactions run round it, each as near its present place as it can be.
 * If that does not fit its compartment, the nearest place that does is used.
 *
 * @returns {{circled: true, reactions: number, radius: number, centre: {x: number, y: number}}
 *   |{circled: false, reason: string}|null}
 */
export function circularizeSelection(map, selection) {
  if (!map || !selection) return null;
  const cycle = selectionCycle(map, selection);
  if (!cycle) return { circled: false, reason: CIRCLE_REFUSAL.cycle };

  const k = cycle.reactions.length;
  const slots = 2 * k;
  const step = (2 * Math.PI) / slots;
  const radius = Math.max(RING_MIN_RADIUS, (slots * RING_STEP) / (2 * Math.PI));

  // One drawn node per metabolite of the cycle: the copy most drawn to among
  // those the two reactions beside it reach. The others are joined into it.
  const ringNodes = [];
  for (let i = 0; i < k; i++) {
    const key = cycle.metabolites[i];
    const before = cycle.reactions[(i + k - 1) % k];
    const after = cycle.reactions[i];
    const copies = [...new Set([...nodesFor(map, before, key), ...nodesFor(map, after, key)])];
    copies.sort((a, b) => segmentCount(map, b) - segmentCount(map, a) || compare(a, b));
    ringNodes.push(copies[0]);
  }

  // The circle in order: metabolite, reaction, metabolite, ... and where each is now.
  const items = [];
  for (let i = 0; i < k; i++) {
    items.push(map.nodes.get(ringNodes[i]));
    items.push(map.reactions.get(cycle.reactions[i]));
  }
  const middle = ringMiddle(items, radius);
  const angles = [];
  for (const item of items) {
    angles.push(Math.atan2(item.y - middle.y, item.x - middle.x));
  }
  let turning = 0;
  for (let j = 0; j < slots; j++) {
    turning += wrap(angles[(j + 1) % slots] - angles[j]);
  }
  const dir = turning < 0 ? -1 : 1;
  let sin = 0;
  let cos = 0;
  for (let j = 0; j < slots; j++) {
    sin += Math.sin(angles[j] - dir * j * step);
    cos += Math.cos(angles[j] - dir * j * step);
  }
  const offset = Math.abs(sin) + Math.abs(cos) > 1e-9 ? Math.atan2(sin, cos) : -Math.PI / 2;
  const slotAngle = (j) => offset + dir * j * step;

  // Where the circle goes: each metabolite where it may stand, each marker
  // where a small metabolite of its compartment could.
  const fits = (c) => {
    for (let j = 0; j < slots; j++) {
      const at = polar(c, radius, slotAngle(j));
      const node = map.nodes.get(ringNodes[Math.floor(j / 2)]);
      const stand = j % 2 === 0 ? node : { ...node, r: 10 };
      if (!nodeFitsAt(map, stand, at.x, at.y)) return false;
    }
    return true;
  };
  const centre = placeCircle(map, items[0], middle, radius, fits);
  if (!centre) return { circled: false, reason: CIRCLE_REFUSAL.room };

  // The other selected reactions. A second reaction between two neighbours of
  // the circle stands outside it, beside the first. One that reaches a single
  // metabolite of the circle stands out from it like a spoke. The rest stay
  // where they are.
  const ringIndex = new Map();
  const ringPairs = new Map();
  for (let i = 0; i < k; i++) {
    ringIndex.set(cycle.metabolites[i], i);
    ringPairs.set(pairKey(cycle.metabolites[i], cycle.metabolites[(i + 1) % k]), i);
  }
  const ringReactions = new Set(cycle.reactions);
  const parallels = [];
  const spokes = [];
  const others = [];
  for (const id of [...selection.reactions].sort(compare)) {
    if (ringReactions.has(id) || !map.reactions.has(id)) continue;
    if (!ringable(map, id)) {
      others.push(id);
      continue;
    }
    const ends = mainEnds(map, id);
    const i = parallelStep(ends, ringPairs);
    if (i >= 0) {
      parallels.push({ id, i, forward: ends.substrates.has(cycle.metabolites[i]) });
      continue;
    }
    const touched = [];
    for (const key of [...ends.substrates.keys(), ...ends.products.keys()]) {
      if (ringIndex.has(key) && !touched.includes(key)) {
        touched.push(key);
      }
    }
    if (touched.length === 1) {
      spokes.push({ id, i: ringIndex.get(touched[0]), forward: ends.substrates.has(touched[0]) });
    } else {
      others.push(id);
    }
  }

  // Reactions kept straight between two metabolites stop being so: they run along the circle now.
  for (const id of [...cycle.reactions, ...parallels.map((p) => p.id), ...spokes.map((s) => s.id)]) {
    if (map.reactions.get(id).linear) {
      setReactionLinear(map, id, false);
    }
  }

  // The copies joined into the circle's nodes: those its reactions, the
  // second reactions beside them and the spokes reach.
  for (let i = 0; i < k; i++) {
    const reach = [cycle.reactions[(i + k - 1) % k], cycle.reactions[i]];
    for (const p of parallels) {
      if (p.i === i || (p.i + 1) % k === i) {
        reach.push(p.id);
      }
    }
    for (const s of spokes) {
      if (s.i === i) {
        reach.push(s.id);
      }
    }
    for (const id of reach) {
      for (const copy of nodesFor(map, id, cycle.metabolites[i])) {
        if (copy !== ringNodes[i]) {
          map.mergeMetaboliteNodes(copy, ringNodes[i]);
        }
      }
    }
  }

  // The metabolites on the circle, named outside it.
  const ringSet = new Set(ringNodes);
  for (let i = 0; i < k; i++) {
    const node = map.nodes.get(ringNodes[i]);
    const angle = slotAngle(2 * i);
    setNode(map, node, polar(centre, radius, angle));
    node.hidden = false;
    node.labelAngle = metaboliteLabelAngle({ labelAngle: angle });
  }

  // Each reaction along the circle, from the metabolite on its substrate side
  // to the one on its product side, named inside the circle, or outside it
  // when it stands outside.
  const placeOnCircle = (id, slot, forward, out) => {
    const angle = slotAngle(slot);
    const at = polar(centre, radius + out, angle);
    moveReaction(map, id, at.x, at.y);
    const travel = angle + dir * Math.PI / 2;
    setReactionAngle(map, id, forward ? travel : travel + Math.PI);
    const name = out > 0 ? NAME_OUT : -NAME_IN;
    map.reactions.get(id).labelOffset = { x: Math.cos(angle) * name, y: Math.sin(angle) * name };
  };
  for (let i = 0; i < k; i++) {
    placeOnCircle(cycle.reactions[i], 2 * i + 1, cycle.forward[i], 0);
  }
  const stacked = new Map();
  for (const p of parallels) {
    const n = stacked.get(p.i) ?? 0;
    stacked.set(p.i, n + 1);
    placeOnCircle(p.id, 2 * p.i + 1, p.forward, PARALLEL_OUT * (n + 1));
  }

  // A cycle reaction's other main metabolites stand outside the circle, on
  // the side of the reaction they are on, each further one further out.
  const placed = new Set(ringSet);
  const extras = [];
  const outside = (id, slot, out, forward) => {
    const { substrates, products } = mainEnds(map, id);
    const sides = [[substrates, forward ? -1 : 1], [products, forward ? 1 : -1]];
    for (const [ends, toward] of sides) {
      let n = 0;
      const angle = slotAngle(slot + toward * 0.5);
      for (const copies of ends.values()) {
        for (const nodeId of copies) {
          if (placed.has(nodeId)) continue;
          placed.add(nodeId);
          const node = map.nodes.get(nodeId);
          const want = polar(centre, radius + out + EXTRA_OUT + EXTRA_STEP * n, angle);
          setNode(map, node, clampNodePosition(map, node, want.x, want.y));
          node.labelAngle = metaboliteLabelAngle({ labelAngle: angle });
          extras.push(nodeId);
          n += 1;
        }
      }
    }
  };
  for (let i = 0; i < k; i++) {
    outside(cycle.reactions[i], 2 * i + 1, 0, cycle.forward[i]);
  }
  stacked.clear();
  for (const p of parallels) {
    const n = stacked.get(p.i) ?? 0;
    stacked.set(p.i, n + 1);
    outside(p.id, 2 * p.i + 1, PARALLEL_OUT * (n + 1), p.forward);
  }

  // A spoke runs straight out from its metabolite, its other main metabolites
  // further along it, and a second spoke on one metabolite beside the first.
  // A metabolite with a spoke straight out writes its name inside the circle.
  stacked.clear();
  for (const s of spokes) {
    const n = stacked.get(s.i) ?? 0;
    stacked.set(s.i, n + 1);
    let spread = n / 2;
    if (n % 2 === 1) {
      spread = -(n + 1) / 2;
    }
    const angle = slotAngle(2 * s.i) + dir * step * 0.5 * spread;
    const at = polar(centre, radius + SPOKE, angle);
    moveReaction(map, s.id, at.x, at.y);
    setReactionAngle(map, s.id, s.forward ? angle : angle + Math.PI);
    map.reactions.get(s.id).labelOffset = { x: -Math.sin(angle) * NAME_OUT, y: Math.cos(angle) * NAME_OUT };
    const { substrates, products } = mainEnds(map, s.id);
    let j = 0;
    for (const copies of [...substrates.values(), ...products.values()]) {
      for (const nodeId of copies) {
        if (placed.has(nodeId)) continue;
        placed.add(nodeId);
        const node = map.nodes.get(nodeId);
        const want = polar(centre, radius + 2 * SPOKE + EXTRA_STEP * j, angle);
        setNode(map, node, clampNodePosition(map, node, want.x, want.y));
        node.labelAngle = metaboliteLabelAngle({ labelAngle: angle });
        extras.push(nodeId);
        j += 1;
      }
    }
    if (n === 0) {
      const inward = slotAngle(2 * s.i) + Math.PI;
      map.nodes.get(ringNodes[s.i]).labelAngle = metaboliteLabelAngle({ labelAngle: inward });
    }
  }

  // Every selected reaction's curves tidied, the cycle's own along the circle.
  const tidied = [...cycle.reactions, ...parallels.map((p) => p.id), ...spokes.map((s) => s.id), ...others];
  for (const id of tidied) {
    tidyReaction(map, id);
  }
  for (const seg of map.segments) {
    const branch = seg.side === 'substrate' || seg.side === 'product';
    if (branch && ringReactions.has(seg.reactionId) && ringSet.has(seg.nodeId)) {
      bendAlong(seg, centre, radius);
    }
  }
  // Straight reactions drawn to anything that moved follow it.
  straightenLinearReactions(map, [...ringNodes, ...extras]);

  return { circled: true, reactions: k, radius, centre };
}

/**
 * The middle of a cycle as drawn now. A cycle already drawn round (whole, or
 * as an arc) keeps the middle of its circle, the least-squares one through
 * its points, so what is written in it stays in the middle. A drawing that is
 * not round, whose circle would be far off or too large, uses its centroid.
 */
function ringMiddle(points, radius) {
  const n = points.length;
  let cx = 0;
  let cy = 0;
  for (const p of points) {
    cx += p.x;
    cy += p.y;
  }
  cx /= n;
  cy /= n;
  let suu = 0;
  let svv = 0;
  let suv = 0;
  let suz = 0;
  let svz = 0;
  let sz = 0;
  let spread = 0;
  for (const p of points) {
    const u = p.x - cx;
    const v = p.y - cy;
    const z = u * u + v * v;
    suu += u * u;
    svv += v * v;
    suv += u * v;
    suz += u * z;
    svz += v * z;
    sz += z;
    spread += Math.sqrt(z);
  }
  spread /= n;
  const det = suu * svv - suv * suv;
  if (!(det > 1e-9 * (suu + svv) * (suu + svv))) return { x: cx, y: cy };
  const a = (suz * svv - svz * suv) / (2 * det);
  const b = (svz * suu - suz * suv) / (2 * det);
  const fitted = Math.sqrt(a * a + b * b + sz / n);
  if (Math.hypot(a, b) > spread || fitted > 2 * radius) return { x: cx, y: cy };
  return { x: cx + a, y: cy + b };
}

/** A reaction that can stand on the circle: drawn, with a backbone, and not a transporter. */
function ringable(map, id) {
  const rxn = map?.reactions?.get(id);
  return Boolean(rxn?.axis) && !rxn.onMembrane;
}

/**
 * The main metabolites drawn on each side of a reaction, as `mergedId` →
 * node ids, in the order its branches are drawn. Cofactors are left out.
 */
function mainEnds(map, id) {
  const substrates = new Map();
  const products = new Map();
  for (const seg of map.segments) {
    const branch = seg.side === 'substrate' || seg.side === 'product';
    if (seg.reactionId !== id || !seg.nodeId || !branch) continue;
    const node = map.nodes.get(seg.nodeId);
    if (!node || isSecondaryNode(node)) continue;
    const ends = seg.side === 'substrate' ? substrates : products;
    const key = node.mergedId ?? node.id;
    if (!ends.has(key)) {
      ends.set(key, []);
    }
    if (!ends.get(key).includes(node.id)) {
      ends.get(key).push(node.id);
    }
  }
  return { substrates, products };
}

/** The drawn copies of metabolite `key` a reaction reaches, substrates first. */
function nodesFor(map, reactionId, key) {
  const { substrates, products } = mainEnds(map, reactionId);
  return [...(substrates.get(key) ?? []), ...(products.get(key) ?? [])];
}

/** How many curves are drawn to a node. */
function segmentCount(map, nodeId) {
  let n = 0;
  for (const seg of map.segments) {
    if (seg.nodeId === nodeId) {
      n += 1;
    }
  }
  return n;
}

/** One key for two metabolites, whichever way round they are given. */
function pairKey(a, b) {
  if (compare(a, b) < 0) return `${a}\u0000${b}`;
  return `${b}\u0000${a}`;
}

/** The step of the circle a reaction runs alongside (both its ends are that step's metabolites), or -1. */
function parallelStep(ends, ringPairs) {
  for (const a of ends.substrates.keys()) {
    for (const b of ends.products.keys()) {
      const i = ringPairs.get(pairKey(a, b));
      if (i !== undefined) return i;
    }
  }
  return -1;
}

/**
 * Where the circle goes: about `middle` if everything on it fits there,
 * else the nearest point of a grid over its compartment where it does, the
 * circle kept inside the compartment. Null when there is none.
 */
function placeCircle(map, node, middle, radius, fits) {
  if (fits(middle)) return middle;
  let rect = null;
  if (node.kind === 'medium') {
    rect = map.environment?.rect ?? null;
  } else {
    const cell = map.cells?.find((c) => c.id === node.cellId);
    rect = cell?.compartments.find((c) => c.id === node.compartment)?.rect ?? null;
  }
  if (!rect) return null;
  const reach = radius + ROOM_PAD;
  const candidates = [];
  for (let y = rect.y + reach; y <= rect.y + rect.h - reach + 1e-9; y += ROOM_GRID) {
    for (let x = rect.x + reach; x <= rect.x + rect.w - reach + 1e-9; x += ROOM_GRID) {
      candidates.push({ x, y, d: Math.hypot(x - middle.x, y - middle.y) });
    }
  }
  candidates.sort((a, b) => a.d - b.d || a.y - b.y || a.x - b.x);
  for (const c of candidates) {
    if (fits(c)) return { x: c.x, y: c.y };
  }
  return null;
}

/** Give a branch to a metabolite on the circle the shape of the circle's arc between its two ends. */
function bendAlong(seg, centre, radius) {
  const node = seg.side === 'substrate' ? seg.from : seg.to;
  const anchor = seg.side === 'substrate' ? seg.to : seg.from;
  const a = Math.atan2(node.y - centre.y, node.x - centre.x);
  const b = Math.atan2(anchor.y - centre.y, anchor.x - centre.x);
  const sweep = wrap(b - a);
  // The handle length that makes a cubic Bézier follow a circular arc.
  const k = (4 / 3) * Math.tan(sweep / 4) * radius;
  seg.b1 = { x: node.x - Math.sin(a) * k, y: node.y + Math.cos(a) * k };
  seg.b2 = { x: anchor.x + Math.sin(b) * k, y: anchor.y - Math.cos(b) * k };
  delete seg.control;
}

/** Put a node at a point and bring the curves drawn to it along. */
function setNode(map, node, point) {
  const delta = { x: point.x - node.x, y: point.y - node.y };
  node.x = point.x;
  node.y = point.y;
  updateConnectedSegments(map, node.id, delta);
}

/** The point `r` from `c` in direction `angle`. */
function polar(c, r, angle) {
  return { x: c.x + r * Math.cos(angle), y: c.y + r * Math.sin(angle) };
}

/** An angle in (-π, π]. */
function wrap(angle) {
  let a = angle % (2 * Math.PI);
  if (a > Math.PI) {
    a -= 2 * Math.PI;
  }
  if (a <= -Math.PI) {
    a += 2 * Math.PI;
  }
  return a;
}

/** Ids in a fixed order, the same in the desktop app: by UTF-16 code unit. */
function compare(a, b) {
  if (a < b) return -1;
  if (a > b) return 1;
  return 0;
}
