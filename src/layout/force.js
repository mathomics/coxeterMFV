/**
 * Small deterministic force relaxation for placing nodes inside a compartment.
 *
 * Three forces, in decreasing priority:
 *   containment  keep every node inside its compartment and out of the organelles
 *                nested within it
 *   springs      pull a reaction marker towards the metabolites it touches
 *   repulsion    push overlapping nodes apart
 *
 * Deterministic: the seeded PRNG only breaks exact ties, so the same model
 * always produces the same map.
 */

import { signedDistance, closestPerimeterPoint, clampToRect } from './geometry.js';

/** Mulberry32: tiny, seedable, good enough for jitter. */
export function makeRandom(seed = 1) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Relax `spec.items` in place and return them. Each item is a circle in map
 * pixels, held inside `bounds` and out of `holes`. `fixed` items do not move.
 * Edges are springs between item indices with a rest `length` in pixels.
 *
 * @param {object} spec
 * @param {Array<{x:number,y:number,r:number,fixed?:boolean,bounds:object,holes?:object[],anchorX?:number,anchorY?:number}>} spec.items
 * @param {Array<{a:number,b:number,length:number,strength?:number}>} spec.edges
 * @param {number} [spec.iterations] default 260
 * @param {number} [spec.repulsion] multiplier on the overlap push, default 1
 * @param {number} [spec.seed] default 12345
 * @param {number} [spec.rankStrength] pull towards anchorX/anchorY, default 0.10, 0 turns it off
 */
export function relax(spec) {
  const {
    items,
    edges = [],
    iterations = 260,
    repulsion = 1.0,
    seed = 12345,
    rankStrength = 0.10,
  } = spec;

  const n = items.length;
  if (!n) return items;
  const rand = makeRandom(seed);
  const vx = new Float64Array(n);
  const vy = new Float64Array(n);

  // Break exact coincidences so repulsion has a direction to work with.
  for (let i = 0; i < n; i++) {
    items[i].x += (rand() - 0.5) * 0.5;
    items[i].y += (rand() - 0.5) * 0.5;
  }

  for (let iter = 0; iter < iterations; iter++) {
    const cooling = 1 - iter / iterations;
    const step = 0.55 * cooling + 0.08;
    vx.fill(0);
    vy.fill(0);

    // Springs
    for (const e of edges) {
      const a = items[e.a];
      const b = items[e.b];
      if (!a || !b) continue;
      let dx = b.x - a.x;
      let dy = b.y - a.y;
      let d = Math.hypot(dx, dy);
      if (d < 1e-6) { dx = 0.7; dy = 0.3; d = 0.76; }
      const k = (e.strength ?? 1) * 0.13;
      const f = ((d - e.length) / d) * k;
      vx[e.a] += dx * f; vy[e.a] += dy * f;
      vx[e.b] -= dx * f; vy[e.b] -= dy * f;
    }

    // Repulsion, O(n^2): compartments hold tens of nodes, not thousands
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        const a = items[i];
        const b = items[j];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let d2 = dx * dx + dy * dy;
        const want = (a.r + b.r) * 3.2 + 14;
        if (d2 > want * want) continue;
        let d = Math.sqrt(d2);
        if (d < 1e-6) { dx = rand() - 0.5; dy = rand() - 0.5; d = 0.5; }
        const push = ((want - d) / d) * 0.85 * repulsion;
        vx[i] -= dx * push; vy[i] -= dy * push;
        vx[j] += dx * push; vy[j] += dy * push;
      }
    }

    // Rank anchoring, which keeps pathways flowing top to bottom
    if (rankStrength > 0) {
      for (let i = 0; i < n; i++) {
        const it = items[i];
        if (it.anchorY === undefined) continue;
        vy[i] += (it.anchorY - it.y) * rankStrength * cooling;
        if (it.anchorX !== undefined) vx[i] += (it.anchorX - it.x) * rankStrength * 0.6 * cooling;
      }
    }

    // Integration and containment
    for (let i = 0; i < n; i++) {
      const it = items[i];
      if (it.fixed) continue;
      it.x += Math.max(-24, Math.min(24, vx[i])) * step;
      it.y += Math.max(-24, Math.min(24, vy[i])) * step;

      if (it.bounds) {
        const p = clampToRect(it, it.bounds, it.r + 6);
        it.x = p.x; it.y = p.y;
      }
      // Push out of nested organelles so the parent's nodes never sit on top of
      // a child compartment. This is a soft, integrated force rather than a
      // teleport: snapping every intruder to the nearest edge piles them all up
      // on the same stretch of membrane.
      if (it.holes) {
        for (const hole of it.holes) {
          const clearance = it.r + 12;
          const sd = signedDistance(hole, it);
          if (sd >= clearance) continue;
          const edge = closestPerimeterPoint(hole, it);
          let ox = edge.nx;
          let oy = edge.ny;
          if (!ox && !oy) { ox = it.x < edge.x ? -1 : 1; oy = 0; }
          const depth = clearance - sd;
          const gain = sd < 0 ? 0.55 : 0.22;
          it.x += ox * depth * gain;
          it.y += oy * depth * gain;
        }
      }
    }
  }

  // Final pass: anything still overlapping an organelle gets placed just
  // outside it, spread around the edge it is closest to.
  for (const it of items) {
    if (it.fixed || !it.holes) continue;
    for (const hole of it.holes) {
      const clearance = it.r + 10;
      if (signedDistance(hole, it) >= clearance) continue;
      const edge = closestPerimeterPoint(hole, it);
      const ox = edge.nx || (it.x < edge.x ? -1 : 1);
      const oy = edge.ny || (it.y < edge.y ? -1 : 1);
      it.x = edge.x + ox * clearance;
      it.y = edge.y + oy * clearance;
      if (it.bounds) {
        const p = clampToRect(it, it.bounds, it.r + 6);
        it.x = p.x; it.y = p.y;
      }
    }
  }
  return items;
}
