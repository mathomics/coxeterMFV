#!/usr/bin/env node
/**
 * Merge partial Figure 4 sweeps into one file.
 *
 * The sweep is long enough that it has to run in parallel chunks, each writing
 * its own JSON. This joins them, sorts by the fiber uptake cap, and reports which
 * of the thirty points are still missing rather than quietly presenting a partial
 * sweep as a whole one.
 *
 *   node tools/merge-figure4.mjs out.json part-*.json
 */
import { readFile, writeFile } from 'node:fs/promises';
import { FIBER_SWEEP } from '../src/communities/gut9-figure4.js';

const [out, ...parts] = process.argv.slice(2);
if (!out || !parts.length) {
  console.error('usage: node tools/merge-figure4.mjs out.json part-*.json');
  process.exit(2);
}

/**
 * Normalise a point's abundances to sum to one.
 *
 * The capped formulation maximises the total biomass, so a point solved just
 * below the maximum growth rate can come back with slightly more than the one
 * gram asked for, and abundances that sum to that. Rescaling is exact, not
 * cosmetic: every constraint is homogeneous in (V, X) bar the community exchange
 * bounds, and those are intervals containing zero, so shrinking a feasible
 * solution leaves it feasible and the relative abundances unchanged. The solver
 * normalises as well, so for its own output this is a no-op. It matters for
 * recorded points that were not normalised, which it keeps consistent with the
 * rest.
 */
function normalise(row) {
  const total = Object.values(row.abundances ?? {}).reduce((sum, x) => sum + x, 0);
  if (!(total > 0) || Math.abs(total - 1) < 1e-12) return row;
  const abundances = Object.fromEntries(
    Object.entries(row.abundances).map(([id, x]) => [id, x / total]),
  );
  return { ...row, abundances, abundanceScale: 1 / total };
}

const byPoint = new Map();
for (const part of parts) {
  let data;
  try {
    data = JSON.parse(await readFile(part, 'utf8'));
  } catch (error) {
    console.error(`skipping ${part}: ${error.message}`);
    continue;
  }
  for (const row of data.results ?? []) {
    if (row.status) continue;                    // a point that did not solve
    byPoint.set(row.bacteroides, normalise(row));
  }
}

const results = [...byPoint.values()].sort((a, b) => a.bacteroides - b.bacteroides);
const missing = FIBER_SWEEP.bacteroides.filter((x) => !byPoint.has(x));

// A point that was attempted and ran out of budget is not the same as one that
// was never attempted, and the file says which is which.
const attempted = new Map();
for (const part of parts) {
  try {
    const data = JSON.parse(await readFile(part, 'utf8'));
    for (const entry of data.notComputed ?? []) {
      if (byPoint.has(entry.point)) continue;              // it solved somewhere else
      // A point may have been attempted at several budgets. The largest it ran
      // past is the informative one: it says how much time is known not to be
      // enough, which a smaller budget does not.
      const seen = attempted.get(entry.point);
      if (!seen || entry.budget > seen.budget) attempted.set(entry.point, entry);
    }
  } catch { /* already reported above */ }
}
const notComputed = [...attempted.values()];

await writeFile(out, `${JSON.stringify({
  source: 'simFigure4.m on model9.xlsx + model9_rxns_params.txt',
  sweep: FIBER_SWEEP,
  computedPoints: results.map((r) => r.bacteroides),
  missingPoints: missing,
  notComputed: notComputed.sort((a, b) => a.point - b.point),
  results,
}, null, 2)}\n`);

console.log(`${out}: ${results.length} of ${FIBER_SWEEP.bacteroides.length} points`);
if (missing.length) console.log(`missing: ${missing.join(', ')}`);
